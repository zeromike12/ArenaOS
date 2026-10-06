//! Processes (ROADMAP 3.3, ADR-0014): a process is an *owned address
//! space* — one PML4 root frame whose user half (entries 0..255) is
//! private and whose kernel half (entries 256..511) was cloned from the
//! kernel view at creation. Cloning means ring-3↔ring-0 transitions
//! never switch CR3 and every process sees exactly one kernel address
//! space; private user halves mean the same user VA in two processes
//! names two different frames.
//!
//! Since 3.4 the process also anchors its capability space ([`crate::cap`],
//! ADR-0015): `caps` below is born empty at `create` and dies with the
//! slot at `destroy`. Real program loading (image format, loader,
//! spawn-from-file) arrives in 4.1. Until then a process is a
//! kernel-internal object, exercised by the M3 self-tests with exact
//! frame accounting and a ring-3 payload actually running inside a
//! process address space.

use crate::arch::x86_64::paging;
use crate::cap;
use crate::frames;
use crate::sync::{SyncCell, without_interrupts};
use core::sync::atomic::{AtomicU64, Ordering};

/// Process-table bound. Fixed capacity, no dynamic growth — the same
/// discipline as the thread table (MAX_THREADS); both revisit when the
/// heap-backed object story matures.
pub const MAX_PROCESSES: usize = 64;

/// A live process: identity, the address space it owns, and its
/// capability space (ADR-0014 §5 anchor: process = address space +
/// capability space; the ADR-0015 half lives inline so both die with
/// the table slot).
#[derive(Clone, Copy)]
pub struct Process {
    pub id: u64,
    pub name: &'static str,
    /// PHYS of the owned PML4 root frame: user half private, kernel half
    /// cloned at creation ([`paging::build_process_pml4`]). Never 0 while
    /// the slot is occupied; freed exactly once, by [`destroy`].
    pub pml4_phys: u64,
    /// Capability slots — see [`crate::cap`] for every operation.
    pub caps: cap::CapSpace,
    /// Registered exit notification `(nid, badge)` — fired when this
    /// process's LAST live thread exits (spawn protocol, ADR-0019).
    /// `nid == u32::MAX` = none registered.
    exit_notif: (u32, u64),
}

/// The process table. Slots are `Option<Process>`; occupancy is the only
/// state (single CPU, all access under IF=0).
static PROCESSES: SyncCell<[Option<Process>; MAX_PROCESSES]> = SyncCell::new([None; MAX_PROCESSES]);

/// Next process id. Pid 0 is reserved and never handed out: it reads as
/// "no process" (the kernel view / bootstrap context).
static NEXT_PID: SyncCell<u64> = SyncCell::new(1);

/// Total processes created since boot (churn evidence for the
/// accounting test; destroy is measured through frame accounting).
static CREATED_TOTAL: AtomicU64 = AtomicU64::new(0);

/// Create a process: allocate and initialize its PML4 (user half empty,
/// kernel half cloned from the live kernel view), claim a table slot,
/// return the new pid.
pub fn create(name: &'static str) -> Result<u64, &'static str> {
    without_interrupts(|| {
        // SAFETY: IF=0; frames allocator live; kernel view built (kmain
        // runs the suite long after paging::init).
        let Some(pml4) = (unsafe { paging::build_process_pml4() }) else {
            return Err("frame exhaustion: no PML4 root for a process");
        };
        // SAFETY: single writer under IF=0; `pml4` is a fresh owned root.
        unsafe {
            let procs = &mut *PROCESSES.get();
            let Some(slot) = procs.iter().position(Option::is_none) else {
                frames::free(pml4).map_err(|_| "create: PML4 root free rejected")?;
                return Err("process table full (MAX_PROCESSES)");
            };
            let id = *NEXT_PID.get();
            *NEXT_PID.get() = id + 1;
            procs[slot] = Some(Process {
                id,
                name,
                pml4_phys: pml4,
                caps: cap::CapSpace::new(),
                exit_notif: (u32::MAX, 0),
            });
            CREATED_TOTAL.fetch_add(1, Ordering::Relaxed);
            Ok(id)
        }
    })
}

/// Register the child's exit notification (ADR-0019): when `pid`'s last
/// live thread exits, the kernel fires `ipc::notify(nid, badge)`.
/// Called by the spawn path before the child's first thread starts.
pub fn set_exit_notif(pid: u64, nid: u32, badge: u64) -> Result<(), &'static str> {
    without_interrupts(|| {
        // SAFETY: single writer under IF=0.
        unsafe {
            let Some(p) = (*PROCESSES.get())
                .iter_mut()
                .flatten()
                .find(|p| p.id == pid)
            else {
                return Err("set_exit_notif: no such process");
            };
            p.exit_notif = (nid, badge);
            Ok(())
        }
    })
}

