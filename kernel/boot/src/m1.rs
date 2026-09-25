//! Milestone 1 self-test suite (ADR-0005).
//!
//! Every test asserts an *observable machine effect* — bytes that came back
//! out of the UART, register values read back after a GDT load, structures
//! parsed from real firmware memory — never "we printed success". Marker
//! grammar is fixed by docs/TESTING.md:
//!
//! ```text
//! m1:test:<name>: PASS
//! m1:test:<name>: FAIL (<reason>)
//! m1: RESULT PASS (p/t)   |   m1: RESULT FAIL (p/t)
//! ```

use crate::arch::x86_64::{self, cr0, cr4, efer, gdt};
use crate::bootinfo;
use crate::drivers::serial;
use crate::log::{self, log_error as error, log_info as info};

type TestFn = fn() -> Result<(), &'static str>;

/// (marker name, description, test)
const TESTS: &[(&str, TestFn)] = &[
    ("serial_loopback", test_serial_loopback),
    ("cpu_long_mode", test_cpu_long_mode),
    ("cpuid_sane", test_cpuid_sane),
    ("gdt_installed", test_gdt_installed),
    ("idt_installed", test_idt_installed),
    ("memory_map", test_memory_map),
    ("wide_math", test_wide_math),
    ("timer_irq_absorbed", test_timer_irq_absorbed),
];

/// 16550 scratch + loopback round-trips (see driver docs for why this proves
/// the port-I/O path and the device, not our own memory).
fn test_serial_loopback() -> Result<(), &'static str> {
    // SAFETY: boot stage owns COM1 exclusively at this point (single CPU,
    // interrupts off, logging is the only other user and is paused inside
    // this synchronous call).
    unsafe { serial::loopback_selftest() }
}

/// Verify we are really in 64-bit long mode with paging, by reading the
/// architectural state back out of the CPU (SDM Vol. 3 §3.4.5, §2.6, and
/// Vol. 4 §2.2.1 for EFER).
fn test_cpu_long_mode() -> Result<(), &'static str> {
    let (cr0v, cr3v, cr4v, eferv) = (
        x86_64::read_cr0(),
        x86_64::read_cr3(),
        x86_64::read_cr4(),
        x86_64::read_efer(),
    );
    info!(
        "m1",
        "cpustate: cr0={cr0v:#x} cr3={cr3v:#x} cr4={cr4v:#x} efer={eferv:#x}"
    );

    if cr0v & cr0::PG == 0 || cr0v & cr0::PE == 0 {
        return Err("CR0: paging or protection not enabled");
    }
    if cr4v & cr4::PAE == 0 {
        return Err("CR4.PAE not set (required in long mode)");
    }
    if eferv & efer::LME == 0 || eferv & efer::LMA == 0 {
        return Err("EFER: long mode not enabled/active");
    }
    Ok(())
}

/// CPUID must return a plausible vendor and features — proves the intrinsic
/// path works and logs what we are running on.
fn test_cpuid_sane() -> Result<(), &'static str> {
    let cpu = x86_64::cpu_info();
    info!("m1", "cpu: {cpu}");
    if cpu.vendor.iter().all(|b| !(0x20..0x7f).contains(b)) {
        return Err("CPUID vendor string is empty/non-printable");
    }
    if !cpu.has_long_mode {
        return Err("CPUID reports no long-mode support (yet we are in long mode?)");
    }
    Ok(())
}

/// After our `lgdt` + far-jump, `sgdt` must report *our* table's base and
/// limit — proving the CPU accepted it (not firmware's leftovers).
fn test_gdt_installed() -> Result<(), &'static str> {
    let (base, limit) = gdt::read_gdtr();
    let (want_base, want_limit) = gdt::expected_gdt();
    info!("m1", "gdt: sgdt read-back base={base:#x} limit={limit:#x}");
    if base != want_base || limit != want_limit {
        return Err("sgdt does not report our GDT (base/limit mismatch)");
    }
    Ok(())
}

