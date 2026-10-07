//! Capability-backed native Mutex, Condvar, and Once primitives (ADR-0107).
//!
//! Each waitable primitive owns one generation-checked key in the exact
//! SyncDomain granted by the Desktop. The kernel compares the key sequence
//! and parks a scheduler thread atomically; no polling, ambient handle, or
//! Linux futex ABI is involved.

use core::{
    cell::UnsafeCell,
    marker::PhantomData,
    sync::atomic::{AtomicBool, AtomicU8, Ordering},
};

use arena_lib::abi::{
    CAP_KIND_SYNC_DOMAIN, RIGHTS_READ, RIGHTS_WRITE, STATUS_BUSY, STATUS_TIMEOUT, SYS_CAP_DESCRIBE,
    SYS_SYNC_INFO, SYS_SYNC_KEY_CREATE, SYS_SYNC_KEY_DESTROY, SYS_SYNC_SEQUENCE, SYS_SYNC_WAIT,
    SYS_SYNC_WAKE, syscall2, syscall6,
};
use arena_startup_abi::startup::{CapabilityRole, StartupView};

const WAKE_ONE: u64 = 1;
const WAKE_ALL: u64 = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    MissingDomain,
    InvalidCapability,
    Kernel(i64),
    TimedOut,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SyncDomain {
    slot: u8,
}

impl SyncDomain {
    /// Resolve the exact domain descriptor verified by `startup::run`.
    pub fn from_startup(view: &StartupView<'_>) -> Result<Self, Error> {
        let descriptor = view
            .capability_for_role(CapabilityRole::SyncDomain)
            .ok_or(Error::MissingDomain)?;
        if descriptor.kind != CAP_KIND_SYNC_DOMAIN
            || u64::from(descriptor.rights) != (RIGHTS_READ | RIGHTS_WRITE)
            || descriptor.slot == 0
            || usize::from(descriptor.slot) >= arena_lib::abi::CAP_SLOTS
        {
            return Err(Error::InvalidCapability);
        }
        let mut observed = [0u64; 3];
        let status = unsafe {
            syscall2(
                SYS_CAP_DESCRIBE,
                u64::from(descriptor.slot),
                observed.as_mut_ptr() as u64,
            )
        };
        if status != 0
            || observed[0] != u64::from(CAP_KIND_SYNC_DOMAIN)
            || observed[2] != (RIGHTS_READ | RIGHTS_WRITE)
        {
            return Err(Error::InvalidCapability);
        }
        Ok(Self {
            slot: descriptor.slot as u8,
        })
    }

    /// Create one bounded condition sequence within this exact domain.
    fn key(self) -> Result<WaitKey, Error> {
        let token = unsafe { syscall6(SYS_SYNC_KEY_CREATE, u64::from(self.slot), 0, 0, 0, 0, 0) };
        if token <= 0 {
            return Err(Error::Kernel(token));
        }
        Ok(WaitKey {
            domain: self,
            token: token as u64,
        })
    }

