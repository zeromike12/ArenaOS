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

use crate::arch::x86_64::{
    cr0, efer, faults, gdt, idt, paging, read_cr0, read_cr3, read_efer, tss,
};
use crate::bootinfo;
use crate::drivers::pit;
use crate::frames;
use crate::heap;
use crate::log::{self, log_error as error, log_info as info};
use crate::timekeeping;
use arena_heap::HeapError;
use core::alloc::Layout;
use core::ptr::NonNull;

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
    ("frame_allocator", test_frame_allocator),
    ("vm_address_space", test_vm_address_space),
    ("vm_write_protect", test_vm_write_protect),
    ("vm_nx", test_vm_nx),
    ("heap_basics", test_heap_basics),
    ("heap_guards", test_heap_guards),
    ("heap_stress", test_heap_stress),
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

    /// #PF: 8-byte write to `addr` — caller guarantees the write faults
    /// (address unmapped, or mapped read-only with CR0.WP=1).
    ///
    /// # Safety
    /// Call only with `faults::arm(14)` in effect; see `divide_by_zero`.
    pub unsafe fn write_to(addr: u64) {
        // SAFETY: as `divide_by_zero`; faulting is the caller's contract
        // (unmapped or RO → guaranteed #PF, never silent success).
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

    /// #PF: instruction fetch at `addr` via `jmp` — caller guarantees the
    /// page is mapped NX, so the *first fetch* faults before any
    /// instruction is decoded. `jmp` (not `call`) keeps RSP untouched, so
    /// the resume path returns to a balanced stack.
    ///
    /// # Safety
    /// Call only with `faults::arm(14)` in effect; see `divide_by_zero`.
    pub unsafe fn execute_at(addr: u64) {
        // SAFETY: as `write_to`; the faulting fetch is the point of the call.
        unsafe {
            core::arch::asm!(
                "lea rax, [rip + 2f]",
                "mov [rip + {resume}], rax",
                "mov rax, {addr}",
                "jmp rax", // #PF (I/D) — handler resumes execution at 2:
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
    unsafe { fault_sites::write_to(UNMAPPED_CANONICAL_ADDR) };
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

// ---------------------------------------------------------------------------
// M2.3 — physical memory: the frame allocator
// ---------------------------------------------------------------------------

/// Is `base` inside one of the captured conventional regions? Independent
/// check for the test side (the allocator itself never consults regions
/// after init — its bitmap *is* the region set).
fn in_conventional(base: u64) -> bool {
    (0..bootinfo::usable_region_count()).any(|i| {
        let r = bootinfo::usable_region(i);
        base >= r.base && base < r.base + r.pages * 4096
    })
}

/// The allocator must manage exactly the clipped conventional regions, and
/// alloc/free must stay honest under stress: unique in-region frame bases,
/// real writable RAM (pattern round-trip), exact free-count accounting,
/// double-free rejection, contiguous runs, and a measured throughput
/// benchmark (the "small benchmark" of ADR-0007, logged for the record).
fn test_frame_allocator() -> Result<(), &'static str> {
    if !frames::ready() {
        return Err("allocator was not initialized at boot");
    }
    if bootinfo::usable_region_overflow() != 0 {
        return Err("conventional regions exceeded the storage cap");
    }

    // (1) Coverage: managed total must equal the independently recomputed
    // clipped-region frame count (same policy, separately derived here).
    let mut expected = 0u64;
    for i in 0..bootinfo::usable_region_count() {
        let r = bootinfo::usable_region(i);
        let start = r.base.max(frames::RESERVE_BELOW).div_ceil(4096) * 4096;
        let end = (r.base + r.pages * 4096).min(frames::PHYS_LIMIT) / 4096 * 4096;
        if end > start {
            expected += (end - start) / 4096;
        }
    }
    if frames::total_frames() != expected {
        return Err("managed frame count disagrees with the memory map");
    }
    // Boot infrastructure legitimately owns frames before this test runs
    // (M2.4 page tables are allocated from here). Bound the pre-test
    // consumption instead of demanding zero.
    let in_use = expected - frames::free_frames();
    if in_use > 64 {
        return Err("implausible frame consumption before the test ran");
    }
    info!(
        "m2",
        "frame_allocator: managing {} frames ({} MiB) from {} conventional regions; {} frames owned by boot infrastructure (page tables)",
        frames::total_frames(),
        frames::total_frames() * 4096 / (1024 * 1024),
        bootinfo::usable_region_count(),
        in_use
    );

    // (2) Alloc a batch: unique, aligned, in-region, and real writable RAM.
    const N: usize = 256;
    let mut held = [0u64; N];
    let free_before = frames::free_frames();
    for slot in held.iter_mut() {
        *slot = frames::alloc().ok_or("allocator exhausted early")?;
    }
    if frames::free_frames() != free_before - N as u64 {
        return Err("free count drifted during batch alloc");
    }
    for (i, &b) in held.iter().enumerate() {
        if b % 4096 != 0 || !in_conventional(b) {
            return Err("frame outside managed conventional memory or misaligned");
        }
        if held[..i].contains(&b) {
            return Err("duplicate frame handed out");
        }
        // SAFETY: b is an allocated conventional frame, identity-mapped by
        // firmware (pre-ExitBootServices), exclusively ours right now.
        unsafe {
            (b as *mut u64).write(b ^ 0x5A5A_A5A5_5A5A_A5A5);
            if (b as *const u64).read() != b ^ 0x5A5A_A5A5_5A5A_A5A5 {
                return Err("frame content did not round-trip (not real RAM?)");
            }
        }
    }

    // (3) Double-free must be rejected; freeing all must restore accounting.
    frames::free(held[0]).map_err(|_| "legitimate free rejected")?;
    if frames::free(held[0]).is_ok() {
        return Err("double free was accepted");
    }
    for &b in held.iter().skip(1) {
        frames::free(b).map_err(|_| "legitimate free rejected")?;
    }
    if frames::free_frames() != free_before {
        return Err("free count not restored after free-all");
    }

    // (4) Contiguous run: 16 frames, pattern across the whole span.
    let c = frames::alloc_contiguous(16).ok_or("contiguous alloc failed")?;
    if c % 4096 != 0 || !in_conventional(c) || !in_conventional(c + 15 * 4096) {
        frames::free_contiguous(c, 16).ok();
        return Err("contiguous run outside managed memory");
    }
    for k in 0..16u64 {
        // SAFETY: allocated contiguous run, identity-mapped, exclusively ours.
        unsafe { ((c + k * 4096) as *mut u64).write(!k) };
    }
    for k in 0..16u64 {
        // SAFETY: plain read-back of the writes above.
        if unsafe { ((c + k * 4096) as *const u64).read() } != !k {
            return Err("contiguous run content did not round-trip");
        }
    }
    frames::free_contiguous(c, 16).map_err(|_| "contiguous free rejected")?;
    if frames::free_frames() != free_before {
        return Err("free count not restored after contiguous run");
    }

    // (5) Stress: 200 interleaved alloc/free rounds with exact accounting.
    let mut buf = [0u64; 64];
    for _round in 0..200 {
        for slot in buf.iter_mut() {
            *slot = frames::alloc().ok_or("stress alloc failed")?;
        }
        for k in (0..64).step_by(2) {
            frames::free(buf[k]).map_err(|_| "stress free rejected")?;
        }
        for k in (1..64).step_by(2) {
            frames::free(buf[k]).map_err(|_| "stress free rejected")?;
        }
        if frames::free_frames() != free_before {
            return Err("stress round leaked or lost frames");
        }
    }

    // (6) Benchmark (ADR-0007 evidence): hot-path alloc+free throughput.
    let t0 = timekeeping::now_ticks();
    for _ in 0..2048 {
        let f = frames::alloc().ok_or("benchmark alloc failed")?;
        frames::free(f).map_err(|_| "benchmark free rejected")?;
    }
    let dt_us = timekeeping::ticks_to_us(timekeeping::now_ticks() - t0);
    info!(
        "m2",
        "frame_allocator: stress 200x64 OK; benchmark 2048 alloc+free in {dt_us}us (~{} ns/op)",
        dt_us * 1000 / 2048
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// M2.4 — kernel address space: higher-half alias, CR0.WP + EFER.NXE, W^X
// ---------------------------------------------------------------------------

/// Magic returned by the .text probe — proves the higher-half *call* really
/// executed our code rather than returning a plausible default.
const VM_PROBE_MAGIC: u64 = 0x2464_A0FF_EE00_5A5A;

/// .text probe: its ADDRESS is the subject (a known byte sequence inside the
/// R+X window); the body just returns the magic.
#[inline(never)]
fn vm_probe_fn() -> u64 {
    core::hint::black_box(VM_PROBE_MAGIC)
}

/// .data/.bss probe: an RW+NX page target for the execute-fault test and a
/// round-trip cell for the alias test. Accessed only through raw volatile
/// pointers at known addresses — no references, no aliasing rules bent.
static VM_PROBE_DATA: crate::sync::SyncCell<u64> = crate::sync::SyncCell::new(0);

/// The installed address space must be live and correct: CR3 is our PML4,
/// WP+NXE are on, the higher-half alias reads/writes the same physical
/// bytes as the identity address (both directions), and a call *through*
/// the higher-half alias of a .text address executes and returns.
fn test_vm_address_space() -> Result<(), &'static str> {
    let cr3 = paging::cr3_phys();
    let live = read_cr3();
    info!("m2", "vm_address_space: cr3={live:#x} (ours {cr3:#x})");
    if cr3 == 0 || live & 0x000F_FFFF_FFFF_F000 != cr3 {
        return Err("live CR3 does not match the PML4 paging::init installed");
    }
    if read_cr0() & cr0::WP == 0 {
        return Err("CR0.WP is not set — ring-0 writes would ignore RO pages");
    }
    if read_efer() & efer::NXE == 0 {
        return Err("EFER.NXE is not set — PTE bit 63 would be ignored");
    }

    // black_box the symbol-derived base BEFORE adding KERNEL_OFFSET.
    // Constant-folding `symbol + 0xFFFF_FFFF_8000_0000` lets LLVM emit a
    // 32-bit RIP-relative materialization whose addend silently wraps —
    // the higher half must be computed by a runtime 64-bit add. (This bit
    // us for real: see ADR-0008 "compiler hazards".)
    let ident = core::hint::black_box(core::ptr::addr_of!(VM_PROBE_DATA) as u64);
    let high = ident + paging::KERNEL_OFFSET;
    if paging::direct_map_to_phys(high) != Some(ident) {
        return Err("direct_map_to_phys disagrees with KERNEL_OFFSET arithmetic");
    }
    // SAFETY: both addresses denote the same mapped, owned static (identity
    // and direct-map views of one frame); volatile to defeat caching;
    // single-CPU boot context, IF=0.
    unsafe {
        (high as *mut u64).write_volatile(0xDEAD_BEEF_0000_0001);
        if (ident as *const u64).read_volatile() != 0xDEAD_BEEF_0000_0001 {
            return Err("higher-half write not visible at the identity address");
        }
        (ident as *mut u64).write_volatile(0x0BAD_F00D_0000_0002);
        if (high as *const u64).read_volatile() != 0x0BAD_F00D_0000_0002 {
            return Err("identity write not visible at the higher-half address");
        }
    }

    // Execute through the higher-half alias of a .text address.
    let fn_base = core::hint::black_box(vm_probe_fn as *const () as u64);
    let fn_high = fn_base + paging::KERNEL_OFFSET;
    // SAFETY: fn_high is the direct-map alias of vm_probe_fn's address —
    // mapped R+X by construction (the test then proves it by running it).
    // black_box keeps the call indirect (see comment above the fold trap).
    let probe: fn() -> u64 = unsafe { core::mem::transmute(core::hint::black_box(fn_high)) };
    let got = probe();
    info!(
        "m2",
        "vm_address_space: alias round-trip OK; higher-half call at {fn_high:#x} returned {got:#x}"
    );
    if got != VM_PROBE_MAGIC {
        return Err("higher-half call to the .text probe returned the wrong magic");
    }
    Ok(())
}

/// W^X, write side: a ring-0 write through the higher-half alias of our own
/// .text must fault — the page is R+X and CR0.WP makes RO bind for the
/// kernel itself. Expected: #PF, CR2 = faulting VA, ec = present+write.
fn test_vm_write_protect() -> Result<(), &'static str> {
    let text_va = core::hint::black_box(vm_probe_fn as *const () as u64) + paging::KERNEL_OFFSET;
    faults::arm(14);
    // SAFETY: armed above; text_va is mapped read-only — write faults by
    // construction (that is the assertion).
    unsafe { fault_sites::write_to(text_va) };
    let obs = faults::observed();
    faults::disarm();
    info!(
        "m2",
        "vm_write_protect: write to RO .text at {text_va:#x}: vector={:#x} ec={:#x} cr2={:#x}",
        obs.vector,
        obs.error_code,
        obs.cr2
    );
    if !obs.valid || obs.vector != 14 {
        return Err("ring-0 write to RO .text did not raise #PF (is CR0.WP honored?)");
    }
    if obs.cr2 != text_va {
        return Err("CR2 does not name the RO text address we wrote");
    }
    if obs.error_code & 0b1_1111 != 0b0_0011 {
        return Err("error code is not present+write for a ring-0 RO write");
    }
    Ok(())
}

/// W^X, execute side: an instruction fetch through the higher-half alias of
/// a .data/.bss page must fault before decoding anything. Expected: #PF,
/// CR2 = faulting VA, ec = present + instruction-fetch (I/D bit).
fn test_vm_nx() -> Result<(), &'static str> {
    let data_va =
        core::hint::black_box(core::ptr::addr_of!(VM_PROBE_DATA) as u64) + paging::KERNEL_OFFSET;
    faults::arm(14);
    // SAFETY: armed above; data_va is mapped RW+NX — the first fetch faults
    // by construction; `jmp` leaves RSP balanced for the resume.
    unsafe { fault_sites::execute_at(data_va) };
    let obs = faults::observed();
    faults::disarm();
    info!(
        "m2",
        "vm_nx: fetch at NX data page {data_va:#x}: vector={:#x} ec={:#x} cr2={:#x}",
        obs.vector,
        obs.error_code,
        obs.cr2
    );
    if !obs.valid || obs.vector != 14 {
        return Err("instruction fetch from an NX page did not raise #PF");
    }
    if obs.cr2 != data_va {
        return Err("CR2 does not name the NX data address we fetched");
    }
    if obs.error_code & 0b1_1111 != 0b1_0001 {
        return Err("error code is not present+instruction-fetch for an NX violation");
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// M2.5 — kernel heap: arena-heap core over frames, guards, stress
// ---------------------------------------------------------------------------

fn heap_layout(size: usize, align: usize) -> Result<Layout, &'static str> {
    Layout::from_size_align(size, align).map_err(|_| "invalid heap layout")
}

/// Basic contract: allocations are distinct, aligned, real writable RAM;
/// accounting is exact; typed alloc/free works; a larger alignment request
/// is honored; freeing everything returns the heap to zero.
fn test_heap_basics() -> Result<(), &'static str> {
    let a = heap::alloc(heap_layout(64, 16)?).ok_or("alloc(64) returned None")?;
    let b = heap::alloc(heap_layout(4096, 16)?).ok_or("alloc(4096) returned None")?;
    let c = heap::alloc_typed::<u64>().ok_or("alloc_typed::<u64> returned None")?;
    let d = heap::alloc(heap_layout(100, 256)?).ok_or("align-256 alloc returned None")?;
    let addrs = [
        a.as_ptr() as usize,
        b.as_ptr() as usize,
        c.as_ptr() as usize,
        d.as_ptr() as usize,
    ];
    for (i, x) in addrs.iter().enumerate() {
        if addrs[i + 1..].contains(x) {
            return Err("two live allocations share an address");
        }
    }
    if !addrs[0].is_multiple_of(16) || !addrs[1].is_multiple_of(16) || !addrs[3].is_multiple_of(256)
    {
        return Err("alignment contract violated");
    }
    if heap::in_use_bytes() != 64 + 4096 + 8 + 100 || heap::blocks_live() != 4 {
        return Err("in-use accounting does not match the live allocations");
    }
    // SAFETY: a..d are live, owned, correctly sized allocations from our
    // own heap; single-CPU boot context.
    unsafe {
        core::ptr::write_bytes(a.as_ptr(), 0x5A, 64);
        core::ptr::write_bytes(b.as_ptr(), 0xA5, 4096);
        c.as_ptr().write_volatile(0x1234_5678_9ABC_DEF0);
        core::ptr::write_bytes(d.as_ptr(), 0x33, 100);
        if *a.as_ptr() != 0x5A
            || *b.as_ptr().add(4095) != 0xA5
            || c.as_ptr().read_volatile() != 0x1234_5678_9ABC_DEF0
            || *d.as_ptr().add(99) != 0x33
        {
            return Err("heap payload pattern round-trip failed");
        }
        heap::free(a).map_err(|_| "free(a) was rejected")?;
        heap::free(b).map_err(|_| "free(b) was rejected")?;
        heap::free_typed(c).map_err(|_| "free_typed(c) was rejected")?;
        heap::free(d).map_err(|_| "free(d) was rejected")?;
    }
    if heap::in_use_bytes() != 0 || heap::blocks_live() != 0 {
        return Err("accounting did not return to zero after freeing all");
    }
    info!(
        "m2",
        "heap_basics: distinct/aligned/patterned; chunks={} reserved={}KiB",
        heap::chunk_count(),
        heap::bytes_reserved() / 1024
    );
    Ok(())
}

/// Guard contract: double free, red-zone overflow, and a foreign pointer
/// are each rejected with the exact error and without mutating the heap;
/// after repairing the overflow the block frees cleanly (rejections leave
/// the heap usable, not merely loud). The three expected ERROR lines on
/// serial are the evidence trail.
fn test_heap_guards() -> Result<(), &'static str> {
    let l = heap_layout(32, 16)?;
    // SAFETY: p/q come from our own heap; every free below is a deliberate
    // probe of a rejection path (documented safe contract of Heap::free).
    unsafe {
        let p = heap::alloc(l).ok_or("alloc p failed")?;
        heap::free(p).map_err(|_| "first free of p was rejected")?;
        if heap::free(p) != Err(HeapError::DoubleFree) {
            return Err("double free was not rejected as DoubleFree");
        }
        let q = heap::alloc(l).ok_or("alloc q failed")?;
        // Clobber the trailing red-zone word's first byte (0xA5 in LE).
        *q.as_ptr().add(32) = 0x99;
        if heap::free(q) != Err(HeapError::RedZoneBack) {
            return Err("red-zone overflow was not rejected as RedZoneBack");
        }
        // A .bss static is not from the heap.
        let decoy = NonNull::new(core::ptr::addr_of!(VM_PROBE_DATA) as *mut u8)
            .ok_or("decoy pointer was null")?;
        if heap::free(decoy) != Err(HeapError::NotFromHeap) {
            return Err("foreign pointer was not rejected as NotFromHeap");
        }
        // Repair q's red zone: the rejected free left it live and intact.
        *q.as_ptr().add(32) = 0xA5;
        heap::free(q).map_err(|_| "repaired q was rejected")?;
    }
    info!(
        "m2",
        "heap_guards: DoubleFree/RedZoneBack/NotFromHeap rejected; heap usable after each"
    );
    Ok(())
}

/// In-guest stress (the host suite covers logic depth): 60 rounds over 32
/// slots of xorshift-driven alloc/free with per-slot tags verified before
/// every free, exact accounting restored to the pre-test baseline, and a
/// 16 KiB post-stress allocation as the coalescing proof. Timed and logged
/// as ns/op evidence.
fn test_heap_stress() -> Result<(), &'static str> {
    let base_use = heap::in_use_bytes();
    let base_live = heap::blocks_live();
    let base_count = heap::alloc_count();
    let mut slots = [None::<NonNull<u8>>; 32];
    let mut tags = [0u64; 32];
    let mut rng = 0x2545_F491_4F6C_DD1Du64;
    let mut ops = 0u64;
    let t0 = timekeeping::now_ticks();
    for round in 0..60u64 {
        for i in 0..slots.len() {
            rng ^= rng >> 12;
            rng ^= rng << 25;
            rng ^= rng >> 27;
            let r = rng.wrapping_mul(0x2545_F491_4F6C_DD1D);
            match slots[i] {
                None => {
                    let size = 8 + (r >> 33) as usize % 1024;
                    // Exhaustion is tolerated (slot stays empty) — the
                    // accounting check below is the real invariant.
                    if let Some(p) = heap::alloc(heap_layout(size, 16)?) {
                        tags[i] = (round << 40) ^ ((i as u64) << 8) ^ (size as u64);
                        // SAFETY: p is a live owned allocation of `size`
                        // bytes; unaligned tag write at its start.
                        unsafe { (p.as_ptr() as *mut u64).write_unaligned(tags[i]) };
                        slots[i] = Some(p);
                        ops += 1;
                    }
                }
                Some(p) => {
                    // SAFETY: p is live and exclusively ours.
                    unsafe {
                        if (p.as_ptr() as *const u64).read_unaligned() != tags[i] {
                            return Err("payload tag corrupted while live");
                        }
                        heap::free(p).map_err(|_| "stress free was rejected")?;
                    }
                    slots[i] = None;
                    ops += 1;
                }
            }
        }
    }
    for slot in slots.iter_mut() {
        if let Some(p) = *slot {
            // SAFETY: draining our own live allocations.
            unsafe { heap::free(p).map_err(|_| "drain free was rejected")? };
            *slot = None;
            ops += 1;
        }
    }
    if heap::in_use_bytes() != base_use || heap::blocks_live() != base_live {
        return Err("stress did not restore exact accounting");
    }
    let big = heap::alloc(heap_layout(16384, 16)?)
        .ok_or("post-stress 16KiB alloc failed — coalescing broken")?;
    // SAFETY: big is a live owned allocation.
    unsafe { heap::free(big).map_err(|_| "big free was rejected")? };
    let dt_us = timekeeping::ticks_to_us(timekeeping::now_ticks() - t0);
    info!(
        "m2",
        "heap_stress: {ops} alloc/free ops in {dt_us}us (~{} ns/op), chunks={}, reserved={}KiB, this-test allocs={}",
        dt_us * 1000 / ops.max(1),
        heap::chunk_count(),
        heap::bytes_reserved() / 1024,
        heap::alloc_count() - base_count
    );
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
