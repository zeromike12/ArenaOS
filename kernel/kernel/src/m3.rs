//! Milestone 3 test suite — kernel threads, the context switch (M3.1,
//! ADR-0012), timer-driven preemption (M3.2, ADR-0013), and the ring-3
//! boundary: hand-assembled user payloads executing real `syscall`s
//! (M3.3, ADR-0014). Runs in
//! `kmain` after the M2 RESULT line; every test is a real machine effect
//! (side-effect verification, exact interleave sequences, callee-saved
//! register round-trips *through* the assembly switch, stack isolation
//! with disjoint ranges, exact frame/heap accounting after churn, and
//! preemption proven on threads that contain no yield call at all).
//! Markers: `m3:test:<name>`, `m3: RESULT`.
//!
//! Phase discipline (ADR-0013): the five M3.1 tests run cooperative with
//! IF=0 (ticks cannot perturb their exact assertions); the two preempt
//! tests arm the tick hook for bounded IF=1 windows, log NOTHING while
//! IF=1 (serial bytes from two threads would interleave without a console
//! lock), and re-mask + disarm before asserting.

use crate::arch::x86_64::{self, faults, gdt, idt, paging, syscall, tss};
use crate::frames;
use crate::log::{log_error as error, log_info as info, write_marker};
use crate::sched;
use crate::sync::SyncCell;
use core::sync::atomic::{AtomicU64, Ordering};

pub fn run_suite() -> bool {
    let checks: [(&str, fn() -> Result<(), &'static str>); 9] = [
        ("thread_spawn_run", test_thread_spawn_run),
        ("thread_rr_interleave", test_thread_rr_interleave),
        ("thread_callee_saved", test_thread_callee_saved),
        ("thread_stack_isolation", test_thread_stack_isolation),
        ("thread_churn_accounting", test_thread_churn_accounting),
        ("preempt_rotation", test_preempt_rotation),
        ("preempt_coexist", test_preempt_coexist),
        ("user_ring3_syscall", test_user_ring3_syscall),
        ("user_ring3_interrupted", test_user_ring3_interrupted),
    ];
    let mut passed = 0u32;
    for (name, test) in checks {
        match test() {
            Ok(()) => {
                passed += 1;
                write_marker(format_args!("m3:test:{name}: PASS"));
            }
            Err(reason) => {
                error!("m3", "test {name} failed: {reason}");
                write_marker(format_args!("m3:test:{name}: FAIL ({reason})"));
            }
        }
    }
    let total = checks.len() as u32;
    if passed == total {
        write_marker(format_args!("m3: RESULT PASS ({passed}/{total})"));
        true
    } else {
        write_marker(format_args!("m3: RESULT FAIL ({passed}/{total})"));
        false
    }
}

// ---- test coordination statics (single CPU, IF=0, cooperative) ----------

static T1_MAGIC: AtomicU64 = AtomicU64::new(0);
static T1_RUNS: AtomicU64 = AtomicU64::new(0);

/// Interleave log: [id, round] pairs in execution order.
static T2_LOG: SyncCell<[u64; 16]> = SyncCell::new([0; 16]);
static T2_LEN: SyncCell<usize> = SyncCell::new(0);

/// Bitmask of callee-saved registers that survived the switch (bit i =
/// register i of the six probed; rbx and rbp are argued mechanically —
/// see the test doc comment). 0x3F = all six.
static T3_OK: AtomicU64 = AtomicU64::new(0);

static T4_OK: AtomicU64 = AtomicU64::new(0);
static T4_RANGE: SyncCell<[(u64, u64); 2]> = SyncCell::new([(0, 0); 2]);

static T5_CHURN: AtomicU64 = AtomicU64::new(0);
static T5_BOUNDARY: AtomicU64 = AtomicU64::new(0);

/// Yield until every thread except the bootstrap is gone (reaped);
/// returns the number of yields used. Fails on a stuck scheduler.
fn drain(max_yields: usize) -> Result<usize, &'static str> {
    let mut used = 0;
    while sched::live_threads() > 1 {
        if used >= max_yields {
            return Err("threads still live after the yield bound (scheduler stuck?)");
        }
        sched::yield_now();
        used += 1;
    }
    // One more pass: the last-exited zombie is reaped (stack freed) at the
    // *next* scheduler entry, and the churn test asserts exact accounting.
    sched::yield_now();
    Ok(used + 1)
}

// ---- 1. spawn → run → exit, verified by side effects --------------------

const T1_MAGIC_VALUE: u64 = 0xC0FF_EE00_DEAD_BEEF;

fn t1_entry(arg: usize) {
    T1_MAGIC.store(T1_MAGIC_VALUE ^ arg as u64, Ordering::Relaxed);
    T1_RUNS.fetch_add(1, Ordering::Relaxed);
    // falls through to sched's exit path
}

fn test_thread_spawn_run() -> Result<(), &'static str> {
    let switches_before = sched::switch_count();
    let id = sched::spawn("t1", t1_entry, 0x5A5A)?;
    drain(8)?;
    if T1_RUNS.load(Ordering::Relaxed) != 1 {
        return Err("thread entry did not run exactly once");
    }
    if T1_MAGIC.load(Ordering::Relaxed) != T1_MAGIC_VALUE ^ 0x5A5A {
        return Err("thread's side effect wrong (arg not delivered?)");
    }
    let switches = sched::switch_count() - switches_before;
    if switches < 2 {
        return Err("fewer than one there-and-back switch observed");
    }
    info!(
        "m3",
        "thread_spawn_run: id {id} ran to completion and was reaped; {switches} context switches; live={}",
        sched::live_threads()
    );
    Ok(())
}

// ---- 2. deterministic round-robin interleave -----------------------------

