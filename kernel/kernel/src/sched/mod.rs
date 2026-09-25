//! Kernel threads and the scheduler core (M3.1/M3.2, ADR-0012/ADR-0013).
//!
//! Model: fixed table of `MAX_THREADS` slots (stable indices — the ready
//! ring stores indices and a compaction would invalidate them), one
//! round-robin ready ring **per CPU** (`MAX_CPUS` structures; execution
//! is single-core until SMP lands — `this_cpu()` is 0 by contract), and
//! two switch triggers sharing one decision routine:
//!
//! * cooperative `yield_now()` (M3.1), and
//! * timer-driven preemption from the vector-32 tick hook
//!   ([`preempt`], M3.2) — the tick calls the *same* `plan_switch` +
//!   `arena_context_switch` from interrupt context; the interrupted
//!   thread's full state is its hardware iretq frame + stub saves + our
//!   standard switch frame, all on its own stack (ADR-0013).
//!
//! Discipline (all machine-checked by the M3 suite):
//! - All scheduler state is mutated under IF=0: cooperative sections
//!   enter `without_interrupts` (ADR-0010) themselves, and the interrupt
//!   gate hands the tick hook IF=0 for free — a tick can therefore never
//!   observe a half-finished decision. **No borrow crosses the context
//!   switch**: the decision phase yields a plan of raw values (save-slot
//!   pointer, restore RSP), the borrow ends, and only then does the
//!   assembly switch run.
//! - Every kernel thread stack is 32 KiB of contiguous frames with a
//!   canary in its bottom qword, checked whenever the thread switches
//!   away and again at reap; a corrupted canary halts the machine with
//!   diagnostics (an overflowed kernel stack is never safe to continue).
//! - A thread cannot free the stack it runs on: `exit` marks it `Zombie`
//!   and the *next* scheduler entry reaps it (`free_contiguous` + slot
//!   cleared). The churn test pins the accounting to exact baselines.
//! - The bootstrap thread is `kmain` itself (slot 0, id 0); its boot
//!   stack is a reserved firmware region the scheduler does not own
//!   (`stack_frames = 0` ⇒ no canary, nothing to free).

pub mod preempt;

use crate::arch::x86_64::context;
use crate::arch::x86_64::paging::KERNEL_OFFSET;
use crate::log::log_error as error;
use crate::sync::{SyncCell, without_interrupts};
use core::sync::atomic::{AtomicU64, Ordering};

/// Scheduler slot cap (M3.1): thread structs are small and static, the
/// ready rings are sized to match, and `spawn` fails cleanly at the cap —
/// a boundary the churn test exercises on purpose.
pub const MAX_THREADS: usize = 64;

/// Per-CPU scheduler structures exist for `MAX_CPUS` CPUs; execution is
/// BSP-only until SMP lands (M5) — see [`this_cpu`].
pub const MAX_CPUS: usize = 4;

/// Kernel stack per thread: 8 contiguous frames = 32 KiB (ADR-0012).
pub const THREAD_STACK_FRAMES: usize = 8;
const THREAD_STACK_BYTES: u64 = (THREAD_STACK_FRAMES * 4096) as u64;

/// Bottom-of-stack canary ("ARENASTK"), checked on every switch-away.
const STACK_CANARY: u64 = 0x4152_454E_4153_544B;

/// Per-thread user regions the syscall dispatcher validates against
/// (code / data / stack / spare — ADR-0014).
pub const USER_REGIONS_MAX: usize = 4;

/// M3.1 stacks come from the direct map's first 2 GiB (ADR-0008); a frame
/// beyond that has no kernel-view alias yet, so `spawn` refuses it rather
/// than hand back an unmapped stack. (512 MiB reference VM: all
/// conventional memory is far below the bound.)
const DIRECT_MAP_LIMIT: u64 = 2 * 1024 * 1024 * 1024;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum State {
    Ready,
    Running,
    /// Exited; awaiting reap by the next scheduler entry.
    Zombie,
}

