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

use crate::arch::x86_64::{faults, gdt, idt, tss};
use crate::drivers::pit;
use crate::log::{self, log_error as error, log_info as info};
use crate::timekeeping;

type TestFn = fn() -> Result<(), &'static str>;

/// (marker name, test)
const TESTS: &[(&str, TestFn)] = &[
    ("tss_installed", test_tss_installed),
    ("exc_de_recovered", test_exc_de_recovered),
    ("exc_pf_recovered", test_exc_pf_recovered),
    ("pit_oneshot", test_pit_oneshot),
    ("tsc_frequency", test_tsc_frequency),
    ("clock_monotonic", test_clock_monotonic),
    ("tick_rate", test_tick_rate),
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

// ---------------------------------------------------------------------------
// M2.2 — timers: PIT modes, TSC calibration, monotonic clock, tick delivery
// ---------------------------------------------------------------------------

/// One-shot PIT programming must be observable: program ~10 ms of
/// oscillator counts, poll the read-back status until OUT latches high, and
/// check the wait took approximately the programmed time on the calibrated
/// clock (bounds generous for TCG scheduling noise, tight enough that a
/// dead or wildly misprogrammed counter fails).
fn test_pit_oneshot() -> Result<(), &'static str> {
    const ONESHOT_COUNTS: u16 = 11_932; // ~10 ms at 1 193 182 Hz
    let t0 = timekeeping::now_us();
    // SAFETY: M2 owns the PIT after timekeeping::init(); IF=0 here (boot
    // invariant), and the periodic tick is re-armed before returning.
    unsafe { pit::set_oneshot(ONESHOT_COUNTS) };
    let mut status;
    let mut spin = 0u64;
    loop {
        // SAFETY: as above; read-back is side-effect-free beyond latching.
        status = unsafe { pit::read_ch0_status() };
        if status & pit::STATUS_OUT != 0 || spin >= 400_000_000 {
            break;
        }
        spin += 1;
        core::hint::spin_loop();
    }
    let elapsed_us = timekeeping::now_us() - t0;
    // SAFETY: leave the machine ticking for every later consumer.
    unsafe { pit::set_periodic_hz(timekeeping::KERNEL_TICK_HZ) };

    info!(
        "m2",
        "pit_oneshot: status={status:#x} (OUT={}) after {elapsed_us}us (spin={spin})",
        status & pit::STATUS_OUT != 0
    );
    if status & pit::STATUS_OUT == 0 {
        return Err("one-shot OUT never asserted (counter not running?)");
    }
    if !(5_000..=200_000).contains(&elapsed_us) {
        return Err("one-shot duration outside plausible bounds for ~10 ms");
    }
    Ok(())
}

/// Independently re-measure the TSC frequency (a fresh PIT calibration
/// window) and cross-check it against the value `timekeeping::init()`
/// computed at boot — two measurements of the same fact must agree.
fn test_tsc_frequency() -> Result<(), &'static str> {
    // SAFETY: PIT owned post-init; IF=0; periodic tick restored below.
    unsafe { pit::set_calibration_mode() };
    let (counts, tsc) = unsafe { pit::calibrate_tsc_window(pit::OSCILLATOR_HZ as u32 / 50) };
    unsafe { pit::set_periodic_hz(timekeeping::KERNEL_TICK_HZ) };

    if counts == 0 || tsc == 0 {
        return Err("calibration window measured zero");
    }
    let hz = tsc * pit::OSCILLATOR_HZ / u64::from(counts);
    let boot_hz = timekeeping::tsc_hz();
    let (lo, hi) = if hz <= boot_hz {
        (hz, boot_hz)
    } else {
        (boot_hz, hz)
    };
    info!(
        "m2",
        "tsc_frequency: re-measured {hz} Hz ({} MHz) vs boot {} MHz over {counts} PIT counts",
        hz / 1_000_000,
        boot_hz / 1_000_000
    );
    if boot_hz == 0 {
        return Err("timekeeping::init() did not produce a calibration");
    }
    if hi - lo > lo * 5 / 100 {
        return Err("re-measured TSC frequency disagrees with boot calibration (>5%)");
    }
    Ok(())
}

