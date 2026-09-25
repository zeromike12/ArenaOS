//! Milestone 2 self-test suite — kernel foundations (ROADMAP.md §M2).
//!
//! Same rules as M1 (ADR-0005): every test asserts an *observable machine
//! effect* — register read-backs, recorded fault facts — never "we printed
//! success". Marker grammar per docs/TESTING.md:
//!
//! ```text
//! m2:test:<name>: PASS
//! m2:test:<name>: FAIL (<reason>)
//! m2: RESULT PASS (p/t)   |   m2: RESULT FAIL (p/t)
//! ```
//!
//! M2.1 (this file today): the exception architecture — TSS/IST installed
//! and read back, and *real* executed faults (divide-by-zero #DE, write to
//! unmapped memory #PF) delivered through our IDT, diagnosed (including CR2
//! and error-code semantics), and recovered via the controlled fault
//! injection protocol (`arch/x86_64/faults.rs`). Later M2 steps append
//! their tests here.

use crate::arch::x86_64::{faults, gdt, tss};
use crate::log::{self, log_error as error, log_info as info};

type TestFn = fn() -> Result<(), &'static str>;

/// (marker name, test)
const TESTS: &[(&str, TestFn)] = &[
    ("tss_installed", test_tss_installed),
    ("exc_de_recovered", test_exc_de_recovered),
    ("exc_pf_recovered", test_exc_pf_recovered),
];

/// Fault sites: minimal assembly that (1) records its own resume address
/// into `faults::RESUME` and (2) executes a guaranteed-faulting instruction.
/// The recovery path lands on the label right after the faulting instruction.
mod fault_sites {
    /// #DE: `div ecx` with ECX = 0.
    ///
    /// # Safety
    /// Call only with `faults::arm(0)` in effect; otherwise the divide fault
    /// takes the production path (diagnostics + halt) — which is correct
    /// behavior, just fatal to the test run.
    pub unsafe fn divide_by_zero() {
        // SAFETY: the faulting instruction is the point of the call; all
        // clobbered registers are declared; the resume slot write is to our
        // own static (single-CPU, IF=0 boot context).
        unsafe {
            core::arch::asm!(
                "lea rax, [rip + 2f]",
                "mov [rip + {resume}], rax",
                "xor edx, edx",
                "mov eax, 1",
                "xor ecx, ecx",
                "div ecx", // #DE — handler resumes execution at 2:
                "2:",
                resume = sym crate::arch::x86_64::faults::RESUME,
                out("rax") _, out("rcx") _, out("rdx") _,
                options(nostack),
            );
        }
    }

    /// #PF: 8-byte write to `addr` (caller guarantees it is unmapped).
    ///
    /// # Safety
    /// Call only with `faults::arm(14)` in effect; see `divide_by_zero`.
    pub unsafe fn write_unmapped(addr: u64) {
        // SAFETY: as `divide_by_zero`; `addr` validity is the caller's
        // contract (unmapped → guaranteed #PF, never silent success).
        unsafe {
            core::arch::asm!(
                "lea rax, [rip + 2f]",
                "mov [rip + {resume}], rax",
                "mov [{addr}], rax", // #PF — handler resumes execution at 2:
                "2:",
                addr = in(reg) addr,
                resume = sym crate::arch::x86_64::faults::RESUME,
                out("rax") _,
                options(nostack),
            );
        }
    }
}

/// Canonical address guaranteed unmapped for the #PF test: far above any
/// RAM or MMIO the firmware maps on a 512 MiB VM (RAM < 4 GiB, MMIO windows
/// < 64 GiB, flash < 4 GiB), far below the kernel higher-half base that M2.4
/// will introduce. Invariant: this address must stay unmapped in every
/// address space we build (documented in docs/TESTING.md).
const UNMAPPED_CANONICAL_ADDR: u64 = 0x0000_6000_0000_0000;

/// The TSS descriptor must be live in the task register, and IST1 must hold
/// the dedicated fault-stack top (#DF/NMI/#MC switch to it — M2.1).
fn test_tss_installed() -> Result<(), &'static str> {
    let tr = tss::read_tr();
    let ist1 = tss::read_back_ist1();
    let want = tss::ist1_stack_top();
    info!(
        "m2",
        "tss: str read-back sel={tr:#x}; IST1 top={ist1:#x} (want {want:#x})"
    );
    if tr != gdt::TSS_SELECTOR {
        return Err("task register does not hold our TSS selector");
    }
    if ist1 != want || want == 0 {
        return Err("TSS IST1 does not point at the fault-stack top");
    }
    Ok(())
}

/// A real divide-by-zero must be delivered as vector 0 through our IDT,
/// recorded by the handler, and recovered from — execution continues in
/// this function, which is itself the proof of the `iretq` path.
fn test_exc_de_recovered() -> Result<(), &'static str> {
    faults::arm(0);
    // SAFETY: armed above — the handler recovers this fault by contract.
    unsafe { fault_sites::divide_by_zero() };
    let obs = faults::observed();
    faults::disarm();
    info!(
        "m2",
        "exc_de: observed vector={:#x} error_code={:#x}; resumed after faulting div",
        obs.vector,
        obs.error_code
    );
    if !obs.valid || obs.vector != 0 {
        return Err("#DE was not delivered/recorded through our exception path");
    }
    Ok(())
}

/// A real write to unmapped memory must be delivered as vector 14, with CR2
/// naming the faulting address and the error code marking
/// not-present + write (bits P=0, W=1 → (ec & 0b11) == 0b10).
fn test_exc_pf_recovered() -> Result<(), &'static str> {
    faults::arm(14);
    // SAFETY: armed above; UNMAPPED_CANONICAL_ADDR is unmapped by invariant.
    unsafe { fault_sites::write_unmapped(UNMAPPED_CANONICAL_ADDR) };
    let obs = faults::observed();
    faults::disarm();
    info!(
        "m2",
        "exc_pf: observed vector={:#x} error_code={:#x} cr2={:#x}; resumed after faulting write",
        obs.vector,
        obs.error_code,
        obs.cr2
    );
    if !obs.valid || obs.vector != 14 {
        return Err("#PF was not delivered/recorded through our exception path");
    }
    if obs.cr2 != UNMAPPED_CANONICAL_ADDR {
        return Err("CR2 did not report the faulting linear address");
    }
    if obs.error_code & 0b11 != 0b10 {
        return Err("error code is not not-present+write for an unmapped write");
    }
    Ok(())
}

/// Run all M2 tests so far, emit markers, return (passed, total).
pub fn run_all() -> (usize, usize) {
    info!("m2", "running milestone-2 self-tests");
    let mut passed = 0usize;
    for (name, test) in TESTS {
        match test() {
            Ok(()) => {
                passed += 1;
                log::write_marker(format_args!("m2:test:{name}: PASS"));
            }
            Err(reason) => {
                error!("m2", "test {name} failed: {reason}");
                log::write_marker(format_args!("m2:test:{name}: FAIL ({reason})"));
            }
        }
    }
    let total = TESTS.len();
    if passed == total {
        log::write_marker(format_args!("m2: RESULT PASS ({passed}/{total})"));
    } else {
        log::write_marker(format_args!("m2: RESULT FAIL ({passed}/{total})"));
    }
    (passed, total)
}
