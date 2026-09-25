//! Milestone 3 test suite — kernel threads, the context switch (M3.1,
//! ADR-0012), and timer-driven preemption (M3.2, ADR-0013). Runs in
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

use crate::arch::x86_64;
use crate::log::{log_error as error, log_info as info, write_marker};
use crate::sched;
use crate::sync::SyncCell;
use core::sync::atomic::{AtomicU64, Ordering};

pub fn run_suite() -> bool {
    let checks: [(&str, fn() -> Result<(), &'static str>); 7] = [
        ("thread_spawn_run", test_thread_spawn_run),
        ("thread_rr_interleave", test_thread_rr_interleave),
        ("thread_callee_saved", test_thread_callee_saved),
        ("thread_stack_isolation", test_thread_stack_isolation),
        ("thread_churn_accounting", test_thread_churn_accounting),
        ("preempt_rotation", test_preempt_rotation),
        ("preempt_coexist", test_preempt_coexist),
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
