//! Timer-driven preemption (M3.2, ADR-0013).
//!
//! The vector-32 stub (idt.rs) EOIs, bumps the absorb counter, and calls
//! [`on_timer_tick`] through the installed hook. The hook counts the
//! running thread's quantum down in PIT ticks; on expiry, with another
//! thread ready, it runs the *same* decision + switch the cooperative
//! path uses — nested inside the interrupt's call chain, on the
//! interrupted thread's own stack. Resume returns up through the hook,
//! handler, and stub epilogue, and `iretq`s into the interrupted body.
//!
//! Soundness rests on one invariant (ADR-0013): every scheduler-state
//! mutation runs with IF=0. The interrupt gate provides it here for
//! free; cooperative sections provide it themselves. A tick can thus
//! never observe a half-finished decision, and this module needs no
//! locking beyond the cell discipline.

use super::{CPUS, SWITCH_COUNT, this_cpu};
use crate::arch::x86_64::context;
use crate::sync::SyncCell;
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};

/// Capped (from_slot << 32 | to_slot) transition log — the deterministic
/// assertion surface for the rotation test (and future scheduler work).
/// Capped, not ringed: entries beyond the cap still count in [`total`].
const LOG_CAP: usize = 64;

static PREEMPT_LOG: SyncCell<[u64; LOG_CAP]> = SyncCell::new([0; LOG_CAP]);
static PREEMPT_LOG_LEN: SyncCell<usize> = SyncCell::new(0);
static PREEMPT_TOTAL: AtomicU64 = AtomicU64::new(0);
static ENABLED: AtomicBool = AtomicBool::new(false);

/// Arm preemption on this CPU: `slice_ticks` PIT ticks per quantum
/// (0 is coerced to 1), hook installed. Call with IF=0; the caller
/// enables interrupts when it wants ticks to start landing.
pub fn enable(slice_ticks: u32) {
    let slice = slice_ticks.max(1);
    crate::sync::without_interrupts(|| {
        // SAFETY: single writer under IF=0.
        unsafe {
            let cpu = &mut (*CPUS.get())[this_cpu()];
            cpu.slice = slice;
            cpu.remaining = slice;
        }
    });
    crate::arch::x86_64::idt::set_timer_hook(Some(on_timer_tick));
    ENABLED.store(true, Ordering::Relaxed);
}

/// Disarm preemption: hook removed first (no further ticks can enter),
/// then the flag. Call with IF=0 for a deterministic boundary.
pub fn disable() {
    crate::arch::x86_64::idt::set_timer_hook(None);
    ENABLED.store(false, Ordering::Relaxed);
    crate::sync::without_interrupts(|| {
        // SAFETY: single writer under IF=0.
        unsafe {
            let cpu = &mut (*CPUS.get())[this_cpu()];
            cpu.slice = 0;
            cpu.remaining = 0;
        }
    });
}

/// Whether preemption is armed.
pub fn enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

/// Total timer-driven switches since boot (log cap does not limit this).
pub fn total() -> u64 {
    PREEMPT_TOTAL.load(Ordering::Relaxed)
}

/// Number of recorded transitions (≤ `LOG_CAP`).
pub fn log_len() -> usize {
    crate::sync::without_interrupts(|| {
        // SAFETY: single reader under IF=0.
        unsafe { *PREEMPT_LOG_LEN.get() }
    })
}

/// Transition `i` as `(from_slot << 32) | to_slot`, if recorded.
pub fn log_entry(i: usize) -> Option<u64> {
    crate::sync::without_interrupts(|| {
        // SAFETY: single reader under IF=0; i bounded by len ≤ LOG_CAP.
        unsafe {
            let len = *PREEMPT_LOG_LEN.get();
            if i < len && len <= LOG_CAP {
                Some((*PREEMPT_LOG.get())[i])
            } else {
                None
            }
        }
    })
}

/// Tick hook — interrupt context: IF=0 (interrupt gate), caller-saved
/// registers saved by the stub below us, EOI already sent. Runs on the
/// interrupted thread's own stack.
pub extern "C" fn on_timer_tick() {
    if !ENABLED.load(Ordering::Relaxed) {
        return;
    }
    // Phase 1 — quantum countdown (scoped borrow; ends before any switch).
    // SAFETY: IF=0 by interrupt gate; no scheduler borrow can be live at
    // interrupt time because every scheduler section itself runs IF=0
    // (ADR-0013 invariant).
    let expired = unsafe {
        let cpu = &mut (*CPUS.get())[this_cpu()];
        if cpu.slice == 0 {
            return; // disarmed between hook read and entry — be inert
        }
        cpu.remaining = cpu.remaining.saturating_sub(1);
        cpu.remaining == 0
    };
    if !expired {
        return;
    }
    // Phase 2 — rotate. plan_switch refills the quantum (for the incoming
    // thread, or for the current one when the ring is empty and no switch
    // happens) and ends every borrow before returning the raw plan.
    let from = unsafe { (*CPUS.get())[this_cpu()].current };
    let Some(plan) = super::plan_switch(true) else {
        return; // nobody to rotate to; the current thread keeps the CPU
    };
    let to = unsafe { (*CPUS.get())[this_cpu()].current };
    record_transition(from, to);
    SWITCH_COUNT.fetch_add(1, Ordering::Relaxed);
    // SAFETY: identical contract to sched::yield_now — raw plan values,
    // borrows ended, IF=0, both stacks mapped RW. This call resumes the
    // incoming thread (wherever its own frame says) and suspends this one
    // until switched back; returning from it unwinds up through the tick
    // handler and stub into the interrupted body (ADR-0013).
    unsafe { context::switch_context(plan.save, plan.restore) };
}

/// Append to the capped transition log; overflow beyond the cap only
/// stops recording, never affects scheduling.
fn record_transition(from: usize, to: usize) {
    PREEMPT_TOTAL.fetch_add(1, Ordering::Relaxed);
    // SAFETY: single writer under IF=0 (interrupt gate).
    unsafe {
        let len = &mut *PREEMPT_LOG_LEN.get();
        if *len < LOG_CAP {
            (*PREEMPT_LOG.get())[*len] = (from as u64) << 32 | to as u64;
            *len += 1;
        }
    }
}
