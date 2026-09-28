//! Timers for ring 3 (M7.0, ADR-0029): the facility the network stack
//! has to have before it has protocols.
//!
//! Every protocol Phase 7 will build needs to know that something did
//! NOT happen within some time: TCP retransmission, ARP aging, DNS
//! timeout and retry, connection establishment, and later TIME_WAIT.
//! ADR-0028 recorded the same gap from the other side — an IPC client
//! cannot currently give up on a service that has stopped answering.
//! Building protocols first and discovering this later is how a
//! codebase ends up with polling loops nobody ever removes, so this is
//! built first and the loops are forbidden.
//!
//! **A timer is a notification, not a signal.** `SYS_TIMER_ARM` takes
//! a notification the caller already holds and a badge bit, and
//! delivers that badge when the deadline passes. Nothing new to block
//! on: every service in ArenaOS already parks in `SYS_WAIT` on one
//! notification with merged badges, so "wait for a device interrupt OR
//! a client request OR a timeout" becomes an ordinary wait with one
//! more bit set. No second thread, no new blocking primitive, and no
//! way for a timeout to be missed by a service that happened to be
//! waiting on something else.
//!
//! The capability gate is the notification itself: arming requires
//! WRITE on it, exactly as `SYS_IRQ_RELAY` does. A process can only
//! aim a timer at a notification it was already trusted with, so no
//! new capability kind is needed to make timers safe.
//!
//! **Resolution is honest.** Timers are checked on the 100 Hz tick, so
//! a deadline means "not before", with up to one tick (10 ms) of lag
//! and no claim of precision beyond that. That is ample for RTO
//! minimums, ARP aging and DNS retry; nothing in Phase 7 may quietly
//! assume finer. A caller that needs to know how long actually passed
//! reads `SYS_CLOCK_NOW`, which is the M2.2 monotonic clock and is
//! microsecond-true.
//!
//! **One-shot only.** Periodic timers are re-armed by their owner.
//! That keeps the kernel side a fixed table with no drift policy to
//! get wrong, and a protocol that wants a repeating tick is a protocol
//! that wants to decide the next interval anyway (a back-off is not a
//! period).
//!
//! Timers are owned by the process that armed them and swept when it
//! dies — the same sweep relay vectors (M5.2), the console mirror
//! (M6.4) and blocked-thread references (M6.5) already get.

use crate::ipc;
use crate::log::log_info as info;
use crate::sync::{SyncCell, without_interrupts};
use crate::timekeeping;

/// Armed timers the system can hold at once. Generous for Phase 7 (a
/// TCP connection needs a couple, ARP a handful); the bound exists so
/// the table is a plain array scanned on the tick.
pub const MAX_TIMERS: usize = 32;

#[derive(Clone, Copy)]
struct Timer {
    live: bool,
    /// The process that armed it — the unit of ownership and sweeping.
    owner: u64,
    /// Notification to signal, and the badge bit to signal it with.
    nid: u32,
    badge: u64,
    /// Monotonic microseconds after which this fires.
    deadline_us: u64,
}

const EMPTY: Timer = Timer {
    live: false,
    owner: 0,
    nid: 0,
    badge: 0,
    deadline_us: 0,
};

static TIMERS: SyncCell<[Timer; MAX_TIMERS]> = SyncCell::new([EMPTY; MAX_TIMERS]);

/// How many timers are armed, as a plain atomic so the tick's fast
/// path costs one relaxed load when nothing is armed — which is every
/// tick of every boot until something arms one (the lesson M6.4's
/// console tap paid for: a facility nobody is using must be free).
static ARMED: core::sync::atomic::AtomicUsize = core::sync::atomic::AtomicUsize::new(0);

#[derive(Clone, Copy, Default)]
pub struct TimerStats {
    pub armed_now: usize,
    pub armed_total: u64,
    pub fired: u64,
    pub cancelled: u64,
    pub swept: u64,
}

static STATS: SyncCell<TimerStats> = SyncCell::new(TimerStats {
    armed_now: 0,
    armed_total: 0,
    fired: 0,
    cancelled: 0,
    swept: 0,
});

pub fn stats() -> TimerStats {
    without_interrupts(|| {
        // SAFETY: single reader under IF=0.
        let mut s = unsafe { *STATS.get() };
        s.armed_now = ARMED.load(core::sync::atomic::Ordering::Relaxed);
        s
    })
}

/// Install the tick task. Call once, during boot.
pub fn init() -> Result<(), &'static str> {
    crate::tick::register(expire_due)
}