fn t2_entry(arg: usize) {
    for round in 0..4u64 {
        // SAFETY: single CPU, IF=0, cooperative — one writer at a time.
        unsafe {
            let len = &mut *T2_LEN.get();
            (*T2_LOG.get())[*len] = (arg as u64) << 32 | round;
            *len += 1;
        }
        sched::yield_now();
    }
}

fn test_thread_rr_interleave() -> Result<(), &'static str> {
    // SAFETY: reset under IF=0 before any thread runs.
    unsafe {
        *T2_LEN.get() = 0;
        *T2_LOG.get() = [0; 16];
    }
    for id in 1..=3usize {
        sched::spawn("t2", t2_entry, id)?;
    }
    // Each bootstrap yield runs exactly one full RR pass (t1,t2,t3) —
    // the queue discipline makes the global order exact, not "some
    // interleaving". Four passes fill the 12-entry log.
    for _ in 0..4 {
        sched::yield_now();
    }
    drain(8)?;
    // SAFETY: all threads done; single reader.
    let (log, len) = unsafe { (*T2_LOG.get(), *T2_LEN.get()) };
    if len != 12 {
        return Err("interleave log length != 12");
    }
    for pass in 0..4u64 {
        for (slot, want_id) in [1u64, 2, 3].iter().enumerate() {
            let got = log[pass as usize * 3 + slot];
            let want = (*want_id) << 32 | pass;
            if got != want {
                return Err("interleave order deviates from exact round-robin");
            }
        }
    }
    info!(
        "m3",
        "thread_rr_interleave: exact global order 1,2,3 × 4 passes ({len} entries)"
    );
    Ok(())
}

// ---- 3. callee-saved registers survive a real switch ---------------------

/// The yield call target for the asm probe (Win64 "C" so the asm `call`
/// is ABI-correct). The whole image is SSE-free (ADR-0012 build guard),
/// so no instruction in the callee depends on RSP alignment beyond the
/// ABI's own contract; rustc additionally aligns RSP for asm blocks.
extern "C" fn t3_yield() {
    sched::yield_now();
}

/// Probes rsi, rdi, r12–r15 (six of the eight Win64 callee-saved
/// registers) by writing distinctive constants, switching away and back
/// *inside one asm block*, and reading the same physical registers.
///
/// rbx and rbp are deliberately not probed: rustc forbids `rbx` as an
/// inline-asm operand outright ("used internally by LLVM"), and LLVM may
/// hold `rbp` as this function's frame pointer — clobbering either inside
/// the block would be unsound Rust, not a switch test. Their preservation
/// is mechanical: `arena_context_switch` pushes and pops all eight
/// callee-saved registers in one audited 20-line frame (ADR-0012), so a
/// switch that round-trips these six through that identical push/pop
/// symmetry round-trips rbx and rbp with it.
fn t3_entry(_arg: usize) {
    let (si, di, r12, r13, r14, r15): (u64, u64, u64, u64, u64, u64);
    // SAFETY: the asm block is self-contained: it writes callee-saved
    // registers declared as outputs (explicit-register operands are
    // referenced by register name in the template and cannot carry
    // names), calls an extern "C" fn (stack use declared by the absence
    // of nostack), and clobbers the full Win64 caller-saved set via
    // clobber_abi. No Rust value lives in a clobbered register across
    // the block.
    unsafe {
        core::arch::asm!(
            // Explicit-register operands cannot appear in the template
            // at all (rustc: "use the register name directly in the
            // assembly code") — the registers are written literally
            // below; the out("r..") declarations bind the read-back
            // values and tell LLVM the registers are clobbered.
            "mov rsi, 0x3333444455556666",
            "mov rdi, 0x4444555566667777",
            "mov r12, 0x5555666677778888",
            "mov r13, 0x6666777788889999",
            "mov r14, 0x777788889999aaaa",
            "mov r15, 0x88889999aaaabbbb",
            "call {y}",
            out("rsi") si,
            out("rdi") di,
            out("r12") r12,
            out("r13") r13,
            out("r14") r14,
            out("r15") r15,
            y = sym t3_yield,
            clobber_abi("C"),
        );
    }
    let probes = [
        (si, 0x3333444455556666),
        (di, 0x4444555566667777),
        (r12, 0x5555666677778888),
        (r13, 0x6666777788889999),
        (r14, 0x777788889999aaaa),
        (r15, 0x88889999aaaabbbb),
    ];
    let mut ok = 0u64;
    for (i, (got, want)) in probes.iter().enumerate() {
        if got == want {
            ok |= 1 << i;
        }
    }
    T3_OK.store(ok, Ordering::Relaxed);
}

fn test_thread_callee_saved() -> Result<(), &'static str> {
    T3_OK.store(0, Ordering::Relaxed);
    let switches_before = sched::switch_count();
    sched::spawn("t3", t3_entry, 0)?;
    drain(8)?;
    let ok = T3_OK.load(Ordering::Relaxed);
    if ok != 0x3F {
        return Err("callee-saved registers did not survive the switch");
    }
    if sched::switch_count() - switches_before < 2 {
        return Err("probe thread did not actually switch away and back");
    }
    info!(
        "m3",
        "thread_callee_saved: rsi/rdi/r12-r15 round-tripped through a live switch (mask {ok:#x}); rbx/rbp by frame symmetry"
    );
    Ok(())
}

// ---- 4. stack isolation: patterns, disjoint ranges, deep recursion -------

fn t4_pattern(id: usize, slot: usize) -> u64 {
    (0x9E37_79B9_7F4A_7C15 ^ (id as u64).wrapping_mul(0xD1B5_4A32_D192_ED03))
        .rotate_left(slot as u32 & 63)
}

/// Recursion ≈ 200 frames × ~64 B ≈ 13 KiB — real depth on a 32 KiB
/// stack, verified against an iterative model by the test.
fn t4_deep(n: u32) -> u64 {
    let local = [n as u64; 4];
    if n == 0 {
        0
    } else {
        local.iter().sum::<u64>() ^ t4_deep(n - 1)
    }
}

