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

/// Per-process bound inside the shared table (Phase 11.0, ADR-0071).
/// Measured: no production or test process holds more than three armed
/// timers at once (timertest's merge proof); four leaves one spare while
/// guaranteeing that any eight processes can still arm a timer each, so
/// no single notification holder can exhaust the system's timers.
pub const MAX_TIMERS_PER_PROCESS: usize = 4;

/// Why arming refused. `Quota` is distinct from a full table so a caller
/// can tell "you hold too many" from "the system is out".
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArmError {
    BadBadge,
    Uncalibrated,
    Full,
    Quota,
}

#[derive(Clone, Copy)]
struct Timer {
    live: bool,
    /// Bumped every time this slot is handed out. The other half of a
    /// timer id, and the reason a stale id is harmless — see
    /// [`make_id`].
    generation: u32,
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
    generation: 0,
    owner: 0,
    nid: 0,
    badge: 0,
    deadline_us: 0,
};

static TIMERS: SyncCell<[Timer; MAX_TIMERS]> = SyncCell::new([EMPTY; MAX_TIMERS]);

/// A timer id is `(generation << 32) | slot`, not a bare slot index.
///
/// The bare index had an ABA hole that ownership checks do not close,
/// and the place it would have bitten is the worst one: a TCP stack
/// holds dozens of retransmission timers in ONE process, so "same
/// owner" is no protection at all. Timer A fires and frees slot 3;
/// slot 3 is handed to timer B; a cancel for A — perfectly reasonable
/// bookkeeping in a protocol that cancels an RTO after a late ACK —
/// silently kills B instead. That is a lost retransmission, appearing
/// only under the interleaving that produced it.
///
/// With a generation, a stale id names a slot whose generation has
/// moved on, and `cancel` refuses it as "not armed" — which is the
/// truthful answer and one the caller already has to handle, because
/// a timer that has already fired gives the same one.
///
/// The generation is masked to 31 bits so an id is always a positive
/// `Status` (the syscall returns it), and wraps after 2^31 reuses of a
/// single slot — at the 100 Hz tick, several centuries.
fn make_id(generation: u32, slot: usize) -> u64 {
    ((generation as u64 & 0x7FFF_FFFF) << 32) | slot as u64
}

fn split_id(id: u64) -> (u32, usize) {
    (
        ((id >> 32) & 0x7FFF_FFFF) as u32,
        (id & 0xFFFF_FFFF) as usize,
    )
}

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
    /// Arms refused because the owner already held its quota.
    pub quota_refused: u64,
    /// Largest number of timers one process has held at once.
    pub per_process_high_water: usize,
}

static STATS: SyncCell<TimerStats> = SyncCell::new(TimerStats {
    armed_now: 0,
    armed_total: 0,
    fired: 0,
    cancelled: 0,
    swept: 0,
    quota_refused: 0,
    per_process_high_water: 0,
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
pub fn arm(owner: u64, nid: u32, badge: u64, delay_us: u64) -> Result<u64, ArmError> {
    if badge == 0 {
        // The merged-badge protocol has no empty word.
        return Err(ArmError::BadBadge);
    }
    let now = timekeeping::now_us();
    if now == 0 {
        return Err(ArmError::Uncalibrated);
    }
    without_interrupts(|| {
        // SAFETY: single writer under IF=0.
        unsafe {
            let timers = &mut *TIMERS.get();
            // Quota before capacity: an over-budget process is refused
            // even when slots are free, and its refusal is its own.
            let held = timers.iter().filter(|t| t.live && t.owner == owner).count();
            if held >= MAX_TIMERS_PER_PROCESS {
                (*STATS.get()).quota_refused += 1;
                return Err(ArmError::Quota);
            }
            let Some(i) = timers.iter().position(|t| !t.live) else {
                return Err(ArmError::Full);
            };
            // The generation belongs to the SLOT and only ever moves
            // forward, so ids handed out for it are never reused.
            let generation = timers[i].generation.wrapping_add(1);
            timers[i] = Timer {
                live: true,
                generation,
                owner,
                nid,
                badge,
                deadline_us: now.saturating_add(delay_us),
            };
            ARMED.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
            let stats = &mut *STATS.get();
            stats.armed_total += 1;
            stats.per_process_high_water = stats.per_process_high_water.max(held + 1);
            Ok(make_id(generation, i))
        }
    })
}

/// Timers `owner` currently holds (the quota's accounting, derived from
/// the table itself, so it cannot drift from the truth or outlive a slot).
pub fn held_by(owner: u64) -> usize {
    without_interrupts(|| {
        // SAFETY: single reader under IF=0.
        unsafe {
            (*TIMERS.get())
                .iter()
                .filter(|t| t.live && t.owner == owner)
                .count()
        }
    })
}

/// Cancel a timer this process armed. Cancelling one that has already
/// fired (or never existed) is an error, not a silent success: a
/// protocol that cancels a retransmission which has in fact already
/// gone out needs to know the difference.
pub fn cancel(owner: u64, id: u64) -> Result<(), &'static str> {
    let (generation, slot) = split_id(id);
    without_interrupts(|| {
        // SAFETY: single writer under IF=0.
        unsafe {
            let timers = &mut *TIMERS.get();
            let Some(t) = timers.get_mut(slot) else {
                return Err("timer: id out of range");
            };
            if !t.live {
                return Err("timer: not armed (already fired or cancelled)");
            }
            // A STALE id: this slot has been handed out again since
            // that id was issued, so the timer it names is long gone
            // and the one sitting here belongs to somebody else's
            // bookkeeping. Refusing is the truthful answer.
            if t.generation != generation {
                return Err("timer: stale id (the slot was reused by a later timer)");
            }
            if t.owner != owner {
                return Err("timer: not yours");
            }
            // Bump on free as well as on arm, so an id cancelled here
            // cannot be replayed against the next occupant either.
            let next_generation = t.generation;
            *t = EMPTY;
            t.generation = next_generation;
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
                    let generation = t.generation;
                    *t = EMPTY;
                    // Generations survive the slot being emptied:
                    // they are what makes an old id stale rather than
                    // ambiguous.
                    t.generation = generation;
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
                    let generation = t.generation;
                    *t = EMPTY;
                    t.generation = generation;
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