/// After our `lidt`, `sidt` must report *our* table — and the very next
/// test proves interrupts actually flow through it.
fn test_idt_installed() -> Result<(), &'static str> {
    let (base, limit) = x86_64::idt::read_idtr();
    let (want_base, want_limit) = x86_64::idt::expected_idt();
    info!("m1", "idt: sidt read-back base={base:#x} limit={limit:#x}");
    if base != want_base || limit != want_limit {
        return Err("sidt does not report our IDT (base/limit mismatch)");
    }
    // Regression guard: verify the *encoding* of every live gate (selector,
    // type/attrs byte at offset 5, IST byte at offset 4 per SDM Fig. 6-7).
    // A misplaced attrs byte here once made every delivery through our IDT
    // triple-fault with #GP(intno*8+2) — the read-back alone can't see it.
    let (bad_sel, bad_attr, bad_off) = x86_64::idt::audit_gates();
    if bad_sel != 0 || bad_attr != 0 || bad_off != 0 {
        error!(
            "m1",
            "idt: audit found bad gates: sel={bad_sel} attrs={bad_attr} offs={bad_off}"
        );
        return Err("IDT gate encoding audit failed");
    }
    Ok(())
}

/// GetMemoryMap from real firmware, classified; a 512 MiB VM must show a
/// sane amount of conventional memory. Also dumps the largest regions so the
/// log shows *what firmware actually reported*.
fn test_memory_map() -> Result<(), &'static str> {
    let summary = match bootinfo::capture() {
        Ok(s) => s,
        Err(status) => {
            error!("m1", "GetMemoryMap failed, status={status:#x}");
            return Err("UEFI GetMemoryMap did not succeed");
        }
    };
    info!(
        "bootinfo",
        "regions={} desc_size={} version={} map_key={:#x}",
        summary.region_count,
        summary.descriptor_size,
        summary.descriptor_version,
        summary.map_key
    );
    info!(
        "bootinfo",
        "memory: conventional={}MiB reclaimable={}MiB largest_free={}MiB@{:#x} total_described={}MiB (incl. reserved/MMIO spans)",
        summary.conventional_mib(),
        summary.reclaimable_mib(),
        summary.largest_conventional_mib(),
        summary.largest_conventional_base,
        summary.total_mib()
    );

    // Dump up to 8 regions (human-readable evidence of real parsing).
    let mut shown = 0usize;
    // SAFETY: runs immediately after a successful capture(), single-threaded
    // boot context — exactly the helper's contract.
    unsafe {
        bootinfo::for_each_region(&summary, |d| {
            if shown < 8 {
                info!(
                    "bootinfo",
                    "region[{shown}]: type={} phys={:#x} pages={} ({:#x} attr)",
                    crate::uefi::memory_type::name(d.memory_type),
                    d.physical_start,
                    d.number_of_pages,
                    d.attribute
                );
                shown += 1;
            }
        });
    }

    if summary.region_count == 0 {
        return Err("firmware reported zero memory-map regions");
    }
    if summary.descriptor_size < core::mem::size_of::<crate::uefi::MemoryDescriptor>() {
        return Err("descriptor size below UEFI v1 struct size");
    }
    if summary.conventional_mib() < 64 {
        return Err("less than 64MiB conventional memory in a 512MiB VM — parsing is wrong");
    }
    Ok(())
}

/// 128-bit multiply/divide through `compiler_builtins` (__multiu128,
/// __udivti3) with black_box inputs so LLVM cannot fold it at compile time.
/// The expected value is computed by the compiler *earlier*, in const
/// context — agreement proves the runtime intrinsic library we bootstrapped
/// offline is real and correct.
fn test_wide_math() -> Result<(), &'static str> {
    const A: u128 = 0xDEAD_BEEF_CAFE_BABE;
    const B: u128 = 0x1234_5678_9ABC_DEF1;
    const EXPECTED: u128 = (A * B) / 7; // fits: ~2.7e37 < u128::MAX

    let a = core::hint::black_box(A);
    let b = core::hint::black_box(B);
    let got = (a * b) / 7;
    info!("m1", "wide_math: ({a:#x} * {b:#x}) / 7 = {got:#x}");
    if got != EXPECTED {
        return Err("u128 arithmetic disagrees with compile-time evaluation");
    }
    Ok(())
}

