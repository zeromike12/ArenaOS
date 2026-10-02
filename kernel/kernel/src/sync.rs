//! Boot-stage interior mutability.
//!
//! This is not general-purpose synchronization — it is a *documented
//! absence* of it, valid only under the boot contract below, and it must
//! not migrate past the boot stage without an ADR (the M3+ kernel gets
//! real locks; see M2.6).

use crate::arch::x86_64;
use core::cell::UnsafeCell;

/// Boot-time single-writer cell.
///
/// SAFETY CONTRACT: the boot stage runs single-CPU with interrupts masked
/// outside firmware calls (UEFI mandates BSP-only execution; ADR-0003).
/// Each cell has exactly one writer during sequential init, before any
/// concurrency exists. SMP-era code must use real synchronization — this
/// type must not migrate past the boot stage without an ADR.
pub struct SyncCell<T>(UnsafeCell<T>);

impl<T> SyncCell<T> {
    pub const fn new(value: T) -> Self {
        Self(UnsafeCell::new(value))
    }
    pub const fn get(&self) -> *mut T {
        self.0.get()
    }
}
unsafe impl<T> Sync for SyncCell<T> {}

/// Run `f` with interrupts masked — irqsave/irqrestore critical section
/// (ADR-0010). On exit the *exact* saved RFLAGS are restored, so IF is
/// re-enabled only if it was set on entry; nesting is therefore correct.
///
/// If `f` panics the restore is skipped, which is fine: the panic path
/// halts the machine anyway (panic.rs).
pub fn without_interrupts<R>(f: impl FnOnce() -> R) -> R {
    let saved = x86_64::read_flags();
    x86_64::cli();
    let result = f();
    // SAFETY: `saved` was produced by read_flags in this same context,
    // moments ago, with no flag mutations in between (cli only cleared IF,
    // and popfq reinstates the full saved value).
    unsafe { x86_64::restore_flags(saved) };
    result
}

/// Whether interrupts are currently enabled (IF flag).
pub fn interrupts_enabled() -> bool {
    x86_64::interrupts_enabled()
}
