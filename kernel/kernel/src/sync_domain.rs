//! Capability-backed native synchronization wait domains (ADR-0107).
//!
//! Sequence comparison, waiter registration, and scheduler blocking share one
//! single-core critical section. A notify that lands before a waiter parks
//! advances the sequence, so the waiter observes the change and never sleeps
//! through it. Domain and key tokens are descriptive; every operation also
//! resolves the exact held SyncDomain capability.

use crate::arch::x86_64::syscall::{STATUS_BAD_ARG, STATUS_BUSY, STATUS_QUOTA};
use crate::log::log_error as error;
use crate::sched;
use crate::sync::{SyncCell, without_interrupts};
use core::sync::atomic::{AtomicUsize, Ordering};

pub const MAX_DOMAINS: usize = 64;
pub const MAX_KEYS_PER_DOMAIN: usize = 32;
pub const MAX_KEYS: usize = MAX_DOMAINS * MAX_KEYS_PER_DOMAIN;
pub const MAX_WAITERS: usize = sched::MAX_THREADS;
pub const MAX_TIMEOUT_US: u64 = 86_400_000_000;

const KEY_INDEX_BITS: u32 = 11;
const KEY_INDEX_MASK: u64 = (1 << KEY_INDEX_BITS) - 1;
const MAX_KEY_GENERATION: u64 = (i64::MAX as u64) >> KEY_INDEX_BITS;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DomainRef {
    pub id: u32,
    pub generation: u32,
}

#[derive(Clone, Copy)]
struct Domain {
    live: bool,
    generation: u32,
    owner: u64,
    keys: u8,
    keys_high: u8,
    waiters_high: u8,
}

const EMPTY_DOMAIN: Domain = Domain {
    live: false,
    generation: 0,
    owner: 0,
    keys: 0,
    keys_high: 0,
    waiters_high: 0,
};

#[derive(Clone, Copy)]
struct Key {
    live: bool,
    domain: u32,
    domain_generation: u32,
    owner: u64,
    generation: u64,
    sequence: u64,
}

const EMPTY_KEY: Key = Key {
    live: false,
    domain: u32::MAX,
    domain_generation: 0,
    owner: 0,
    generation: 0,
    sequence: 0,
};

#[derive(Clone, Copy, PartialEq, Eq)]
enum WaitState {
    Empty,
    Parked,
    Notified,
    TimedOut,
    DomainGone,
    KeyGone,
}

#[derive(Clone, Copy)]
struct Waiter {
    state: WaitState,
    tid: u64,
    pid: u64,
    domain: u32,
    domain_generation: u32,
    key_slot: u16,
    key_generation: u64,
    deadline_us: u64,
}

const EMPTY_WAITER: Waiter = Waiter {
    state: WaitState::Empty,
    tid: 0,
    pid: 0,
    domain: u32::MAX,
    domain_generation: 0,
    key_slot: u16::MAX,
    key_generation: 0,
    deadline_us: 0,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    BadArgument,
    Busy,
    Quota,
    TimedOut,
    DomainGone,
    KeyGone,
}