    /// Current/high-water keys and parked waiters for resource accounting.
    pub fn info(self) -> Result<DomainInfo, Error> {
        let mut words = [0u64; 4];
        let status = unsafe {
            syscall6(
                SYS_SYNC_INFO,
                u64::from(self.slot),
                words.as_mut_ptr() as u64,
                0,
                0,
                0,
                0,
            )
        };
        if status != 0 {
            return Err(Error::Kernel(status));
        }
        Ok(DomainInfo {
            keys: words[0],
            keys_high: words[1],
            parked_waiters: words[2],
            waiters_high: words[3],
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DomainInfo {
    pub keys: u64,
    pub keys_high: u64,
    pub parked_waiters: u64,
    pub waiters_high: u64,
}

struct WaitKey {
    domain: SyncDomain,
    token: u64,
}

impl WaitKey {
    fn sequence(&self) -> Result<u64, Error> {
        let sequence = unsafe {
            syscall6(
                SYS_SYNC_SEQUENCE,
                u64::from(self.domain.slot),
                self.token,
                0,
                0,
                0,
                0,
            )
        };
        if sequence <= 0 {
            return Err(Error::Kernel(sequence));
        }
        Ok(sequence as u64)
    }

    fn wait(&self, observed: u64, timeout_us: u64) -> Result<(), Error> {
        let status = unsafe {
            syscall6(
                SYS_SYNC_WAIT,
                u64::from(self.domain.slot),
                self.token,
                observed,
                timeout_us,
                0,
                0,
            )
        };
        match status {
            0 => Ok(()),
            STATUS_TIMEOUT => Err(Error::TimedOut),
            status => Err(Error::Kernel(status)),
        }
    }

    fn wake_one(&self) -> Result<u64, Error> {
        self.wake(WAKE_ONE)
    }

    fn wake_all(&self) -> Result<u64, Error> {
        self.wake(WAKE_ALL)
    }

    fn wake(&self, mode: u64) -> Result<u64, Error> {
        let count = unsafe {
            syscall6(
                SYS_SYNC_WAKE,
                u64::from(self.domain.slot),
                self.token,
                mode,
                0,
                0,
                0,
            )
        };
        if count < 0 {
            Err(Error::Kernel(count))
        } else {
            Ok(count as u64)
        }
    }
}

impl Drop for WaitKey {
    fn drop(&mut self) {
        let _ = unsafe {
            syscall6(
                SYS_SYNC_KEY_DESTROY,
                u64::from(self.domain.slot),
                self.token,
                0,
                0,
                0,
                0,
            )
        };
    }
}

/// A native sequence event for building synchronization patterns below the
/// Mutex/Condvar layer. Signals are not retained: capture `sequence`, publish
/// your predicate, then `wait` with that captured value while paired with a
/// lock that protects the predicate.
pub struct Event {
    key: WaitKey,
}

impl Event {
    pub fn new(domain: SyncDomain) -> Result<Self, Error> {
        Ok(Self { key: domain.key()? })
    }

    pub fn sequence(&self) -> Result<u64, Error> {
        self.key.sequence()
    }

    pub fn wait(&self, observed: u64, timeout_us: u64) -> Result<(), Error> {
        self.key.wait(observed, timeout_us)
    }

    pub fn signal_one(&self) -> Result<u64, Error> {
        self.key.wake_one()
    }

    pub fn signal_all(&self) -> Result<u64, Error> {
        self.key.wake_all()
    }
}

/// A cooperative, blocking mutex. Contenders park on a private native wait
/// key; the kernel wakes one contender after the release-store to the lock.
pub struct Mutex<T> {
    locked: AtomicBool,
    value: UnsafeCell<T>,
    key: WaitKey,
}

unsafe impl<T: Send> Send for Mutex<T> {}
unsafe impl<T: Send> Sync for Mutex<T> {}

impl<T> Mutex<T> {
    pub fn new(domain: SyncDomain, value: T) -> Result<Self, Error> {
        Ok(Self {
            locked: AtomicBool::new(false),
            value: UnsafeCell::new(value),
            key: domain.key()?,
        })
    }

    pub fn try_lock(&self) -> Option<MutexGuard<'_, T>> {
        self.locked
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .ok()
            .map(|_| MutexGuard {
                mutex: self,
                _not_send: PhantomData,
            })
    }

    pub fn lock(&self) -> Result<MutexGuard<'_, T>, Error> {
        loop {
            if let Some(guard) = self.try_lock() {
                return Ok(guard);
            }
            let observed = self.key.sequence()?;
            if let Some(guard) = self.try_lock() {
                return Ok(guard);
            }
            match self.key.wait(observed, 0) {
                Ok(()) => {}
                Err(Error::Kernel(STATUS_BUSY)) => {
                    let _ = crate::threads::yield_now();
                }
                Err(error) => return Err(error),
            }
        }
    }
}

pub struct MutexGuard<'a, T> {
    mutex: &'a Mutex<T>,
    // Guards are process-local and cannot move to another user thread.
    _not_send: PhantomData<*mut ()>,
}

impl<T> core::ops::Deref for MutexGuard<'_, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        // SAFETY: this guard is created only after acquiring the mutex.
        unsafe { &*self.mutex.value.get() }
    }
}