#[derive(Clone, Copy)]
pub struct KThread {
    pub id: u64,
    pub name: &'static str,
    pub state: State,
    entry: fn(usize),
    arg: usize,
    /// Stack base *physical* address (0 = not owned: the bootstrap thread).
    stack_base: u64,
    stack_frames: usize,
    /// CR3 (PML4 PHYS) of this thread's address space (M3.3, ADR-0014).
    /// Kernel threads (including the bootstrap) carry the kernel-view
    /// PML4; process threads carry their process's own PML4 (kernel half
    /// cloned, user half private). plan_switch installs it at every
    /// switch-in when it differs from the live CR3 — so switching back
    /// to any kernel thread restores the kernel view, which is what
    /// makes proc::destroy's "not the live CR3" guard satisfiable.
    /// Never 0 (write_cr3(0) would be fatal; plan_switch keeps a
    /// belt-and-braces zero check).
    cr3: u64,
    /// Registered user-memory regions ((lo, hi) page-granular pairs;
    /// (0,0) = slot unused) — the syscall dispatcher validates every
    /// user pointer against exactly these (ADR-0014). Kernel-only
    /// threads leave them zeroed, so any user-pointer syscall from them
    /// is rejected.
    regions: [(u64, u64); USER_REGIONS_MAX],
}

/// Fixed-capacity FIFO of slot indices (one CPU's round-robin ready ring).
#[derive(Clone, Copy)]
struct Ring {
    q: [usize; MAX_THREADS],
    head: usize,
    len: usize,
}

impl Ring {
    const fn new() -> Self {
        Self {
            q: [0; MAX_THREADS],
            head: 0,
            len: 0,
        }
    }
    fn push(&mut self, idx: usize) -> bool {
        if self.len == MAX_THREADS {
            return false;
        }
        self.q[(self.head + self.len) % MAX_THREADS] = idx;
        self.len += 1;
        true
    }
    fn pop(&mut self) -> Option<usize> {
        if self.len == 0 {
            return None;
        }
        let idx = self.q[self.head];
        self.head = (self.head + 1) % MAX_THREADS;
        self.len -= 1;
        Some(idx)
    }
}

/// Per-CPU scheduler state (ADR-0013): who is running here, what is
/// ready to run here, and this CPU's preemption quantum accounting.
#[derive(Clone, Copy)]
pub(super) struct CpuSched {
    current: usize,
    ready: Ring,
    /// Quantum length in PIT ticks; 0 = preemption disarmed on this CPU.
    slice: u32,
    /// Ticks left for `current`.
    remaining: u32,
}

impl CpuSched {
    const fn new() -> Self {
        Self {
            current: 0,
            ready: Ring::new(),
            slice: 0,
            remaining: 0,
        }
    }
}

static THREADS: SyncCell<[Option<KThread>; MAX_THREADS]> = SyncCell::new([None; MAX_THREADS]);
/// Saved RSP per slot (the switch's save-slot target; raw access by
/// design — a pointer into here crosses the switch, a borrow must not).
static CTX: SyncCell<[u64; MAX_THREADS]> = SyncCell::new([0; MAX_THREADS]);
static CPUS: SyncCell<[CpuSched; MAX_CPUS]> = SyncCell::new([CpuSched::new(); MAX_CPUS]);
static NEXT_ID: SyncCell<u64> = SyncCell::new(1);
static SWITCH_COUNT: AtomicU64 = AtomicU64::new(0);
static SPAWNED_TOTAL: AtomicU64 = AtomicU64::new(0);

/// The CPU this code runs on. Single-core execution until SMP (M5):
/// only the BSP exists, and every scheduler structure is already shaped
/// per-CPU so that "start the APs" is a data change, not a redesign.
pub fn this_cpu() -> usize {
    0
}

/// Raw plan produced by a decision phase: everything the assembly switch
/// needs, with every borrow already ended (ADR-0012 discipline).
#[derive(Clone, Copy)]
pub(super) struct Plan {
    pub save: *mut u64,
    pub restore: u64,
}

/// Placeholder entry for the bootstrap thread — never invoked: slot 0 is
/// already running when the scheduler is born and has no trampoline frame.
fn bootstrap_entry_never_runs(_: usize) {}

/// Register the bootstrap thread (kmain, slot 0). Called once from
/// `kmain` before any thread work; a second call is a bug and fails.
pub fn init() -> Result<(), &'static str> {
    without_interrupts(|| {
        // SAFETY: single writer under IF=0 (boot-contract discipline).
        let threads = unsafe { &mut *THREADS.get() };
        if threads[0].is_some() {
            return Err("scheduler already initialized");
        }
        threads[0] = Some(KThread {
            id: 0,
            name: "kmain",
            state: State::Running,
            entry: bootstrap_entry_never_runs,
            arg: 0,
            stack_base: 0,
            stack_frames: 0,
            // The bootstrap thread owns the kernel view: every switch
            // back to it restores the canonical CR3 (M3.3b).
            cr3: crate::arch::x86_64::paging::kernel_cr3_phys(),
            regions: [(0, 0); USER_REGIONS_MAX],
        });
        // SAFETY: same discipline; fresh scheduler, known values.
        unsafe {
            *CPUS.get() = [CpuSched::new(); MAX_CPUS];
            *NEXT_ID.get() = 1;
        }
        Ok(())
    })
}