/// The monotonic clock must actually be monotonic, and its microsecond
/// scale must be real: a clock-measured busy-wait must take at least the
/// requested time, and the same interval cross-checked against a raw
/// PIT-oscillator window must agree.
fn test_clock_monotonic() -> Result<(), &'static str> {
    let a = timekeeping::now_us();
    let b = timekeeping::now_us();
    if b < a {
        return Err("clock went backwards between adjacent reads");
    }

    const WAIT_US: u64 = 2_000;
    if !timekeeping::busy_wait_us(WAIT_US) {
        return Err("busy-wait hit its anti-hang guard (clock not advancing?)");
    }
    let c = timekeeping::now_us();
    let waited = c - b;
    if !(WAIT_US..=WAIT_US * 4).contains(&waited) {
        return Err("busy-wait duration outside [1x, 4x] of request");
    }

    // Cross-check the scale against the oscillator directly: run a ~2 ms
    // PIT-count window and compare PIT-derived µs with clock-derived µs.
    // SAFETY: PIT owned post-init; IF=0; periodic tick restored below.
    unsafe { pit::set_calibration_mode() };
    let (counts, tsc) = unsafe { pit::calibrate_tsc_window(pit::OSCILLATOR_HZ as u32 / 500) };
    unsafe { pit::set_periodic_hz(timekeeping::KERNEL_TICK_HZ) };
    if counts == 0 {
        return Err("cross-check window measured zero counts");
    }
    let pit_us = u64::from(counts) * 1_000_000 / pit::OSCILLATOR_HZ;
    let tsc_us = timekeeping::ticks_to_us(tsc);
    info!(
        "m2",
        "clock_monotonic: waited {waited}us for {WAIT_US}us request; cross-check window pit={pit_us}us clock={tsc_us}us"
    );
    let (lo, hi) = if pit_us <= tsc_us {
        (pit_us, tsc_us)
    } else {
        (tsc_us, pit_us)
    };
    if hi - lo > hi / 10 + 1 {
        return Err("clock microseconds disagree with PIT-oscillator microseconds (>10%)");
    }
    Ok(())
}

/// Timer *interrupts* must flow at the programmed rate through the whole
/// chain: PIT mode 3 → IOAPIC pin 0 → LAPIC vector 32 → our IDT → absorb
/// stub (EOI both controllers) → counter. Ten ticks at 100 Hz ≈ 100 ms;
/// measured with the calibrated clock, bounds generous for TCG.
fn test_tick_rate() -> Result<(), &'static str> {
    const REQUIRED_TICKS: u64 = 10;
    const WINDOW_CAP_US: u64 = 2_000_000; // 2 s anti-hang cap
    let before = idt::absorbed_irq_count();
    let t0 = timekeeping::now_us();
    // SAFETY: IF window is bounded by the cap below and re-masked on every
    // exit path; single CPU; absorb stub EOIs both controllers (M1 proof).
    crate::arch::x86_64::sti();
    let mut ticks;
    loop {
        ticks = idt::absorbed_irq_count() - before;
        if ticks >= REQUIRED_TICKS || timekeeping::now_us() - t0 > WINDOW_CAP_US {
            break;
        }
        core::hint::spin_loop();
    }
    let t1 = timekeeping::now_us();
    crate::arch::x86_64::cli();

    let window_us = t1 - t0;
    let avg_us = window_us.checked_div(ticks).unwrap_or(u64::MAX);
    let expect_us = 1_000_000 / u64::from(timekeeping::KERNEL_TICK_HZ);
    info!(
        "m2",
        "tick_rate: {ticks} ticks in {window_us}us (avg {avg_us}us, expected ~{expect_us}us at {} Hz)",
        timekeeping::KERNEL_TICK_HZ
    );
    if ticks < REQUIRED_TICKS {
        return Err("timer interrupts stopped flowing at the programmed rate");
    }
    // TCG stretches guest-visible latency unpredictably; require the average
    // period to be within [0.5x, 4x] programmed — a wrong divisor, dead
    // IOAPIC route, or missed EOIs (which stall delivery entirely) all fail.
    if avg_us < expect_us / 2 || avg_us > expect_us * 4 {
        return Err("average tick period outside plausible bounds");
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