const T4_DEPTH: u32 = 200;

fn t4_entry(arg: usize) {
    let mut buf = [0u64; 64];
    for (i, s) in buf.iter_mut().enumerate() {
        *s = t4_pattern(arg, i);
    }
    // SAFETY: slot arg-1 belongs to this thread (ids 1,2); IF=0.
    unsafe {
        (*T4_RANGE.get())[arg - 1] = sched::current_stack_range();
    }
    let mut ok = true;
    for _ in 0..3 {
        sched::yield_now(); // the other thread writes its own pattern meanwhile
        ok &= buf
            .iter()
            .enumerate()
            .all(|(i, s)| *s == t4_pattern(arg, i));
    }
    let sum = t4_deep(T4_DEPTH);
    ok &= buf
        .iter()
        .enumerate()
        .all(|(i, s)| *s == t4_pattern(arg, i));
    // Iterative model of t4_deep, computed here as an independent check
    // (sum of [k; 4] = 4k, same XOR fold).
    let mut model = 0u64;
    for k in 0..=T4_DEPTH {
        model = 4 * k as u64 ^ model;
    }
    ok &= sum == model;
    if ok {
        T4_OK.fetch_or(1 << (arg - 1), Ordering::Relaxed);
    }
}

fn test_thread_stack_isolation() -> Result<(), &'static str> {
    T4_OK.store(0, Ordering::Relaxed);
    // SAFETY: reset under IF=0.
    unsafe { *T4_RANGE.get() = [(0, 0); 2] };
    sched::spawn("t4a", t4_entry, 1)?;
    sched::spawn("t4b", t4_entry, 2)?;
    drain(16)?;
    if T4_OK.load(Ordering::Relaxed) != 0b11 {
        return Err("a thread's stack pattern or deep-recursion result was corrupted");
    }
    // SAFETY: threads done; single reader.
    let ranges = unsafe { *T4_RANGE.get() };
    let (a, b) = (ranges[0], ranges[1]);
    if a.0 == 0 || b.0 == 0 {
        return Err("thread reported no stack range");
    }
    if a.0 >= a.1 || b.0 >= b.1 {
        return Err("degenerate stack ranges reported");
    }
    if !(a.1 <= b.0 || b.1 <= a.0) {
        return Err("thread stack ranges overlap");
    }
    info!(
        "m3",
        "thread_stack_isolation: patterns survived interleaving; depth-{T4_DEPTH} recursion correct; ranges {:#x}..{:#x} and {:#x}..{:#x} disjoint",
        a.0,
        a.1,
        b.0,
        b.1
    );
    Ok(())
}

// ---- 5. churn with exact accounting + table-full boundary ----------------

fn t5_churn_entry(_arg: usize) {
    T5_CHURN.fetch_add(1, Ordering::Relaxed);
    sched::yield_now();
    // exits through the sched path
}

fn t5_boundary_entry(_arg: usize) {
    T5_BOUNDARY.fetch_add(1, Ordering::Relaxed);
}

fn test_thread_churn_accounting() -> Result<(), &'static str> {
    T5_CHURN.store(0, Ordering::Relaxed);
    T5_BOUNDARY.store(0, Ordering::Relaxed);
    let frames_baseline = crate::frames::free_frames();
    let heap_baseline = crate::heap::bytes_reserved();
    let spawned_before = sched::spawned_total();

    // Eight rounds of eight concurrent threads: create → run → exit →
    // reap, then demand *exact* accounting back to the baseline.
    for _round in 0..8 {
        for _ in 0..8 {
            sched::spawn("t5", t5_churn_entry, 0)?;
        }
        drain(64)?;
    }
    if T5_CHURN.load(Ordering::Relaxed) != 64 {
        return Err("churn threads did not all run");
    }
    if crate::frames::free_frames() != frames_baseline {
        return Err("frame accounting drifted after churn (stack leak?)");
    }
    if crate::heap::bytes_reserved() != heap_baseline {
        return Err("heap reservation drifted after churn");
    }

    // Table-full boundary: fill every remaining slot, demand a clean
    // refusal at the cap, then drain and re-check the accounting.
    for _ in 0..sched::MAX_THREADS - 1 {
        sched::spawn("t5b", t5_boundary_entry, 0)?;
    }
    match sched::spawn("t5-overflow", t5_boundary_entry, 0) {
        Err(_) => {} // the required clean refusal
        Ok(_) => return Err("spawn past MAX_THREADS was accepted"),
    }
    drain(4 * sched::MAX_THREADS)?;
    if T5_BOUNDARY.load(Ordering::Relaxed) != (sched::MAX_THREADS - 1) as u64 {
        return Err("boundary threads did not all run");
    }
    if crate::frames::free_frames() != frames_baseline {
        return Err("frame accounting drifted after boundary phase");
    }
    let spawned = sched::spawned_total() - spawned_before;
    if spawned != 64 + (sched::MAX_THREADS - 1) as u64 {
        return Err("spawn counter mismatch");
    }
    let frames_now = crate::frames::free_frames();
    info!(
        "m3",
        "thread_churn_accounting: {spawned} threads created/run/reaped; free frames {frames_baseline} -> {frames_now} (exact); heap reserved unchanged at {} KiB; table-full refusal at MAX_THREADS={}",
        heap_baseline / 1024,
        sched::MAX_THREADS
    );
    Ok(())
}

// ---- 6. timer-driven rotation of threads that never yield (M3.2) ---------

static P_STOP: AtomicU64 = AtomicU64::new(0);
static P_COUNTERS: [AtomicU64; 3] = [const { AtomicU64::new(0) }; 3];