/// Create a thread and enqueue it Ready on this CPU. It runs `entry(arg)`
/// on its own 32 KiB stack the next time the scheduler picks it, then
/// exits and is reaped automatically. Returns the thread id.
pub fn spawn(name: &'static str, entry: fn(usize), arg: usize) -> Result<u64, &'static str> {
    spawn_inner(
        name,
        entry,
        arg,
        crate::arch::x86_64::paging::kernel_cr3_phys(),
    )
}

/// Like [`spawn`], but the thread runs in the address space rooted at
/// `cr3_phys` (a process PML4 from [`crate::proc::create`], whose kernel
/// half is cloned and user half private). plan_switch installs it as CR3
/// at every switch-in to this thread, so its ring-3 code sees the
/// process's user half while syscalls/interrupts keep running on the one
/// shared kernel half — no CR3 switch at the privilege boundary.
pub fn spawn_with_cr3(
    name: &'static str,
    entry: fn(usize),
    arg: usize,
    cr3_phys: u64,
) -> Result<u64, &'static str> {
    if cr3_phys == 0 || cr3_phys % crate::arch::x86_64::paging::PAGE != 0 {
        return Err("spawn_with_cr3: not a valid PML4 root");
    }
    spawn_inner(name, entry, arg, cr3_phys)
}

fn spawn_inner(
    name: &'static str,
    entry: fn(usize),
    arg: usize,
    cr3: u64,
) -> Result<u64, &'static str> {
    without_interrupts(|| {
        reap();
        // SAFETY: single writer under IF=0.
        let (idx, id) = unsafe {
            let threads = &mut *THREADS.get();
            let Some(idx) = threads.iter().position(Option::is_none) else {
                return Err("thread table full (MAX_THREADS)");
            };
            let id = *NEXT_ID.get();
            *NEXT_ID.get() = id + 1;
            (idx, id)
        };
        let Some(phys) = crate::frames::alloc_contiguous(THREAD_STACK_FRAMES) else {
            return Err("frame exhaustion: no contiguous run for a thread stack");
        };
        if phys + THREAD_STACK_BYTES > DIRECT_MAP_LIMIT {
            // Unmappable in the kernel view (3.1 bound) — give it back.
            let _ = crate::frames::free_contiguous(phys, THREAD_STACK_FRAMES);
            return Err("stack frame beyond the 2 GiB direct map (M3.1 bound)");
        }
        let base_va = phys.wrapping_add(KERNEL_OFFSET);
        // SAFETY: fresh exclusive frames, mapped RW in the kernel view.
        let rsp0 = unsafe {
            *(base_va as *mut u64) = STACK_CANARY;
            context::new_thread_stack(base_va + THREAD_STACK_BYTES)
        };
        // SAFETY: slot `idx` is free (found above) and stays ours (IF=0).
        unsafe {
            let threads = &mut *THREADS.get();
            threads[idx] = Some(KThread {
                id,
                name,
                state: State::Ready,
                entry,
                arg,
                stack_base: phys,
                stack_frames: THREAD_STACK_FRAMES,
                cr3,
                regions: [(0, 0); USER_REGIONS_MAX],
            });
            (*CTX.get())[idx] = rsp0;
            let cpu = &mut (*CPUS.get())[this_cpu()];
            if !cpu.ready.push(idx) {
                // Unreachable: ready len ≤ MAX_THREADS-1 while a slot was
                // free — but leave no leak if the invariant is ever broken.
                threads[idx] = None;
                let _ = crate::frames::free_contiguous(phys, THREAD_STACK_FRAMES);
                return Err("ready queue overflow");
            }
        }
        SPAWNED_TOTAL.fetch_add(1, Ordering::Relaxed);
        if cr3 != crate::arch::x86_64::paging::kernel_cr3_phys() {
            crate::log::log_info!(
                "sched",
                "spawn(proc): id={} slot={} entry={:x} cr3={:x}",
                id,
                idx,
                entry as usize,
                cr3
            );
        }
        Ok(id)
    })
}