/// The registered exit notification of a live process (`None` when
/// unknown or none registered).
pub fn exit_notif_of(pid: u64) -> Option<(u32, u64)> {
    without_interrupts(|| {
        // SAFETY: single reader under IF=0.
        unsafe {
            (*PROCESSES.get())
                .iter()
                .flatten()
                .find(|p| p.id == pid)
                .and_then(|p| (p.exit_notif.0 != u32::MAX).then_some(p.exit_notif))
        }
    })
}

/// The PML4 root PHYS of a live process (`None` for unknown pids).
pub fn pml4_of(pid: u64) -> Option<u64> {
    without_interrupts(|| {
        // SAFETY: single reader under IF=0.
        unsafe {
            (*PROCESSES.get())
                .iter()
                .flatten()
                .find(|p| p.id == pid)
                .map(|p| p.pml4_phys)
        }
    })
}

/// Destroy a process: free every user-half frame (leaf pages *and*
/// intermediate tables, via [`paging::destroy_user_half`]), then free the
/// owned root frame, then release the table slot. Returns the total
/// number of frames freed.
///
/// Refuses to destroy the address space currently loaded in CR3 — a
/// thread must be switched out of it first (the scheduler restores the
/// kernel view when the incoming thread's cr3 differs; the bootstrap
/// thread carries the kernel-view cr3, so any switch back to it does).
pub fn destroy(pid: u64) -> Result<u64, &'static str> {
    crate::image_registry::manager_death_check(pid);
    // M6.5 (ADR-0028): FIRST, answer everyone this process owed a
    // reply to, with a typed status. A client blocked in
    // `SYS_IPC_CALL` has no timeout and no way to observe its
    // server's liveness; without this it waits forever for a reply
    // nobody will ever stage.
    //
    // This runs BEFORE the teardown below, and deliberately outside
    // it, for two reasons. The endpoints a process served are
    // discovered from its own capability space, which the teardown is
    // about to erase — and the teardown holds a `&mut` borrow of the
    // process table that reading those capabilities would alias. The
    // first attempt did it inside, after the slot was already cleared,
    // and silently found nothing: the sweep reported zero failed
    // calls on a client that was demonstrably blocked.
    //
    // A pid that does not exist owes nobody anything, so doing this
    // before the existence check below costs nothing and keeps the
    // ordering simple.
    let failed = crate::ipc::fail_calls_for_server(pid);
    if failed > 0 {
        crate::log::log_info!(
            "proc",
            "destroy pid {pid}: {failed} in-flight call(s) answered STATUS_SERVICE_GONE"
        );
    }
    // Then drop every kernel reference to this process's threads, and
    // only then kill them. Order matters and is not stylistic: an
    // endpoint or notification still holding a dead thread's id would
    // try to wake a corpse, which halts the machine by design.
    //
    // Killing is what makes a process destroyable AT ALL while it is
    // doing something. Before M6.5 `destroy` tore down the address
    // space and left any parked thread live forever — fine for the
    // suites, which only ever destroyed processes whose threads had
    // already exited, and useless for a supervisor, whose whole job is
    // to kill drivers that are blocked waiting for work or for a
    // device that will never answer.
    // M6.5b: tell the supervisor before the process is gone. It only
    // MARKS the death here — restarting allocates and maps, which this
    // context must not do (see supervise::note_death).
    if crate::supervise::is_supervised(pid) {
        crate::supervise::note_death(pid);
        crate::log::log_info!(
            "proc",
            "destroy pid {pid}: a SUPERVISED service died — the supervisor will restart it"
        );
    }
    // M7.0 (ADR-0029): a dead process must not keep signalling. The
    // fourth thing swept here, after relay vectors (M5.2), the console
    // mirror (M6.4) and blocked-thread references (M6.5) — the pattern
    // is now the rule for anything that holds a pid.
    let timers = crate::timer::release_by_owner(pid);
    if timers > 0 {
        crate::log::log_info!("proc", "destroy pid {pid}: swept {timers} armed timer(s)");
    }
    let (servers, waiters, calls) = crate::ipc::release_blocked_of(pid);
    let killed = crate::sched::kill_threads_of(pid);
    if killed > 0 || calls > 0 {
        crate::log::log_info!(
            "proc",
            "destroy pid {pid}: killed {killed} live thread(s) (released {servers} endpoint server slot(s), {waiters} notification waiter(s), {calls} abandoned caller slot(s))"
        );
    }
    without_interrupts(|| {
        // SAFETY: IF=0; the root is owned by the table slot found here
        // and freed exactly once (the slot is cleared in the same
        // critical section); destroy_user_half's contract (root not
        // live, allocator live) is checked/enforced below.
        unsafe {
            let procs = &mut *PROCESSES.get();
            let Some(idx) = procs
                .iter()
                .position(|slot| matches!(slot, Some(p) if p.id == pid))
            else {
                return Err("destroy: no such process");
            };
            let p = procs[idx].expect("slot located by position above");
            if paging::cr3_phys() == p.pml4_phys {
                return Err("destroy: process address space is the live CR3");
            }
            let freed = paging::destroy_user_half(p.pml4_phys) as u64;
            frames::free(p.pml4_phys).map_err(|_| "destroy: root frame free rejected")?;
            // A dying capspace drops every Image reference, including
            // inherited and IPC-landed copies, before releasing its slot.
            p.caps.each_cap(crate::image_registry::drop_cap);
            p.caps.each_cap(crate::shared::drop_cap);
            procs[idx] = None;
            // PTE teardown has already skipped live SharedRegion backings.
            // Mapping pins outlive the caps, then disappear exactly once.
            crate::shared::release_maps(pid);
            // Sweep the process's owned IRQ relays (M5.2, ADR-0022): a
            // dead driver's armed vectors must not keep notifying a dead
            // notification — the relay table entries die with the owner.
            let swept = crate::relay::release_by_owner(pid);
            // M6.4 (ADR-0027): the console output mirror is owned the
            // same way a relay vector is — a dead channel must not
            // leave the kernel appending to a ring nobody drains, or
            // notifying a notification that no longer exists.
            if crate::console::detach_output_by_owner(pid) {
                crate::log::log_info!("proc", "destroy pid {pid}: console output mirror detached");
            }
            if swept > 0 {
                crate::log::log_info!(
                    "proc",
                    "destroy pid {pid}: released {swept} owned IRQ relay vector(s)"
                );
            }
            Ok(freed + 1)
        }
    })
}