impl<T> core::ops::DerefMut for MutexGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        // SAFETY: the non-Send exclusive guard owns the mutex for its life.
        unsafe { &mut *self.mutex.value.get() }
    }
}

impl<T> Drop for MutexGuard<'_, T> {
    fn drop(&mut self) {
        self.mutex.locked.store(false, Ordering::Release);
        let _ = self.mutex.key.wake_one();
    }
}

/// Multi-waiter condition variable backed by sequence comparison and
/// scheduler parking. Every wake changes the sequence even when no waiter is
/// parked, which closes the unlock-to-park lost-wakeup window.
pub struct Condvar {
    key: WaitKey,
}

impl Condvar {
    pub fn new(domain: SyncDomain) -> Result<Self, Error> {
        Ok(Self { key: domain.key()? })
    }

    pub fn wait<'a, T>(&self, guard: MutexGuard<'a, T>) -> Result<MutexGuard<'a, T>, Error> {
        let mutex = guard.mutex;
        if mutex.key.domain != self.key.domain {
            return Err(Error::InvalidCapability);
        }
        let observed = self.key.sequence()?;
        drop(guard);
        let wait = self.key.wait(observed, 0);
        let guard = mutex.lock()?;
        wait.map(|()| guard)
    }

    /// Return `(guard, timed_out)`. Timeout delivery uses the kernel's
    /// monotonic 100 Hz tick and may be late by at most one tick.
    pub fn wait_timeout<'a, T>(
        &self,
        guard: MutexGuard<'a, T>,
        timeout_us: u64,
    ) -> Result<(MutexGuard<'a, T>, bool), Error> {
        let mutex = guard.mutex;
        if mutex.key.domain != self.key.domain {
            return Err(Error::InvalidCapability);
        }
        let observed = self.key.sequence()?;
        drop(guard);
        let wait = self.key.wait(observed, timeout_us);
        let guard = mutex.lock()?;
        match wait {
            Ok(()) => Ok((guard, false)),
            Err(Error::TimedOut) => Ok((guard, true)),
            Err(error) => Err(error),
        }
    }

    pub fn notify_one(&self) -> Result<u64, Error> {
        self.key.wake_one()
    }

    pub fn notify_all(&self) -> Result<u64, Error> {
        self.key.wake_all()
    }
}

/// Run one initializer and park other callers until it completes.
pub struct Once {
    state: AtomicU8,
    key: WaitKey,
}

impl Once {
    pub fn new(domain: SyncDomain) -> Result<Self, Error> {
        Ok(Self {
            state: AtomicU8::new(0),
            key: domain.key()?,
        })
    }

    pub fn call_once(&self, initialize: impl FnOnce()) -> Result<(), Error> {
        let mut initialize = Some(initialize);
        loop {
            match self.state.load(Ordering::Acquire) {
                2 => return Ok(()),
                0 => {
                    if self
                        .state
                        .compare_exchange(0, 1, Ordering::AcqRel, Ordering::Acquire)
                        .is_ok()
                    {
                        initialize.take().expect("Once initializer selected")();
                        self.state.store(2, Ordering::Release);
                        self.key.wake_all()?;
                        return Ok(());
                    }
                }
                _ => {
                    let observed = self.key.sequence()?;
                    if self.state.load(Ordering::Acquire) != 1 {
                        continue;
                    }
                    match self.key.wait(observed, 0) {
                        Ok(()) => {}
                        Err(Error::Kernel(STATUS_BUSY)) => {
                            let _ = crate::threads::yield_now();
                        }
                        Err(error) => return Err(error),
                    }
                }
            }
        }
    }

    pub fn is_complete(&self) -> bool {
        self.state.load(Ordering::Acquire) == 2
    }
}
