//! Deferred tick work (M7.0, ADR-0029): the small set of jobs that must
//! run on every timer interrupt, in order, at interrupt context.
//!
//! The 100 Hz tick already had two customers before this module
//! existed — the scheduler's preemption hook and, since M6.4, the
//! console mirror's deferred wake — and the second one took the single
//! auxiliary slot `idt` offers. The timer facility needs the same
//! service, and "whoever registers last wins" is not a mechanism.
//!
//! So: one auxiliary hook, owned here, dispatching a fixed list.
//!
//! **What a tick task may do.** It runs in interrupt context with IF=0,
//! on the interrupted thread's kernel stack. It must not block, must
//! not allocate, and must not take long. It MAY wake threads —
//! `ipc::notify` is enqueue-only and the IRQ relay path has done
//! exactly that since M5.2 — which is the whole reason this seam
//! exists: waking is the one privileged thing a periodic job needs.
//!
//! **What it may not do**, learned in M6.4: it may not be called from
//! `serial::putc`. A job that wants to wake somebody in response to
//! console output raises a flag there and does the waking HERE,
//! because putc runs inside the scheduler's own log lines and waking a
//! thread from underneath the scheduler re-enters it.

use crate::arch::x86_64::idt;
use crate::sync::{SyncCell, without_interrupts};

/// Tick tasks this kernel can hold. Three today (console mirror wake,
/// timer expiry, and room for one more); the bound exists so the table
/// is a plain array with no allocation on a path that runs 100 times a
/// second.
pub const MAX_TASKS: usize = 4;

static TASKS: SyncCell<[Option<extern "C" fn()>; MAX_TASKS]> = SyncCell::new([None; MAX_TASKS]);

/// Register a job to run on every tick. Order of registration is order
/// of execution. Call during init, with IF=0.
pub fn register(task: extern "C" fn()) -> Result<(), &'static str> {
    without_interrupts(|| {
        // SAFETY: single writer under IF=0, init-time only.
        unsafe {
            let tasks = &mut *TASKS.get();
            let Some(slot) = tasks.iter_mut().find(|t| t.is_none()) else {
                return Err("tick: task table full (MAX_TASKS)");
            };
            *slot = Some(task);
            // The first registration installs the dispatcher; later
            // ones join a list that is already running.
            idt::set_tick_aux_hook(Some(dispatch));
            Ok(())
        }
    })
}

/// The auxiliary tick hook: run every registered task, in order.
///
/// Cheap when empty and cheap when full — the whole point of the
/// per-task fast paths (a console mirror with no channel attached and
/// a timer table with nothing armed both return after one load).
extern "C" fn dispatch() {
    // Copy the list out before calling anything: a task is free to do
    // whatever a tick task may do, and holding a borrow of this table
    // across those calls would be exactly the kind of aliasing this
    // kernel refuses to risk.
    let tasks = without_interrupts(|| {
        // SAFETY: single reader under IF=0 (interrupt context).
        unsafe { *TASKS.get() }
    });
    for task in tasks.iter().flatten() {
        task();
    }
}