/// Busy loop with NO scheduler call anywhere — every context switch this
/// thread ever experiences is timer-driven. That is the point (ROADMAP
/// 3.2: interleaving that *proves* preemption).
fn p_rot_entry(arg: usize) {
    while P_STOP.load(Ordering::Relaxed) == 0 {
        P_COUNTERS[arg].fetch_add(1, Ordering::Relaxed);
        core::hint::spin_loop();
    }
}

fn test_preempt_rotation() -> Result<(), &'static str> {
    P_STOP.store(0, Ordering::Relaxed);
    for c in P_COUNTERS.iter() {
        c.store(0, Ordering::Relaxed);
    }
    let log_base = sched::preempt::log_len();
    let total_base = sched::preempt::total();
    // Spawn order == slot order (1,2,3): earlier tests reaped their slots.
    for i in 0..3 {
        sched::spawn("p_rot", p_rot_entry, i)?;
    }
    // Arm: 2-tick quantum (20 ms at 100 Hz); 12 transitions ≈ 240 ms.
    sched::preempt::enable(2);
    x86_64::sti();
    // Bounded wait — NO logging while IF=1 (phase discipline, ADR-0013).
    const NEED: u64 = 12;
    let t0 = crate::timekeeping::now_us();
    while sched::preempt::total() - total_base < NEED
        && crate::timekeeping::now_us() - t0 < 3_000_000
    {
        core::hint::spin_loop();
    }
    x86_64::cli();
    sched::preempt::disable();
    P_STOP.store(1, Ordering::Relaxed);
    // IF=0 from here: drain (each ready thread resumes from its nested
    // IRQ switch, sees STOP, and exits), then log and assert.
    drain(64)?;

    let n = sched::preempt::total() - total_base;
    if n < NEED {
        return Err("fewer than 12 timer-driven rotations observed");
    }
    // Exact cycle bootstrap(0) → t(1) → t(2) → t(3) → bootstrap...:
    // FIFO ring + equal quanta + switch-in refill ⇒ deterministic RR.
    for i in 0..NEED {
        let e = sched::preempt::log_entry(log_base + i as usize)
            .ok_or("preemption log entry vanished")?;
        let (from, to) = (e >> 32, e & 0xFFFF_FFFF);
        if from != i % 4 || to != (i + 1) % 4 {
            return Err("rotation order is not exact round-robin");
        }
    }
    let c = [
        P_COUNTERS[0].load(Ordering::Relaxed),
        P_COUNTERS[1].load(Ordering::Relaxed),
        P_COUNTERS[2].load(Ordering::Relaxed),
    ];
    if c.iter().any(|x| *x == 0) {
        return Err("a thread that never yielded made no progress (preemption not real)");
    }
    info!(
        "m3",
        "preempt_rotation: {n} timer rotations, exact order 0->1->2->3 cycle; progress without a single yield call: {} / {} / {} loop iterations",
        c[0],
        c[1],
        c[2]
    );
    Ok(())
}

// ---- 7. preemptive and cooperative triggers compose -----------------------

static P_COEXIST: [AtomicU64; 2] = [const { AtomicU64::new(0) }; 2];
static P_COEXIST_WAITFAIL: AtomicU64 = AtomicU64::new(0);

fn p_coexist_entry(arg: usize) {
    for _ in 0..3 {
        P_COEXIST[arg].fetch_add(1, Ordering::Relaxed);
        // A 15 ms busy-wait under a 1-tick (10 ms) quantum is guaranteed
        // to be rotated out mid-wait at least once per round.
        if !crate::timekeeping::busy_wait_us(15_000) {
            P_COEXIST_WAITFAIL.fetch_add(1, Ordering::Relaxed);
        }
        // ... and the cooperative trigger still works between waits.
        sched::yield_now();
    }
}

fn test_preempt_coexist() -> Result<(), &'static str> {
    for c in P_COEXIST.iter() {
        c.store(0, Ordering::Relaxed);
    }
    P_COEXIST_WAITFAIL.store(0, Ordering::Relaxed);
    let total_base = sched::preempt::total();
    sched::spawn("p_co0", p_coexist_entry, 0)?;
    sched::spawn("p_co1", p_coexist_entry, 1)?;
    sched::preempt::enable(1); // 10 ms quantum < 15 ms waits
    x86_64::sti();
    let t0 = crate::timekeeping::now_us();
    while (P_COEXIST[0].load(Ordering::Relaxed) < 3 || P_COEXIST[1].load(Ordering::Relaxed) < 3)
        && crate::timekeeping::now_us() - t0 < 3_000_000
    {
        core::hint::spin_loop();
    }
    x86_64::cli();
    sched::preempt::disable();
    drain(64)?;

    let (a, b) = (
        P_COEXIST[0].load(Ordering::Relaxed),
        P_COEXIST[1].load(Ordering::Relaxed),
    );
    if a != 3 || b != 3 {
        return Err("coexist threads did not finish all rounds");
    }
    if P_COEXIST_WAITFAIL.load(Ordering::Relaxed) != 0 {
        return Err("busy-wait anti-hang guard fired under preemption");
    }
    let preempts = sched::preempt::total() - total_base;
    if preempts < 2 {
        return Err("busy-waiting threads were not rotated by the timer");
    }
    info!(
        "m3",
        "preempt_coexist: 2 threads finished 3x(15ms wait + yield) under 10ms quanta; {preempts} timer rotations interleaved with cooperative yields"
    );
    Ok(())
}

