//! Boot-stage interior mutability.
//!
//! This is not general-purpose synchronization — it is a *documented
//! absence* of it, valid only under the boot contract below, and it must
//! not migrate past the boot stage without an ADR (the M3+ kernel gets
//! real locks; see M2.6).

use core::cell::UnsafeCell;

/// Boot-time single-writer cell.
///
/// SAFETY CONTRACT: the boot stage runs single-CPU with interrupts masked
/// outside firmware calls (UEFI mandates BSP-only execution; ADR-0003).
/// Each cell has exactly one writer during sequential init, before any
/// concurrency exists. SMP-era code must use real synchronization — this
/// type must not migrate past the boot stage without an ADR.
pub(crate) struct SyncCell<T>(UnsafeCell<T>);

impl<T> SyncCell<T> {
    pub(crate) const fn new(value: T) -> Self {
        Self(UnsafeCell::new(value))
    }
    pub(crate) const fn get(&self) -> *mut T {
        self.0.get()
    }
}
unsafe impl<T> Sync for SyncCell<T> {}