/// Voluntarily give up the CPU: run the round-robin decision and, if
/// another thread is ready, switch to it. Returns when this thread is
/// resumed (or immediately if the ready ring is empty — spinning on an
/// empty ring would starve the very thread asking others to run).
pub fn yield_now() {
    without_interrupts(|| {
        let Some(plan) = plan_switch(true) else {
            return;
        };
        SWITCH_COUNT.fetch_add(1, Ordering::Relaxed);
        // SAFETY: plan holds raw values; every borrow ended in
        // plan_switch; both stacks are mapped RW; IF=0 (we are inside
        // without_interrupts), so no interrupt sees the half-switched
        // state. This call resumes the incoming thread and suspends us
        // until our own frame is switched back to.
        unsafe { context::switch_context(plan.save, plan.restore) };
    });
}

/// Trampoline target (ADR-0012): runs the current thread's entry, then
/// exits. Never returns — `exit_now` switches away for good.
pub extern "C" fn thread_main() -> ! {
    // Copy the (immutable while Running) entry info out; no borrow stays
    // live across the entry call, which may itself yield.
    let (entry, arg) = without_interrupts(|| {
        // SAFETY: single reader under IF=0; CURRENT is a Running slot.
        unsafe {
            let cur = (*CPUS.get())[this_cpu()].current;
            let t = (*THREADS.get())[cur].expect("current thread vanished");
            // M3.3b bring-up instrumentation: a process thread's first
            // run logs the pointer it is about to call (IF=0 here).
            if t.cr3 != crate::arch::x86_64::paging::kernel_cr3_phys() {
                crate::log::log_info!(
                    "sched",
                    "thread_main(proc): id={} entry={:x} arg={} cr3={:x}",
                    t.id,
                    t.entry as usize,
                    t.arg,
                    t.cr3
                );
            }
            (t.entry, t.arg)
        }
    });
    entry(arg);
    exit_now()
}

/// The current thread is done: zombie it, switch to the next ready
/// thread, and let a later scheduler entry reap the stack.
fn exit_now() -> ! {
    let plan = without_interrupts(|| {
        check_canary_current();
        // SAFETY: single writer under IF=0.
        unsafe {
            let cur = (*CPUS.get())[this_cpu()].current;
            // as_mut() — NOT expect(): on a Copy item, place.expect()
            // returns a temporary and the assignment would silently be
            // discarded (proved standalone; caught by the drain tests).
            (*THREADS.get())[cur]
                .as_mut()
                .expect("current thread vanished")
                .state = State::Zombie;
        }
        plan_switch(false)
    });
    let Some(plan) = plan else {
        // Impossible while the bootstrap thread exists (it is never a
        // zombie and is either Running or Ready) — a bug, not a state to
        // limp on in.
        error!("sched", "thread exited with an empty ready queue");
        crate::halt::halt_machine("scheduler: exited with empty ready queue");
    };
    SWITCH_COUNT.fetch_add(1, Ordering::Relaxed);
    // SAFETY: as in yield_now. The exiting thread's slot stays allocated
    // (Zombie) until reaped, so its save slot remains valid — but the
    // scheduler never enqueues zombies, so this switch must not return.
    unsafe { context::switch_context(plan.save, plan.restore) };
    // A resumed zombie means the scheduler violated its own invariant.
    error!("sched", "zombie thread was resumed after exit");
    crate::halt::halt_machine("scheduler: zombie resumed");
}