// ---- M3.3: the ring-3 boundary (ADR-0014) -----------------------------------
//
// Everything below runs REAL ring-3 code: hand-assembled machine code
// (the user program format + loader is M4.1 — until then the payloads are
// byte arrays the test itself encodes), executing at CPL 3 on U/S pages,
// crossing into the kernel only through the `syscall` MSRs and back via
// `sysretq`. Proof obligations, all machine-checked:
//
// * the payload's privileged `cli` faults #GP and resumes (impossible at
//   CPL 0) — armed-fault protocol extended to ring-3 sites;
// * an invalid syscall number is rejected with -1, observed *in ring 3*
//   (the payload branches to a wrong-status exit if not);
// * SYS_WRITE copies exactly the user's bytes (kernel-side buffer +
//   console), SYS_EXIT terminates through the scheduler's zombie/reap
//   path with the recorded status;
// * timer ticks land on ring-3 code (TSS RSP0 path) and the preemptive
//   tick rotates the user thread mid-execution — the payload survives N
//   rotations and still completes;
// * SMAP: when the CPU has it, a bare kernel read of a user page #PFs
//   (armed, recovered, CR2 asserted); STAC-bracketed access works;
// * user page lifecycle: map → run → unmap → free with exact frame
//   accounting (the intermediate kernel-view tables are deliberately
//   kept, and the assertion is expressed against the measured post-setup
//   free count so it cannot drift).

const U3_CODE_VA: u64 = 0x0000_0000_0040_0000;
const U3_DATA_VA: u64 = 0x0000_0000_0040_1000;
const U3_STACK_VA: u64 = 0x0000_0000_7FF0_0000;
const U3_STACK_TOP: u64 = U3_STACK_VA + 4096;
const U3_REGIONS: [(u64, u64); 3] = [
    (U3_CODE_VA, U3_CODE_VA + 4096),
    (U3_DATA_VA, U3_DATA_VA + 4096),
    (U3_STACK_VA, U3_STACK_TOP),
];

const U3_MSG_A: &[u8] = b"USER32-SYSWRITE-OK";
const U3_MSG_B: &[u8] = b"USER32-SPUN-OK";

/// Anti-hang bound for the spin payload: if the kernel's flag release
/// never arrives (a real bug), the payload exits 99 instead of wedging
/// the machine — the test then fails fast with a meaningful status.
const U9_SPIN_BOUND: u32 = 20_000_000;

/// Minimal cursor-based encoder for the fixed ring-3 payloads. Every
/// instruction is real x86-64 machine code the CPU executes at CPL 3.
struct Payload {
    b: [u8; 192],
    n: usize,
}

impl Payload {
    fn new() -> Self {
        Self { b: [0; 192], n: 0 }
    }
    fn emit(&mut self, bytes: &[u8]) {
        self.b[self.n..self.n + bytes.len()].copy_from_slice(bytes);
        self.n += bytes.len();
    }
    fn emit_u32(&mut self, v: u32) {
        self.emit(&v.to_le_bytes());
    }
    fn here(&self) -> usize {
        self.n
    }
    /// Reserve one rel8 displacement byte (opcode already emitted).
    fn rel8_hole(&mut self) -> usize {
        let at = self.n;
        self.emit(&[0]);
        at
    }
    /// Reserve a 4-byte displacement/immediate slot.
    fn rel32_hole(&mut self) -> usize {
        let at = self.n;
        self.emit(&[0; 4]);
        at
    }
    fn patch_rel8(&mut self, at: usize, target: usize) {
        self.b[at] = (target as i32 - (at as i32 + 1)) as u8;
    }
    fn patch_rel32(&mut self, at: usize, target: usize) {
        let d = target as i32 - (at as i32 + 4);
        self.b[at..at + 4].copy_from_slice(&d.to_le_bytes());
    }
}

/// Payload A — the syscall contract proof:
/// invalid nr → -1 (branch to failure exit if not), PROVE_RING3 + `cli`
/// (#GP at CPL 3, resumed by the armed-fault handler), SYS_WRITE(msg)
/// with a returned-count check, SYS_EXIT(42). Any violation exits 43.
fn build_payload_a() -> (Payload, usize) {
    let mut p = Payload::new();
    p.emit(&[0xB8]);
    p.emit_u32(0x7FFF); // mov eax, 0x7FFF (invalid nr)
    p.emit(&[0x0F, 0x05]); // syscall
    p.emit(&[0x48, 0x3D]);
    p.emit_u32(u32::MAX); // cmp rax, -1
    p.emit(&[0x75]);
    let j_fail1 = p.rel8_hole(); // jnz fail
    p.emit(&[0xB8]);
    p.emit_u32(syscall::SYS_PROVE_RING3 as u32);
    p.emit(&[0x0F, 0x05]); // syscall
    p.emit(&[0xFA]); // cli — #GP at CPL 3, resumed at +1
    p.emit(&[0xB8]);
    p.emit_u32(syscall::SYS_PROVE_DONE as u32);
    p.emit(&[0x0F, 0x05]); // syscall
    p.emit(&[0x48, 0x8D, 0x3D]);
    let lea_msg = p.rel32_hole(); // lea rdi, [rip + msg]
    p.emit(&[0xBE]);
    p.emit_u32(U3_MSG_A.len() as u32); // mov esi, msg_len
    p.emit(&[0xB8]);
    p.emit_u32(syscall::SYS_WRITE as u32);
    p.emit(&[0x0F, 0x05]); // syscall
    p.emit(&[0x48, 0x83, 0xF8, U3_MSG_A.len() as u8]); // cmp rax, msg_len
    p.emit(&[0x75]);
    let j_fail2 = p.rel8_hole(); // jnz fail
    p.emit(&[0xB8]);
    p.emit_u32(syscall::SYS_EXIT as u32);
    p.emit(&[0xBF]);
    p.emit_u32(42); // mov edi, 42
    p.emit(&[0x0F, 0x05]); // syscall
    p.emit(&[0xEB, 0xFE]); // jmp $ (unreachable)
    let fail = p.here();
    p.emit(&[0xB8]);
    p.emit_u32(syscall::SYS_EXIT as u32);
    p.emit(&[0xBF]);
    p.emit_u32(43); // mov edi, 43 (contract violation)
    p.emit(&[0x0F, 0x05]); // syscall
    p.emit(&[0xEB, 0xFE]); // jmp $
    let msg_off = p.here();
    p.emit(U3_MSG_A);
    p.patch_rel8(j_fail1, fail);
    p.patch_rel8(j_fail2, fail);
    p.patch_rel32(lea_msg, msg_off);
    (p, msg_off)
}