/// Prove real hardware interrupts traverse our IDT: EDK2 programs the 8254
/// PIT (we saw its vector-32 IRQs while still on the firmware IDT), so
/// briefly raising IF must tick the absorb-counter inside our stub —
/// followed by a clean `iretq` back here. This is the M1 regression guard
/// for the triple-fault class documented in idt.rs.
fn test_timer_irq_absorbed() -> Result<(), &'static str> {
    // Platform state (probed at runtime + confirmed against QEMU's own
    // interrupt trace and device models): EDK2/OVMF hands off with the PIT
    // routed IOAPIC → LAPIC as vector 32, and the legacy 8259 pair fully
    // masked (IMR=0xFF/0xFF) with its vector base left UNPROGRAMMED. The
    // LAPIC path is therefore the tick source, and the stub's LAPIC EOI is
    // what lets ticks keep flowing. Do NOT unmask 8259 IRQ0 to "help": with
    // irq_base=0 the ExtINT acknowledge through LINT0 delivers vector 0x00
    // — a #DE straight into our gate 0 (observed live; docs/TESTING.md).
    //
    // We require TWO absorbed ticks, not one: the second only arrives if the
    // stub's EOI actually cleared the controller's in-service state — proof
    // of the full deliver → absorb → EOI → re-deliver cycle, not just one
    // lucky pending edge.
    let before = x86_64::idt::absorbed_irq_count();

    // Our IDT is installed (verified by an earlier test) with an
    // absorb+EOI stub for every external vector; single CPU. IF is re-masked
    // on every exit path below. Ticks arrive every ~55 ms (PIT 18.2 Hz), so
    // two ticks need ~110 ms — the spin bound (~seconds) is a safety net.
    x86_64::sti();
    const SPIN_BOUND: u64 = 400_000_000;
    const REQUIRED_TICKS: u64 = 2;
    let mut spin = 0u64;
    let mut absorbed;
    loop {
        absorbed = x86_64::idt::absorbed_irq_count() - before;
        if absorbed >= REQUIRED_TICKS {
            break;
        }
        spin += 1;
        if spin >= SPIN_BOUND {
            break;
        }
        core::hint::spin_loop();
    }
    x86_64::cli();

    if absorbed < REQUIRED_TICKS {
        error!(
            "m1",
            "timer_irq: only {absorbed}/{REQUIRED_TICKS} ticks absorbed (spin={spin})"
        );
        return Err("fewer external interrupts absorbed than required within spin bound");
    }
    info!(
        "m1",
        "timer_irq: absorbed {absorbed} hardware IRQ(s) through our IDT (EOI cycle verified, spin={spin})"
    );
    Ok(())
}

/// Run all tests, emit markers, return (passed, total).
pub fn run_all() -> (usize, usize) {
    let mut passed = 0usize;
    for (name, test) in TESTS {
        match test() {
            Ok(()) => {
                passed += 1;
                log::write_marker(format_args!("m1:test:{name}: PASS"));
            }
            Err(reason) => {
                error!("m1", "test {name} failed: {reason}");
                log::write_marker(format_args!("m1:test:{name}: FAIL ({reason})"));
            }
        }
    }
    let total = TESTS.len();
    if passed == total {
        log::write_marker(format_args!("m1: RESULT PASS ({passed}/{total})"));
    } else {
        log::write_marker(format_args!("m1: RESULT FAIL ({passed}/{total})"));
    }
    (passed, total)
}
