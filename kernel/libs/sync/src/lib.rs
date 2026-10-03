//! ArenaOS synchronization primitives (M2.6, ADR-0010).
//!
//! `no_std`, zero dependencies (ADR-0004), host-testable: the contention
//! and owner-tracking tests below run on real parallel host threads, which
//! is where lock bugs actually live.
//!
//! # What is here
//!
//! [`Spinlock<T>`] — a test-and-set spinlock with an RAII guard and an
//! owner token recorded on every acquire. Single-core correctness first
//! (the boot contract: BSP only, IF=0); the acquire loop is already the
//! SMP-shaped one (atomic compare-exchange + `spin_loop`), and the ticket/
//! queued decision is deferred to an ADR when SMP lands, per ROADMAP 2.6.
//!
//! # What is deliberately NOT here
//!
//! - **No interrupt masking inside the lock.** Whether a critical region
//!   also needs IF cleared is an arch-level policy (deadlock rules differ
//!   per subsystem); the boot stage composes this lock with
//!   `sync::without_interrupts` where needed.
//! - **No blocking primitives** (sleeplocks, channels, futexes): they need
//!   a scheduler (M3).
//!
//! # Contracts
//!
//! - Locks are acquired through [`Spinlock::lock`]/[`try_lock`] and
//!   released by dropping the guard. Recursive acquisition by the same
//!   owner is a bug: debug builds detect it via the owner token and panic;
//!   release builds deadlock (standard, documented test-and-set behavior).
//! - The owner token comes from the `cpu_id` function supplied at
//!   construction (the boot stage passes the constant BSP id; host tests
//!   pass per-thread tokens). It exists for debugging and diagnostics —
//!   never for correctness decisions.

#![cfg_attr(not(test), no_std)]

use core::cell::UnsafeCell;
use core::ops::{Deref, DerefMut};
use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};

/// Owner-token value meaning "nobody holds the lock".
const NO_OWNER: u32 = u32::MAX;

/// Test-and-set spinlock with debug owner tracking.
///
/// See the crate docs for contracts. Memory ordering: acquire on the
/// held-flag transition, release on guard drop; the owner token is a
/// relaxed diagnostic side-channel and orders nothing.
pub struct Spinlock<T: ?Sized> {
    held: AtomicBool,
    owner: AtomicU32,
    cpu_id: fn() -> u32,
    value: UnsafeCell<T>,
}

// SAFETY: The whole point of the type — `&Spinlock<T>` gives coordinated
// `&mut T` access across threads/CPUs when `T` may move between them
// (Send), and the guard's `&mut T` never outlives the lock.
unsafe impl<T: ?Sized + Send> Sync for Spinlock<T> {}
unsafe impl<T: ?Sized + Send> Send for Spinlock<T> {}

impl<T> Spinlock<T> {
    /// Create an unlocked spinlock protecting `value`. `cpu_id` supplies
    /// the owner token for the current executor (constant BSP id at boot;
    /// per-thread in host tests). Must be a pure, cheap function — it runs
    /// inside the acquire path.
    pub const fn new(value: T, cpu_id: fn() -> u32) -> Self {
        Self {
            held: AtomicBool::new(false),
            owner: AtomicU32::new(NO_OWNER),
            cpu_id,
            value: UnsafeCell::new(value),
        }
    }

    /// Exclusive access without any atomics (the borrow checker proves
    /// nobody else can be inside).
    pub fn get_mut(&mut self) -> &mut T {
        self.value.get_mut()
    }
}

impl<T: ?Sized> Spinlock<T> {
    /// Acquire the lock, spinning while it is held.
    ///
    /// # Panics (debug builds)
    ///
    /// On recursive acquisition: the owner token already equals this
    /// executor's id. Release builds deadlock instead (documented).
    pub fn lock(&self) -> SpinlockGuard<'_, T> {
        while self
            .held
            .compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            debug_assert_ne!(
                self.owner.load(Ordering::Relaxed),
                (self.cpu_id)(),
                "spinlock: recursive acquire — this executor already holds it"
            );
            core::hint::spin_loop();
        }
        self.owner.store((self.cpu_id)(), Ordering::Relaxed);
        SpinlockGuard { lock: self }
    }

    /// Acquire only if uncontended; never spins.
    pub fn try_lock(&self) -> Option<SpinlockGuard<'_, T>> {
        if self
            .held
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_ok()
        {
            self.owner.store((self.cpu_id)(), Ordering::Relaxed);
            Some(SpinlockGuard { lock: self })
        } else {
            None
        }
    }

    /// Current holder's owner token (None when unlocked). Diagnostic only
    /// — a concurrent answer may be stale the moment it is read.
    pub fn owner_token(&self) -> Option<u32> {
        match self.owner.load(Ordering::Relaxed) {
            NO_OWNER => None,
            other => Some(other),
        }
    }

    /// Whether the lock is currently held. Diagnostic only (same caveat).
    pub fn is_locked(&self) -> bool {
        self.held.load(Ordering::Relaxed)
    }
}