/// Test-only independent Image conservation oracle: traverse every actual
/// occupied process capspace slot (not the production ref counters).
pub fn for_each_cap(mut f: impl FnMut(cap::Cap)) {
    without_interrupts(|| unsafe {
        for p in (*PROCESSES.get()).iter().flatten() {
            p.caps.each_cap(&mut f);
        }
    });
}

/// Run `f` against a live process's capability space (ADR-0015: the cap
/// layer reaches spaces only through this pair, so the process table is
/// borrowed exactly once per operation). `None` for an unknown pid.
pub fn with_caps<R>(pid: u64, f: impl FnOnce(&cap::CapSpace) -> R) -> Option<R> {
    without_interrupts(|| {
        // SAFETY: single reader under IF=0.
        unsafe {
            (*PROCESSES.get())
                .iter()
                .flatten()
                .find(|p| p.id == pid)
                .map(|p| f(&p.caps))
        }
    })
}

/// Mutable twin of [`with_caps`] — the only path that writes cap slots.
pub fn with_caps_mut<R>(pid: u64, f: impl FnOnce(&mut cap::CapSpace) -> R) -> Option<R> {
    without_interrupts(|| {
        // SAFETY: single writer under IF=0.
        unsafe {
            (*PROCESSES.get())
                .iter_mut()
                .flatten()
                .find(|p| p.id == pid)
                .map(|p| f(&mut p.caps))
        }
    })
}

/// Occupied process-table slots right now.
pub fn live_count() -> usize {
    without_interrupts(|| {
        // SAFETY: single reader under IF=0.
        unsafe { (*PROCESSES.get()).iter().flatten().count() }
    })
}

/// Total processes created since boot.
/// Snapshot of the live process table for `SYS_PROC_LIST` (ADR-0020):
/// fills `out` with `(pid, live_thread_count)` pairs in table order and
/// returns how many pairs were written (at most `out.len()`).
pub fn list_live(out: &mut [(u64, usize)]) -> usize {
    without_interrupts(|| {
        // SAFETY: single reader under IF=0; the nested
        // proc_live_threads bracket is re-entrant (save/restore flags).
        unsafe {
            let mut n = 0;
            for slot in (*PROCESSES.get()).iter() {
                if n >= out.len() {
                    break;
                }
                if let Some(p) = slot {
                    out[n] = (p.id, crate::sched::proc_live_threads(p.id));
                    n += 1;
                }
            }
            n
        }
    })
}

pub fn created_total() -> u64 {
    CREATED_TOTAL.load(Ordering::Relaxed)
}