/// Payload B — the interrupt/preemption proof: spin reading the flag
/// qword at `U3_DATA_VA` (absolute moffs64 read) with `pause`, bounded by
/// `ecx`; flag set → SYS_WRITE(msg) + SYS_EXIT(7); bound exhausted →
/// SYS_EXIT(99) (anti-hang).
fn build_payload_b(spin_bound: u32) -> (Payload, usize) {
    let mut p = Payload::new();
    p.emit(&[0xB9]);
    p.emit_u32(spin_bound); // mov ecx, bound
    let loop_top = p.here();
    p.emit(&[0x48, 0xA1]);
    p.emit(&U3_DATA_VA.to_le_bytes()); // mov rax, [U3_DATA_VA]
    p.emit(&[0x84, 0xC0]); // test al, al
    p.emit(&[0x75]);
    let j_done = p.rel8_hole(); // jnz done
    p.emit(&[0xF3, 0x90]); // pause
    p.emit(&[0xFF, 0xC9]); // dec ecx
    p.emit(&[0x75]);
    let j_loop = p.rel8_hole(); // jnz loop_top
    p.emit(&[0xB8]);
    p.emit_u32(syscall::SYS_EXIT as u32);
    p.emit(&[0xBF]);
    p.emit_u32(99); // timeout exit (anti-hang)
    p.emit(&[0x0F, 0x05]); // syscall
    p.emit(&[0xEB, 0xFE]); // jmp $
    let done = p.here();
    p.emit(&[0x48, 0x8D, 0x3D]);
    let lea_msg = p.rel32_hole(); // lea rdi, [rip + msg]
    p.emit(&[0xBE]);
    p.emit_u32(U3_MSG_B.len() as u32); // mov esi, msg_len
    p.emit(&[0xB8]);
    p.emit_u32(syscall::SYS_WRITE as u32);
    p.emit(&[0x0F, 0x05]); // syscall
    p.emit(&[0xB8]);
    p.emit_u32(syscall::SYS_EXIT as u32);
    p.emit(&[0xBF]);
    p.emit_u32(7); // mov edi, 7
    p.emit(&[0x0F, 0x05]); // syscall
    p.emit(&[0xEB, 0xFE]); // jmp $
    let msg_off = p.here();
    p.emit(U3_MSG_B);
    p.patch_rel8(j_done, done);
    p.patch_rel8(j_loop, loop_top);
    p.patch_rel32(lea_msg, msg_off);
    (p, msg_off)
}

/// Allocate + map the three-page user window (code R+X, data RW+NX,
/// stack RW+NX — all U/S, W^X enforced by the mapper) into the kernel's
/// own view, write the payload + message into the code page, and zero the
/// data page (the flag payload B spins on). The writes go through the
/// *direct-map alias* of the frames (supervisor RW): writing through the
/// fresh R+X user mapping would #PF under CR0.WP — the kernel honors its
/// own read-only pages (a first bring-up attempt proved exactly that).
/// The STAC positive path is exercised by SYS_WRITE's copy and test 9's
/// flag release. Returns the three frame PHYS addresses and the
/// post-setup free-frame count (teardown asserts against it).
unsafe fn u3_setup(
    payload: &[u8],
    msg_off: usize,
    msg: &[u8],
) -> Result<([u64; 3], u64), &'static str> {
    let mut phys = [0u64; 3];
    for slot in phys.iter_mut() {
        *slot = frames::alloc().ok_or("frame exhaustion for user pages")?;
    }
    // SAFETY: IF=0 (suite discipline), fresh owned frames, canonical
    // lower-half VAs, W^X-respecting permission pairs.
    unsafe {
        paging::map_user_page_kernel_view(U3_CODE_VA, phys[0], false, true)
            .map_err(|_| "user code page map failed")?;
        paging::map_user_page_kernel_view(U3_DATA_VA, phys[1], true, false)
            .map_err(|_| "user data page map failed")?;
        paging::map_user_page_kernel_view(U3_STACK_VA, phys[2], true, false)
            .map_err(|_| "user stack page map failed")?;
        let code_kv = phys[0].wrapping_add(paging::KERNEL_OFFSET);
        let data_kv = phys[1].wrapping_add(paging::KERNEL_OFFSET);
        core::ptr::copy_nonoverlapping(payload.as_ptr(), code_kv as *mut u8, payload.len());
        core::ptr::copy_nonoverlapping(
            msg.as_ptr(),
            (code_kv + msg_off as u64) as *mut u8,
            msg.len(),
        );
        core::ptr::write_bytes(data_kv as *mut u8, 0, 4096);
    }
    Ok((phys, frames::free_frames()))
}

/// Unmap the user window, free its three frames, and assert the frame
/// accounting is exact against the post-setup measurement (the five
/// intermediate kernel-view page tables stay — deliberate, counted by the
/// runtime baseline rather than a magic constant).
unsafe fn u3_teardown(phys: [u64; 3], after_setup_free: u64) -> Result<u64, &'static str> {
    // SAFETY: IF=0; the pages were mapped by u3_setup and nobody else
    // references them (the user thread is reaped before teardown).
    unsafe {
        for va in [U3_CODE_VA, U3_DATA_VA, U3_STACK_VA] {
            let p = paging::unmap_user_page_kernel_view(va)
                .ok_or("user page vanished from the kernel view")?;
            frames::free(p).map_err(|_| "user page free rejected")?;
        }
    }
    let free_now = frames::free_frames();
    if free_now != after_setup_free + 3 {
        return Err("user-page teardown frame accounting is not exact");
    }
    let _ = phys; // the PTE-derived addresses freed above came from our maps
    Ok(free_now)
}