/// RAII guard: holds the lock until dropped.
pub struct SpinlockGuard<'a, T: ?Sized> {
    lock: &'a Spinlock<T>,
}

impl<T: ?Sized> Deref for SpinlockGuard<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        // SAFETY: the guard's existence proves this executor holds the
        // lock; no other `&T`/`&mut T` can exist concurrently.
        unsafe { &*self.lock.value.get() }
    }
}

impl<T: ?Sized> DerefMut for SpinlockGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        // SAFETY: as `deref`, and the borrow checker ties this `&mut` to
        // the guard's lifetime.
        unsafe { &mut *self.lock.value.get() }
    }
}

impl<T: ?Sized> Drop for SpinlockGuard<'_, T> {
    fn drop(&mut self) {
        // Clear the owner first: a spinner that observes `held == true`
        // with a stale owner must never see *its own* id there.
        self.lock.owner.store(NO_OWNER, Ordering::Relaxed);
        self.lock.held.store(false, Ordering::Release);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::sync::Arc;
    use std::thread;

    thread_local! {
        /// Per-thread owner token, standing in for a CPU id on the host.
        static TEST_CPU: Cell<u32> = const { Cell::new(NO_OWNER) };
    }

    fn test_cpu_id() -> u32 {
        TEST_CPU.with(Cell::get)
    }

    fn set_cpu(id: u32) {
        TEST_CPU.with(|c| c.set(id));
    }

    #[test]
    fn lock_guard_gives_exclusive_value_access() {
        let lock = Spinlock::new(41u64, test_cpu_id);
        set_cpu(1);
        assert!(!lock.is_locked());
        {
            let mut g = lock.lock();
            assert!(lock.is_locked());
            *g += 1;
        }
        assert!(!lock.is_locked());
        assert_eq!(*lock.lock(), 42);
    }

    #[test]
    fn try_lock_is_none_while_held_and_some_after_drop() {
        let lock = Spinlock::new(0u32, test_cpu_id);
        set_cpu(1);
        let g = lock.lock();
        assert!(lock.try_lock().is_none(), "try_lock must fail while held");
        drop(g);
        let g2 = lock.try_lock().expect("try_lock must succeed once free");
        assert_eq!(*g2, 0);
    }

    #[test]
    fn owner_token_tracks_acquirer_across_threads() {
        static LOCK: Spinlock<u64> = Spinlock::new(7, test_cpu_id);
        static OBSERVED: AtomicBool = AtomicBool::new(false);
        let locked_by_9 = thread::spawn(|| {
            set_cpu(9);
            let g = LOCK.lock();
            // Hold the lock until the observing thread has verified the
            // diagnostics — otherwise it could finish before the asserts.
            while !OBSERVED.load(Ordering::Relaxed) {
                thread::yield_now();
            }
            *g
        });
        // Wait until the other thread is inside, then verify diagnostics.
        while !LOCK.is_locked() {
            thread::yield_now();
        }
        set_cpu(3);
        assert_eq!(LOCK.owner_token(), Some(9), "owner must be the holder");
        assert!(
            LOCK.try_lock().is_none(),
            "other executor must not break in"
        );
        OBSERVED.store(true, Ordering::Relaxed);
        let value = locked_by_9.join().unwrap();
        assert_eq!(value, 7);
        assert_eq!(LOCK.owner_token(), None, "owner must clear on release");
        assert!(!LOCK.is_locked());
    }

    #[test]
    #[should_panic(expected = "recursive acquire")]
    fn recursive_acquire_panics_in_debug_builds() {
        let lock = Spinlock::new(0u32, test_cpu_id);
        set_cpu(5);
        let _outer = lock.lock();
        let _inner = lock.lock(); // debug_assert fires in the spin loop
    }

    #[test]
    fn mutual_exclusion_under_four_parallel_threads() {
        static COUNTER: Spinlock<u64> = Spinlock::new(0, test_cpu_id);
        const THREADS: u64 = 4;
        const PER_THREAD: u64 = 50_000;
        let handles: Vec<_> = (0..THREADS)
            .map(|id| {
                thread::spawn(move || {
                    set_cpu(id as u32);
                    for _ in 0..PER_THREAD {
                        // Plain (non-atomic) increment: only correct if
                        // the lock really excludes the other threads.
                        *COUNTER.lock() += 1;
                    }
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }
        assert_eq!(*COUNTER.lock(), THREADS * PER_THREAD);
    }

    #[test]
    fn protects_handoff_of_non_sync_data() {
        // A Vec behind the lock: threads push through the guard only.
        let lock = Arc::new(Spinlock::new(Vec::<u64>::new(), test_cpu_id));
        let mut handles = Vec::new();
        for id in 0..4u64 {
            let lock = Arc::clone(&lock);
            handles.push(thread::spawn(move || {
                set_cpu(id as u32);
                for round in 0..1_000u64 {
                    lock.lock().push(id * 1_000 + round);
                }
            }));
        }
        for h in handles {
            h.join().unwrap();
        }
        let v = lock.lock();
        assert_eq!(v.len(), 4_000);
        let mut sorted = v.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), 4_000, "no element may be lost or duplicated");
    }
}