/// Decision phase (shared by yield, exit, and the preemptive tick —
/// ADR-0013): reap zombies, canary-check the outgoing thread, pick the
/// next ready thread, refill its quantum, update states/ring, and return
/// the raw switch plan. `enqueue_current` = yield/preempt semantics
/// (current goes to the back of its CPU's ring); `false` = exit
/// semantics (current stays Zombie).
///
/// Every borrow ends before this returns — the plan is raw by design.
pub(super) fn plan_switch(enqueue_current: bool) -> Option<Plan> {
    reap();
    if enqueue_current {
        check_canary_current();
    }
    // SAFETY: single writer under IF=0; all reads/writes complete here.
    unsafe {
        let cpu = &mut (*CPUS.get())[this_cpu()];
        let next = cpu.ready.pop()?;
        let cur = cpu.current;
        let threads = &mut *THREADS.get();
        if enqueue_current {
            // as_mut(), not expect() — expect() on a Copy place assigns
            // to a discarded temporary (see exit_now).
            threads[cur]
                .as_mut()
                .expect("current thread vanished")
                .state = State::Ready;
            let ok = cpu.ready.push(cur);
            debug_assert!(ok, "ready ring overflow with a free slot");
        }
        threads[next].as_mut().expect("ready slot vanished").state = State::Running;
        cpu.current = next;
        // Fresh quantum for the incoming thread (strict RR; also resets
        // the countdown when a yield found the ring empty and no switch
        // happens — the current thread simply gets a new slice).
        cpu.remaining = cpu.slice;
        // M3.3 (ADR-0014): the ring-3→ring-0 entry stack pair (TSS RSP0 +
        // the syscall stub's GS scratch) and the address space always
        // describe the thread about to run. Written together, here, so
        // they can never disagree; safe before the actual switch because
        // the switch itself only touches kernel-half memory (thread
        // stacks live in the direct map; the save slots live in the
        // image — both present in every address space we build).
        let nt = threads[next].expect("ready slot vanished"); // Copy
        if nt.stack_frames > 0 {
            let top = nt.stack_base.wrapping_add(KERNEL_OFFSET) + THREAD_STACK_BYTES;
            crate::arch::x86_64::tss::set_rsp0(top);
            crate::arch::x86_64::syscall::set_cpu_kernel_stack(top);
        }
        if nt.cr3 != 0 {
            let live = crate::arch::x86_64::read_cr3();
            if live != nt.cr3 {
                crate::arch::x86_64::write_cr3(nt.cr3);
            }
        }
        Some(Plan {
            save: (CTX.get() as *mut u64).add(cur),
            restore: (*CTX.get())[next],
        })
    }
}

/// Free exited threads: final canary check, stack frames back to the
/// allocator, slot cleared. Called at the top of every decision phase —
/// a thread cannot free the stack it is running on (ADR-0012).
fn reap() {
    // SAFETY: single writer under IF=0.
    unsafe {
        let threads = &mut *THREADS.get();
        for slot in threads.iter_mut() {
            let Some(t) = slot else { continue };
            if t.state != State::Zombie {
                continue;
            }
            if t.stack_frames > 0 {
                let canary = *(t.stack_base.wrapping_add(KERNEL_OFFSET) as *const u64);
                if canary != STACK_CANARY {
                    error!(
                        "sched",
                        "stack canary corrupt at reap: thread {} '{}' base={:#x} canary={:#x}",
                        t.id,
                        t.name,
                        t.stack_base,
                        canary
                    );
                    crate::halt::halt_machine("kernel stack overflow (canary at reap)");
                }
                if let Err(e) = crate::frames::free_contiguous(t.stack_base, t.stack_frames) {
                    error!("sched", "stack free failed for thread {}: {}", t.id, e);
                    crate::halt::halt_machine("thread stack free failed");
                }
            }
            *slot = None;
        }
    }
}

/// Canary check for the thread that is about to switch away.
fn check_canary_current() {
    // SAFETY: single reader under IF=0.
    unsafe {
        let cur = (*CPUS.get())[this_cpu()].current;
        let t = (*THREADS.get())[cur].expect("current thread vanished");
        if t.stack_frames == 0 {
            return; // bootstrap: stack not owned, no canary
        }
        let canary = *(t.stack_base.wrapping_add(KERNEL_OFFSET) as *const u64);
        if canary != STACK_CANARY {
            error!(
                "sched",
                "stack canary corrupt: thread {} '{}' base={:#x} canary={:#x} (want {:#x})",
                t.id,
                t.name,
                t.stack_base,
                canary,
                STACK_CANARY
            );
            crate::halt::halt_machine("kernel stack overflow (canary at switch)");
        }
    }
}

// ---- observability (the M3 suite asserts on these) ----------------------

/// Threads not yet exited (Ready + Running, including the bootstrap).
/// Zombies count as gone; the next scheduler entry reaps them.
pub fn live_threads() -> usize {
    without_interrupts(|| {
        // SAFETY: single reader under IF=0.
        unsafe {
            (*THREADS.get())
                .iter()
                .filter(|s| matches!(s, Some(t) if t.state != State::Zombie))
                .count()
        }
    })
}

/// Total context switches performed (one per save/restore pair),
/// cooperative and preemptive alike.
pub fn switch_count() -> u64 {
    SWITCH_COUNT.load(Ordering::Relaxed)
}

/// Total successful spawns since init.
pub fn spawned_total() -> u64 {
    SPAWNED_TOTAL.load(Ordering::Relaxed)
}