impl Error {
    pub const fn status(self) -> i64 {
        match self {
            Self::BadArgument => STATUS_BAD_ARG,
            Self::Busy => STATUS_BUSY,
            Self::Quota => STATUS_QUOTA,
            Self::TimedOut => crate::arch::x86_64::syscall::STATUS_TIMEOUT,
            Self::DomainGone => crate::arch::x86_64::syscall::STATUS_SERVICE_GONE,
            Self::KeyGone => crate::arch::x86_64::syscall::STATUS_SERVICE_GONE,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DomainInfo {
    pub keys: u64,
    pub keys_high: u64,
    pub parked_waiters: u64,
    pub waiters_high: u64,
}

static DOMAINS: SyncCell<[Domain; MAX_DOMAINS]> = SyncCell::new([EMPTY_DOMAIN; MAX_DOMAINS]);
static KEYS: SyncCell<[Key; MAX_KEYS]> = SyncCell::new([EMPTY_KEY; MAX_KEYS]);
static WAITERS: SyncCell<[Waiter; MAX_WAITERS]> = SyncCell::new([EMPTY_WAITER; MAX_WAITERS]);
static PARKED: AtomicUsize = AtomicUsize::new(0);
static WAKE_ONE: AtomicUsize = AtomicUsize::new(0);
static WAKE_ALL: AtomicUsize = AtomicUsize::new(0);
static TIMED_OUT: AtomicUsize = AtomicUsize::new(0);
static SWEPT: AtomicUsize = AtomicUsize::new(0);

pub fn init() -> Result<(), &'static str> {
    crate::tick::register(expire_due)
}

pub fn create_domain(owner: u64) -> Result<DomainRef, Error> {
    without_interrupts(|| unsafe {
        let domains = &mut *DOMAINS.get();
        let Some((index, old)) = domains
            .iter()
            .copied()
            .enumerate()
            .find(|(_, domain)| !domain.live && domain.generation < u32::MAX)
        else {
            return Err(Error::Quota);
        };
        let generation = old.generation + 1;
        domains[index] = Domain {
            live: true,
            generation,
            owner,
            keys: 0,
            keys_high: 0,
            waiters_high: 0,
        };
        Ok(DomainRef {
            id: index as u32,
            generation,
        })
    })
}

pub fn create_key(domain_ref: DomainRef, owner: u64) -> Result<u64, Error> {
    without_interrupts(|| unsafe {
        let domains = &mut *DOMAINS.get();
        let Some(domain) = domains
            .get_mut(domain_ref.id as usize)
            .filter(|domain| domain.live && domain.generation == domain_ref.generation)
        else {
            return Err(Error::BadArgument);
        };
        if usize::from(domain.keys) >= MAX_KEYS_PER_DOMAIN {
            return Err(Error::Quota);
        }
        let keys = &mut *KEYS.get();
        let Some((slot, old)) = keys
            .iter()
            .copied()
            .enumerate()
            .find(|(_, key)| !key.live && key.generation < MAX_KEY_GENERATION)
        else {
            return Err(Error::Quota);
        };
        let generation = old.generation + 1;
        keys[slot] = Key {
            live: true,
            domain: domain_ref.id,
            domain_generation: domain_ref.generation,
            owner,
            generation,
            sequence: 1,
        };
        domain.keys += 1;
        domain.keys_high = domain.keys_high.max(domain.keys);
        Ok((generation << KEY_INDEX_BITS) | slot as u64)
    })
}

fn decode_key(token: u64) -> Result<(usize, u64), Error> {
    let slot = (token & KEY_INDEX_MASK) as usize;
    let generation = token >> KEY_INDEX_BITS;
    if token == 0 || slot >= MAX_KEYS || generation == 0 {
        return Err(Error::BadArgument);
    }
    Ok((slot, generation))
}

fn key_ref<'a>(
    keys: &'a mut [Key; MAX_KEYS],
    domain_ref: DomainRef,
    token: u64,
) -> Result<(usize, &'a mut Key), Error> {
    let (slot, generation) = decode_key(token)?;
    let key = keys.get_mut(slot).ok_or(Error::BadArgument)?;
    if !key.live
        || key.generation != generation
        || key.domain != domain_ref.id
        || key.domain_generation != domain_ref.generation
    {
        return Err(Error::BadArgument);
    }
    Ok((slot, key))
}

pub fn destroy_key(owner: u64, domain_ref: DomainRef, token: u64) -> Result<(), Error> {
    without_interrupts(|| unsafe {
        let keys = &mut *KEYS.get();
        let (slot, key) = key_ref(keys, domain_ref, token)?;
        if key.owner != owner {
            return Err(Error::BadArgument);
        }
        if (*WAITERS.get()).iter().any(|waiter| {
            waiter.state == WaitState::Parked
                && usize::from(waiter.key_slot) == slot
                && waiter.key_generation == key.generation
                && waiter.domain == domain_ref.id
                && waiter.domain_generation == domain_ref.generation
        }) {
            return Err(Error::Busy);
        }
        let generation = key.generation;
        *key = EMPTY_KEY;
        key.generation = generation;
        let domain = &mut (*DOMAINS.get())[domain_ref.id as usize];
        domain.keys -= 1;
        Ok(())
    })
}

pub fn sequence(domain_ref: DomainRef, token: u64) -> Result<u64, Error> {
    without_interrupts(|| unsafe {
        let keys = &mut *KEYS.get();
        let (_, key) = key_ref(keys, domain_ref, token)?;
        Ok(key.sequence)
    })
}

pub fn wait(
    pid: u64,
    domain_ref: DomainRef,
    token: u64,
    observed: u64,
    timeout_us: u64,
) -> Result<(), Error> {
    // SYS_SYNC_WAIT is entered with IF cleared by the syscall SFMASK. Keep
    // this contract explicit: the sequence comparison, waiter insertion,
    // and scheduler park must not admit a timer preemption between them.
    if crate::sync::interrupts_enabled() {
        return Err(Error::Busy);
    }
    if timeout_us > MAX_TIMEOUT_US {
        return Err(Error::BadArgument);
    }
    let now = crate::timekeeping::now_us();
    let deadline_us = if timeout_us == 0 {
        0
    } else {
        now.checked_add(timeout_us).ok_or(Error::BadArgument)?
    };
    let tid = sched::current_thread_id();
    let prepare = without_interrupts(|| unsafe {
        let keys = &mut *KEYS.get();
        let (key_slot, key) = key_ref(keys, domain_ref, token)?;
        if key.sequence != observed {
            return Ok(None);
        }
        let waiters = &mut *WAITERS.get();
        if waiters
            .iter()
            .any(|waiter| waiter.state != WaitState::Empty && waiter.tid == tid)
        {
            return Err(Error::Busy);
        }
        let Some((waiter_slot, _)) = waiters
            .iter()
            .copied()
            .enumerate()
            .find(|(_, waiter)| waiter.state == WaitState::Empty)
        else {
            return Err(Error::Quota);
        };
        waiters[waiter_slot] = Waiter {
            state: WaitState::Parked,
            tid,
            pid,
            domain: domain_ref.id,
            domain_generation: domain_ref.generation,
            key_slot: key_slot as u16,
            key_generation: key.generation,
            deadline_us,
        };
        let domain = &mut (*DOMAINS.get())[domain_ref.id as usize];
        let parked = waiters
            .iter()
            .filter(|waiter| {
                waiter.state == WaitState::Parked
                    && waiter.domain == domain_ref.id
                    && waiter.domain_generation == domain_ref.generation
            })
            .count();
        domain.waiters_high = domain.waiters_high.max(parked as u8);
        PARKED.fetch_add(1, Ordering::Relaxed);
        Ok(Some(waiter_slot))
    })?;
    let Some(waiter_slot) = prepare else {
        return Ok(());
    };
    if !sched::try_block_current() {
        without_interrupts(|| unsafe {
            let waiter = &mut (*WAITERS.get())[waiter_slot];
            if waiter.state == WaitState::Parked && waiter.tid == tid {
                *waiter = EMPTY_WAITER;
                PARKED.fetch_sub(1, Ordering::Relaxed);
            }
        });
        return Err(Error::Busy);
    }
    without_interrupts(|| unsafe {
        let waiter = &mut (*WAITERS.get())[waiter_slot];
        if waiter.tid != tid {
            return Err(Error::BadArgument);
        }
        let result = match waiter.state {
            WaitState::Notified => Ok(()),
            WaitState::TimedOut => Err(Error::TimedOut),
            WaitState::DomainGone => Err(Error::DomainGone),
            WaitState::KeyGone => Err(Error::KeyGone),
            WaitState::Parked | WaitState::Empty => {
                return Err(Error::Busy);
            }
        };
        *waiter = EMPTY_WAITER;
        result
    })
}

pub fn wake(domain_ref: DomainRef, token: u64, all: bool) -> Result<u64, Error> {
    let tids = without_interrupts(|| unsafe {
        let keys = &mut *KEYS.get();
        let (key_slot, key) = key_ref(keys, domain_ref, token)?;
        let Some(sequence) = key
            .sequence
            .checked_add(1)
            .filter(|next| *next <= i64::MAX as u64)
        else {
            return Err(Error::Quota);
        };
        key.sequence = sequence;
        let waiters = &mut *WAITERS.get();
        let mut tids = [0u64; MAX_WAITERS];
        let mut count = 0usize;
        for waiter in waiters.iter_mut() {
            if waiter.state == WaitState::Parked
                && waiter.domain == domain_ref.id
                && waiter.domain_generation == domain_ref.generation
                && usize::from(waiter.key_slot) == key_slot
                && waiter.key_generation == key.generation
            {
                waiter.state = WaitState::Notified;
                waiter.deadline_us = 0;
                tids[count] = waiter.tid;
                count += 1;
                PARKED.fetch_sub(1, Ordering::Relaxed);
                if !all {
                    break;
                }
            }
        }
        if all {
            WAKE_ALL.fetch_add(count, Ordering::Relaxed);
        } else {
            WAKE_ONE.fetch_add(count, Ordering::Relaxed);
        }
        Ok((tids, count))
    })?;
    for tid in &tids.0[..tids.1] {
        if sched::wake(*tid).is_err() {
            error!("sync", "wake: scheduler refused parked thread {tid}");
            crate::halt::halt_machine("native sync waiter wake invariant failed");
        }
    }
    Ok(tids.1 as u64)
}

pub fn info(domain_ref: DomainRef) -> Result<DomainInfo, Error> {
    without_interrupts(|| unsafe {
        let Some(domain) = (*DOMAINS.get())
            .get(domain_ref.id as usize)
            .filter(|domain| domain.live && domain.generation == domain_ref.generation)
        else {
            return Err(Error::BadArgument);
        };
        let parked_waiters = (*WAITERS.get())
            .iter()
            .filter(|waiter| {
                waiter.state == WaitState::Parked
                    && waiter.domain == domain_ref.id
                    && waiter.domain_generation == domain_ref.generation
            })
            .count();
        Ok(DomainInfo {
            keys: u64::from(domain.keys),
            keys_high: u64::from(domain.keys_high),
            parked_waiters: parked_waiters as u64,
            waiters_high: u64::from(domain.waiters_high),
        })
    })
}

pub fn destroy_domain(owner: u64, domain_ref: DomainRef) -> Result<(), Error> {
    let tids = without_interrupts(|| unsafe {
        let domains = &mut *DOMAINS.get();
        let Some(domain) = domains.get(domain_ref.id as usize).filter(|domain| {
            domain.live && domain.generation == domain_ref.generation && domain.owner == owner
        }) else {
            return Err(Error::BadArgument);
        };
        let _ = domain;
        Ok(retire_domain_locked(domain_ref, WaitState::DomainGone))
    })?;
    wake_tids(&tids);
    Ok(())
}

/// Remove every wait owned by a dying Process before its threads are killed.
pub fn release_waiters_by_process(pid: u64) -> usize {
    without_interrupts(|| unsafe {
        let waiters = &mut *WAITERS.get();
        let mut n = 0usize;
        for waiter in waiters.iter_mut() {
            if waiter.state != WaitState::Empty && waiter.pid == pid {
                if waiter.state == WaitState::Parked {
                    PARKED.fetch_sub(1, Ordering::Relaxed);
                }
                *waiter = EMPTY_WAITER;
                n += 1;
            }
        }
        SWEPT.fetch_add(n, Ordering::Relaxed);
        n
    })
}

/// Retire keys created by a dying Process. A helper can share an AppInstance
/// domain, so domain teardown alone is too late to reclaim its private keys.
/// Any other Process parked on one of those keys receives a typed wake.
pub fn release_keys_by_process(pid: u64) -> (usize, usize) {
    let (keys_released, tids) = without_interrupts(|| unsafe {
        let keys = &mut *KEYS.get();
        let waiters = &mut *WAITERS.get();
        let mut tids = [0u64; MAX_WAITERS];
        let mut wake_count = 0usize;
        let mut keys_released = 0usize;
        for (key_slot, key) in keys.iter_mut().enumerate() {
            if !key.live || key.owner != pid {
                continue;
            }
            for waiter in waiters.iter_mut() {
                if waiter.state == WaitState::Parked
                    && usize::from(waiter.key_slot) == key_slot
                    && waiter.key_generation == key.generation
                    && waiter.domain == key.domain
                    && waiter.domain_generation == key.domain_generation
                {
                    waiter.state = WaitState::KeyGone;
                    waiter.deadline_us = 0;
                    tids[wake_count] = waiter.tid;
                    wake_count += 1;
                    PARKED.fetch_sub(1, Ordering::Relaxed);
                }
            }
            if let Some(domain) = (*DOMAINS.get()).get_mut(key.domain as usize)
                && domain.live
                && domain.generation == key.domain_generation
            {
                domain.keys -= 1;
            }
            let generation = key.generation;
            *key = EMPTY_KEY;
            key.generation = generation;
            keys_released += 1;
        }
        SWEPT.fetch_add(wake_count, Ordering::Relaxed);
        (keys_released, (tids, wake_count))
    });
    wake_tids(&tids);
    (keys_released, tids.1)
}

/// Invalidate every domain created by `owner` and cancel delegated waiters.
pub fn release_by_owner(owner: u64) -> usize {
    let mut total = 0usize;
    let refs = without_interrupts(|| unsafe {
        (*DOMAINS.get())
            .iter()
            .enumerate()
            .filter_map(|(index, domain)| {
                (domain.live && domain.owner == owner).then_some(DomainRef {
                    id: index as u32,
                    generation: domain.generation,
                })
            })
            .collect::<FixedDomains>()
    });
    for domain_ref in refs.iter() {
        let tids = without_interrupts(|| retire_domain_locked(*domain_ref, WaitState::DomainGone));
        total += tids.1;
        wake_tids(&tids);
    }
    total
}

struct FixedDomains {
    entries: [DomainRef; MAX_DOMAINS],
    count: usize,
}

impl FixedDomains {
    fn iter(&self) -> impl Iterator<Item = &DomainRef> {
        self.entries[..self.count].iter()
    }
}

impl FromIterator<DomainRef> for FixedDomains {
    fn from_iter<T: IntoIterator<Item = DomainRef>>(iter: T) -> Self {
        let mut entries = [DomainRef {
            id: 0,
            generation: 0,
        }; MAX_DOMAINS];
        let mut count = 0;
        for item in iter {
            if count == MAX_DOMAINS {
                break;
            }
            entries[count] = item;
            count += 1;
        }
        Self { entries, count }
    }
}

fn retire_domain_locked(domain_ref: DomainRef, result: WaitState) -> ([u64; MAX_WAITERS], usize) {
    let mut tids = [0u64; MAX_WAITERS];
    let mut count = 0usize;
    unsafe {
        let waiters = &mut *WAITERS.get();
        for waiter in waiters.iter_mut() {
            if waiter.state == WaitState::Parked
                && waiter.domain == domain_ref.id
                && waiter.domain_generation == domain_ref.generation
            {
                waiter.state = result;
                waiter.deadline_us = 0;
                tids[count] = waiter.tid;
                count += 1;
                PARKED.fetch_sub(1, Ordering::Relaxed);
            }
        }
        let keys = &mut *KEYS.get();
        for key in keys.iter_mut() {
            if key.live
                && key.domain == domain_ref.id
                && key.domain_generation == domain_ref.generation
            {
                let generation = key.generation;
                *key = EMPTY_KEY;
                key.generation = generation;
            }
        }
        let domains = &mut *DOMAINS.get();
        if let Some(domain) = domains.get_mut(domain_ref.id as usize) {
            if domain.live && domain.generation == domain_ref.generation {
                let generation = domain.generation;
                *domain = EMPTY_DOMAIN;
                domain.generation = generation;
            }
        }
        SWEPT.fetch_add(count, Ordering::Relaxed);
    }
    (tids, count)
}

fn wake_tids(tids: &([u64; MAX_WAITERS], usize)) {
    for tid in &tids.0[..tids.1] {
        if sched::wake(*tid).is_err() {
            error!("sync", "domain cleanup could not wake parked thread {tid}");
            crate::halt::halt_machine("native sync domain cleanup wake invariant failed");
        }
    }
}

extern "C" fn expire_due() {
    if PARKED.load(Ordering::Relaxed) == 0 {
        return;
    }
    let now = crate::timekeeping::now_us();
    let tids = without_interrupts(|| unsafe {
        let waiters = &mut *WAITERS.get();
        let mut tids = [0u64; MAX_WAITERS];
        let mut count = 0usize;
        for waiter in waiters.iter_mut() {
            if waiter.state == WaitState::Parked
                && waiter.deadline_us != 0
                && now >= waiter.deadline_us
            {
                waiter.state = WaitState::TimedOut;
                waiter.deadline_us = 0;
                tids[count] = waiter.tid;
                count += 1;
                PARKED.fetch_sub(1, Ordering::Relaxed);
                TIMED_OUT.fetch_add(1, Ordering::Relaxed);
            }
        }
        (tids, count)
    });
    wake_tids(&tids);
}