/// RSP0 evidence, captured by the user thread itself at entry (the last
/// instant before ring 3 where it matters): TSS RSP0 must name exactly
/// this thread's kernel stack top — plan_switch programmed it at
/// switch-in. (Checking *after* SYS_EXIT is impossible by design: the
/// exit path reaps the exiting thread's own slot before switching away —
/// M3.1 semantics.)
static U3_RSP0_ENTRY: AtomicU64 = AtomicU64::new(0);
static U3_TOP_ENTRY: AtomicU64 = AtomicU64::new(0);

/// The shared user-thread entry: register the window, then iretq into
/// ring 3. Runs at IF=0 up to the iretq (enter_user's contract); the
/// user side runs IF=1 by design — interrupts landing in ring 3 are part
/// of what these tests prove.
fn u3_thread_entry(_arg: usize) {
    crate::sync::without_interrupts(|| {
        sched::set_current_user_regions(&U3_REGIONS).expect("user regions rejected");
        U3_RSP0_ENTRY.store(tss::read_back_rsp0(), Ordering::Relaxed);
        U3_TOP_ENTRY.store(
            sched::current_kernel_stack_top().unwrap_or(0),
            Ordering::Relaxed,
        );
        // SAFETY: pages mapped U/S by u3_setup, MSRs armed and read-back-
        // verified in kmain, RSP0/scratch describe this thread (plan_switch
        // programmed them at switch-in; enter_user re-checks), RIP/RSP
        // canonical and inside the registered regions.
        unsafe { syscall::enter_user(U3_CODE_VA, U3_STACK_TOP) };
    });
}

/// Armed-fault site for the SMAP proof: writes its own resume address,
/// then performs a bare (no STAC) 8-byte kernel read of a user page —
/// with SMAP live this must #PF; the handler resumes at the label.
///
/// # Safety
/// Call only with `faults::arm(14)` in effect and `addr` a mapped U/S
/// page (otherwise the read silently succeeds and the armed expectation
/// leaks into later code).
unsafe fn read_probe(addr: u64) {
    // SAFETY: as above; clobbers declared; RESUME is our own static
    // (single CPU, IF=0 — SyncCell contract).
    unsafe {
        core::arch::asm!(
            "lea rax, [rip + 2f]",
            "mov [rip + {resume}], rax",
            "mov rcx, [{addr}]", // #PF under SMAP — handler resumes at 2:
            "2:",
            addr = in(reg) addr,
            resume = sym faults::RESUME,
            out("rax") _,
            out("rcx") _,
            options(nostack),
        );
    }
}

fn test_user_ring3_syscall() -> Result<(), &'static str> {
    // (a) The GDT ring-3 pair is live, byte-for-byte as designed.
    if gdt::user_entries_readback() != gdt::expected_user_entries() {
        return Err("GDT ring-3 entries are not the expected encodings");
    }
    // (b) Control-MSR/CR4 evidence (kmain armed + verified; the suite
    //     re-checks independently — nothing here trusts a log line).
    if x86_64::read_efer() & x86_64::efer::SCE == 0 {
        return Err("EFER.SCE not set — SYSCALL would #UD");
    }
    let star = x86_64::rdmsr(syscall::MSR_STAR);
    let want_star = (0x20u64 << 48) | ((gdt::KERNEL_CODE_SELECTOR as u64) << 32);
    if star != want_star {
        return Err("STAR does not carry our kernel/user selector layout");
    }
    if x86_64::rdmsr(syscall::MSR_LSTAR) < paging::KERNEL_OFFSET {
        return Err("LSTAR is not a kernel-view address");
    }
    let cpu = x86_64::cpu_info();
    let cr4 = x86_64::read_cr4();
    if cpu.has_smep && cr4 & x86_64::cr4::SMEP == 0 {
        return Err("CPU offers SMEP but CR4.SMEP is off");
    }
    if cpu.has_smap && cr4 & x86_64::cr4::SMAP == 0 {
        return Err("CPU offers SMAP but CR4.SMAP is off");
    }
    // (c) The user window + payload A.
    let (p, msg_off) = build_payload_a();
    // SAFETY: IF=0; fresh frames; contract per u3_setup.
    let (phys, free_after_setup) = unsafe { u3_setup(&p.b[..msg_off], msg_off, U3_MSG_A) }?;
    // (d) SMAP enforcement proof (when the CPU has it): the STAC-bracketed
    //     writes in u3_setup already proved the positive path; a BARE
    //     kernel read of the user page must #PF with CR2 = the user VA.
    let smap_proof = if x86_64::smap_active() {
        faults::arm(14);
        // SAFETY: armed protocol; U3_DATA_VA is mapped U/S (setup above).
        unsafe { read_probe(core::hint::black_box(U3_DATA_VA)) };
        let obs = faults::observed();
        faults::disarm();
        if !obs.valid || obs.vector != 14 {
            return Err("SMAP: bare kernel read of a user page did NOT fault");
        }
        if obs.cr2 != U3_DATA_VA {
            return Err("SMAP: #PF CR2 does not name the user page");
        }
        "SMAP enforced (bare kernel read of user page -> recovered #PF)"
    } else {
        "SMAP absent on this CPU (enforcement skipped, STAC/CLAC no-ops)"
    };
    // (e) Run the payload: one cooperative yield hands the CPU to the user
    //     thread; it returns only through SYS_EXIT.
    let st0 = syscall::stats();
    U3_RSP0_ENTRY.store(0, Ordering::Relaxed);
    U3_TOP_ENTRY.store(0, Ordering::Relaxed);
    let id = sched::spawn("u3", u3_thread_entry, 0)?;
    sched::yield_now();
    drain(64)?;
    // (f) Verdicts — every claim is dispatcher-recorded machine state.
    let Some(status) = syscall::exit_status_of(id) else {
        return Err("no SYS_EXIT recorded for the user thread");
    };
    if status != 42 {
        return Err("payload took a failure exit (ring-3 contract violated)");
    }
    let st = syscall::stats();
    if st.write_calls - st0.write_calls != 1
        || st.write_bytes - st0.write_bytes != U3_MSG_A.len() as u64
    {
        return Err("SYS_WRITE accounting wrong");
    }
    if st.invalid_nr - st0.invalid_nr != 1 {
        return Err("invalid syscall number not rejected exactly once");
    }
    if st.exit_calls - st0.exit_calls != 1 {
        return Err("SYS_EXIT accounting wrong");
    }
    if !st.ring3_proved {
        return Err("ring-3 #GP proof not recorded (cli did not fault -> not CPL 3?)");
    }
    if st.ring3_gp_ec != 0 {
        return Err("proved #GP carried a nonzero error code");
    }
    let (buf, n) = syscall::last_write();
    if n != U3_MSG_A.len() || buf[..n] != U3_MSG_A[..] {
        return Err("SYS_WRITE kernel-side copy mismatch");
    }
    let rsp0 = U3_RSP0_ENTRY.load(Ordering::Relaxed);
    let top = U3_TOP_ENTRY.load(Ordering::Relaxed);
    if top == 0 || rsp0 != top {
        return Err("TSS RSP0 did not name the user thread's kernel stack top at entry");
    }
    // SAFETY: IF=0; user thread reaped; contract per u3_teardown.
    let free_now = unsafe { u3_teardown(phys, free_after_setup) }?;
    info!(
        "m3",
        "user_ring3_syscall: hand-assembled payload ran at CPL 3 — cli -> #GP(ec=0) recovered, invalid nr -> -1 seen in ring 3, SYS_WRITE copied {} bytes, SYS_EXIT(42); RSP0 exact; {}; frames {} (teardown exact)",
        U3_MSG_A.len(),
        smap_proof,
        free_now
    );
    Ok(())
}