/// Kernel-view stack range `(base_va_incl, top_va_excl)` of the current
/// thread; `(0, 0)` for the bootstrap thread (stack not scheduler-owned).
pub fn current_stack_range() -> (u64, u64) {
    without_interrupts(|| {
        // SAFETY: single reader under IF=0.
        unsafe {
            let cur = (*CPUS.get())[this_cpu()].current;
            let t = (*THREADS.get())[cur].expect("current thread vanished");
            if t.stack_frames == 0 {
                return (0, 0);
            }
            let base = t.stack_base.wrapping_add(KERNEL_OFFSET);
            (base, base + THREAD_STACK_BYTES)
        }
    })
}

// ---- M3.3: user-thread plumbing (ADR-0014) ---------------------------------

/// Kernel stack top of the current thread (one past the last byte — the
/// value TSS RSP0 and the syscall scratch carry); `None` for the
/// bootstrap thread, whose stack the scheduler does not own.
pub fn current_kernel_stack_top() -> Option<u64> {
    without_interrupts(|| {
        // SAFETY: single reader under IF=0.
        unsafe {
            let cur = (*CPUS.get())[this_cpu()].current;
            let t = (*THREADS.get())[cur].expect("current thread vanished");
            if t.stack_frames == 0 {
                return None;
            }
            Some(t.stack_base.wrapping_add(KERNEL_OFFSET) + THREAD_STACK_BYTES)
        }
    })
}

/// Kernel stack top of the thread with `id`, while it exists (test
/// evidence for the RSP0 programming; `None` when no live thread has it).
pub fn thread_stack_top(id: u64) -> Option<u64> {
    without_interrupts(|| {
        // SAFETY: single reader under IF=0.
        unsafe {
            for t in (*THREADS.get()).iter().flatten() {
                if t.id == id && t.stack_frames > 0 {
                    return Some(t.stack_base.wrapping_add(KERNEL_OFFSET) + THREAD_STACK_BYTES);
                }
            }
            None
        }
    })
}

/// The entry pointer stored in thread `id`'s slot (readback for the
/// suite: what spawn wrote is what the trampoline will call).
pub fn thread_entry_of(id: u64) -> Option<usize> {
    without_interrupts(|| {
        // SAFETY: single reader under IF=0.
        unsafe {
            (*THREADS.get())
                .iter()
                .flatten()
                .find(|t| t.id == id)
                .map(|t| t.entry as usize)
        }
    })
}

/// Id of the thread running on this CPU.
pub fn current_thread_id() -> u64 {
    without_interrupts(|| {
        // SAFETY: single reader under IF=0.
        unsafe {
            let cur = (*CPUS.get())[this_cpu()].current;
            (*THREADS.get())[cur].expect("current thread vanished").id
        }
    })
}

/// The current thread's registered user regions (copy-out; zeros in the
/// unused slots). The syscall dispatcher validates user pointers against
/// exactly this table.
pub fn current_user_regions() -> [(u64, u64); USER_REGIONS_MAX] {
    without_interrupts(|| {
        // SAFETY: single reader under IF=0.
        unsafe {
            let cur = (*CPUS.get())[this_cpu()].current;
            (*THREADS.get())[cur]
                .expect("current thread vanished")
                .regions
        }
    })
}

/// Register the current thread's user regions (page-granular `(lo, hi)`
/// pairs, at most [`USER_REGIONS_MAX`], zeros fill the rest). Call with
/// IF=0 before `enter_user` (or at process-thread creation).
pub fn set_current_user_regions(regions: &[(u64, u64)]) -> Result<(), &'static str> {
    if regions.len() > USER_REGIONS_MAX {
        return Err("too many user regions");
    }
    without_interrupts(|| {
        // SAFETY: single writer under IF=0; as_mut() — expect-assign on a
        // Copy place would write to a temporary (CODING-CONVENTIONS).
        unsafe {
            let cur = (*CPUS.get())[this_cpu()].current;
            let t = (*THREADS.get())[cur]
                .as_mut()
                .expect("current thread vanished");
            t.regions = [(0, 0); USER_REGIONS_MAX];
            for (i, r) in regions.iter().enumerate() {
                t.regions[i] = *r;
            }
        }
        Ok(())
    })
}

/// Terminate the current thread from kernel context — the SYS_EXIT door
/// into the normal zombie/reap path (ADR-0014). Diverges like `exit_now`.
pub fn terminate() -> ! {
    exit_now()
}