/// Arm a one-shot timer: deliver `badge` on notification `nid` once
/// `delay_us` of monotonic time has passed. Returns the timer id.
///
/// The delay is RELATIVE on purpose. An absolute deadline makes the
/// caller read the clock, do arithmetic, and then race whatever
/// happens between the read and the call; "in 200 ms" cannot be stale
/// by construction.
pub fn arm(owner: u64, nid: u32, badge: u64, delay_us: u64) -> Result<u32, &'static str> {
    if badge == 0 {
        return Err("timer: badge must be nonzero (the merged-badge protocol has no empty word)");
    }
    let now = timekeeping::now_us();
    if now == 0 {
        return Err("timer: the monotonic clock is not calibrated");
    }
    without_interrupts(|| {
        // SAFETY: single writer under IF=0.
        unsafe {
            let timers = &mut *TIMERS.get();
            let Some(i) = timers.iter().position(|t| !t.live) else {
                return Err("timer: table full (MAX_TIMERS)");
            };
            timers[i] = Timer {
                live: true,
                owner,
                nid,
                badge,
                deadline_us: now.saturating_add(delay_us),
            };
            ARMED.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
            (*STATS.get()).armed_total += 1;
            Ok(i as u32)
        }
    })
}

/// Cancel a timer this process armed. Cancelling one that has already
/// fired (or never existed) is an error, not a silent success: a
/// protocol that cancels a retransmission which has in fact already
/// gone out needs to know the difference.
pub fn cancel(owner: u64, id: u32) -> Result<(), &'static str> {
    without_interrupts(|| {
        // SAFETY: single writer under IF=0.
        unsafe {
            let timers = &mut *TIMERS.get();
            let Some(t) = timers.get_mut(id as usize) else {
                return Err("timer: id out of range");
            };
            if !t.live {
                return Err("timer: not armed (already fired or cancelled)");
            }
            if t.owner != owner {
                return Err("timer: not yours");
            }
            *t = EMPTY;
            ARMED.fetch_sub(1, core::sync::atomic::Ordering::Relaxed);
            (*STATS.get()).cancelled += 1;
            Ok(())
        }
    })
}

/// Drop every timer armed by `pid` — called from `proc::destroy`, so a
/// dead process cannot keep signalling a notification that may itself
/// be gone. Returns how many were swept.
pub fn release_by_owner(pid: u64) -> usize {
    without_interrupts(|| {
        // SAFETY: single writer under IF=0.
        unsafe {
            let timers = &mut *TIMERS.get();
            let mut n = 0;
            for t in timers.iter_mut() {
                if t.live && t.owner == pid {
                    *t = EMPTY;
                    n += 1;
                }
            }
            if n > 0 {
                ARMED.fetch_sub(n, core::sync::atomic::Ordering::Relaxed);
                (*STATS.get()).swept += n as u64;
            }
            n
        }
    })
}

/// The tick task: fire every timer whose deadline has passed.
///
/// Collect under IF=0, notify outside it — the same shape
/// `console::mirror_flush` uses, and for the same reason: `notify`
/// wakes threads, and waking them with the timer table borrowed would
/// be an aliasing hazard the moment a woken thread arms another timer.
extern "C" fn expire_due() {
    // The fast path: nothing armed, one relaxed load, 100 times a
    // second forever.
    if ARMED.load(core::sync::atomic::Ordering::Relaxed) == 0 {
        return;
    }
    let now = timekeeping::now_us();
    let mut due: [(u32, u64); MAX_TIMERS] = [(0, 0); MAX_TIMERS];
    let mut n = 0usize;
    without_interrupts(|| {
        // SAFETY: single writer under IF=0 (interrupt context).
        unsafe {
            let timers = &mut *TIMERS.get();
            for t in timers.iter_mut() {
                if t.live && now >= t.deadline_us {
                    due[n] = (t.nid, t.badge);
                    n += 1;
                    *t = EMPTY;
                }
            }
            if n > 0 {
                ARMED.fetch_sub(n, core::sync::atomic::Ordering::Relaxed);
                (*STATS.get()).fired += n as u64;
            }
        }
    });
    for &(nid, badge) in &due[..n] {
        // A dead notification is not worth halting for: the owner is
        // being torn down and the sweep is about to remove this timer
        // anyway.
        let _ = ipc::notify(nid, badge);
    }
}

/// Boot-time evidence that the facility is installed and idle.
pub fn log_ready() {
    info!(
        "timer",
        "timer facility ready: {MAX_TIMERS} slots, 100 Hz tick (a deadline means NOT BEFORE, ~10 ms lag), delivery by notification badge"
    );
}