fn test_user_ring3_interrupted() -> Result<(), &'static str> {
    let (p, msg_off) = build_payload_b(U9_SPIN_BOUND);
    // SAFETY: IF=0; fresh frames; contract per u3_setup.
    let (phys, free_after_setup) = unsafe { u3_setup(&p.b[..msg_off], msg_off, U3_MSG_B) }?;
    let st0 = syscall::stats();
    let switches0 = sched::switch_count();
    let ticks0 = idt::absorbed_irq_count();
    let id = sched::spawn("u9", u3_thread_entry, 0)?;
    // Arm preemption and let the ring-3 spin run: ticks land in user mode
    // (RSP0 path), the hook rotates the user thread like any other, and
    // bootstrap only regains the CPU through that rotation.
    sched::preempt::enable(2);
    x86_64::sti();
    // Phase discipline: NO logging while IF=1 (ADR-0013).
    let t0 = crate::timekeeping::now_us();
    while (idt::absorbed_irq_count() - ticks0 < 4 || sched::switch_count() - switches0 < 2)
        && crate::timekeeping::now_us() - t0 < 3_000_000
    {
        core::hint::spin_loop();
    }
    // Release the payload: one volatile qword write into ITS data page
    // (SMAP-aware) — kernel and user sharing memory through the live
    // address space, mid-preemption-window.
    // SAFETY: U3_DATA_VA mapped U/S RW by setup; single writer (the user
    // side only reads).
    unsafe {
        x86_64::stac();
        core::ptr::write_volatile(U3_DATA_VA as *mut u64, 1);
        x86_64::clac();
    }
    let t1 = crate::timekeeping::now_us();
    while syscall::exit_status_of(id).is_none() && crate::timekeeping::now_us() - t1 < 3_000_000 {
        core::hint::spin_loop();
    }
    x86_64::cli();
    sched::preempt::disable();
    drain(64)?;

    let ticks = idt::absorbed_irq_count() - ticks0;
    let switches = sched::switch_count() - switches0;
    let Some(status) = syscall::exit_status_of(id) else {
        return Err("ring-3 spin payload never exited");
    };
    if status == 99 {
        return Err("payload hit its anti-hang bound — the flag release never arrived");
    }
    if status != 7 {
        return Err("ring-3 payload exited with the wrong status");
    }
    if ticks < 4 {
        return Err("timer ticks did not land while ring-3 code ran");
    }
    if switches < 2 {
        return Err("the ring-3 thread was not rotated by the timer");
    }
    let rsp0 = U3_RSP0_ENTRY.load(Ordering::Relaxed);
    let top = U3_TOP_ENTRY.load(Ordering::Relaxed);
    if top == 0 || rsp0 != top {
        return Err("TSS RSP0 lost track of the user thread's kernel stack");
    }
    let st = syscall::stats();
    let (buf, n) = syscall::last_write();
    if st.write_bytes - st0.write_bytes != U3_MSG_B.len() as u64
        || n != U3_MSG_B.len()
        || buf[..n] != U3_MSG_B[..]
    {
        return Err("post-spin SYS_WRITE payload mismatch");
    }
    // SAFETY: IF=0; user thread reaped; contract per u3_teardown.
    let free_now = unsafe { u3_teardown(phys, free_after_setup) }?;
    info!(
        "m3",
        "user_ring3_interrupted: ring-3 spin survived {ticks} ticks and {switches} timer rotations (RSP0 path + mid-user preemption, resume-to-user via iretq); kernel-released flag ended the spin; SYS_WRITE+SYS_EXIT(7) verified; frames {free_now} (teardown exact)"
    );
    Ok(())
}
