//! IPC v1 (M4.4, ADR-0018): endpoints, synchronous call/reply, badged
//! merged notifications, and capability transfer inside messages.
//!
//! The object layer only — no user pointers cross this module. The
//! syscall handlers (`arch::x86_64::syscall`) own ABI validation and
//! every STAC-bracketed user-memory touch, in the *owner thread's*
//! context; this module owns the objects, the bounded queues, the
//! blocking discipline, and the cap-space side of transfers.
//!
//! Discipline (ADR-0012, extended here): **no borrow of an IPC static
//! survives a scheduler call.** Every operation runs in short IF=0
//! phases — mutate/collect under a borrow, end the borrow, then
//! `wake`/`block_current`/`cap::grant` on raw facts. A blocked thread's
//! dangling `&mut` into a static that the waker also mutates would be
//! aliasing UB even on one CPU with interrupts off.
//!
//! Rendezvous state machine per queue slot:
//!
//! ```text
//! Empty ──call──▶ Waiting ──deliver──▶ Delivered ──reply──▶ Replied ──caller resumes──▶ Empty
//! ```
//!
//! Delivery happens in the *caller's* context when a server is parked
//! in `recv` (call → take_request → wake), or in the *server's* context
//! when the request was already queued (recv → take_request, no wake —
//! the server is the running thread). Either way the transferred cap is
//! installed into the server's space with `cap::grant` (first free
//! slot; a full space drops the cap and counts it — v1 caps describe,
//! never own, so dropping is safe; ADR-0018).

use crate::arch::x86_64::syscall::{STATUS_BAD_ARG, STATUS_BUSY, Status};
use crate::cap::{Cap, CapObj};
use crate::log::log_error as error;
use crate::sched;
use crate::sync::SyncCell;
use crate::sync::without_interrupts;

pub const MAX_ENDPOINTS: usize = 8;
pub const MAX_NOTIFS: usize = 8;
/// Bounded caller queue per endpoint — a full queue answers
/// `STATUS_BUSY`, never a silent drop (ADR-0018).
const QUEUE_DEPTH: usize = 4;

/// The "no capability" marker in message buffers (ADR-0018): a cap word
/// holds either a landing slot index (< `CAP_SLOTS`) or this.
pub const CAP_NONE: u64 = u64::MAX;
/// Sentinel for "no thread" — thread ids are small; MAX is unambiguous.
const NO_TID: u64 = u64::MAX;

#[derive(Clone, Copy, PartialEq, Eq)]
enum SlotState {
    Empty,
    /// A caller is blocked; no server has taken the request yet.
    Waiting,
    /// Handed to `server`; the reply has not been staged yet.
    Delivered,
    /// Reply staged; the caller is being (or has been) woken.
    Replied,
}

#[derive(Clone, Copy)]
struct CallSlot {
    state: SlotState,
    /// Blocked caller's thread id (drives the reply wake).
    caller: u64,
    words: [u64; 2],
    /// Staged at `call`, moved into the server's space at delivery
    /// (then `Cap::EMPTY` again).
    send_cap: Cap,
    /// The server thread the request was delivered to.
    server: u64,
    /// Where `send_cap` landed in the server's space (`CAP_NONE` =
    /// none sent, or dropped into a full space).
    landed_cap: u64,
    reply_words: [u64; 2],
    /// Staged by `reply`, installed into the caller's space when the
    /// caller resumes (owner-context discipline).
    reply_cap: Cap,
}

const EMPTY_SLOT: CallSlot = CallSlot {
    state: SlotState::Empty,
    caller: 0,
    words: [0; 2],
    send_cap: Cap::EMPTY,
    server: NO_TID,
    landed_cap: CAP_NONE,
    reply_words: [0; 2],
    reply_cap: Cap::EMPTY,
};

#[derive(Clone, Copy)]
struct Endpoint {
    live: bool,
    q: [CallSlot; QUEUE_DEPTH],
    /// Thread parked in `recv` (`NO_TID` = none). v1: one server per
    /// endpoint — a second `recv` gets `STATUS_BUSY` (ADR-0018).
    server: u64,
}

const EMPTY_EP: Endpoint = Endpoint {
    live: false,
    q: [EMPTY_SLOT; QUEUE_DEPTH],
    server: NO_TID,
};

#[derive(Clone, Copy)]
struct Notif {
    live: bool,
    /// Merged badge word: `notify` ORs into it, `wait` takes and clears.
    pending: u64,
    /// Thread parked in `wait` (`NO_TID` = none). v1: one waiter —
    /// a second `wait` gets `STATUS_BUSY`.
    waiter: u64,
}

const EMPTY_NOTIF: Notif = Notif {
    live: false,
    pending: 0,
    waiter: NO_TID,
};

/// Dispatcher-visible IPC accounting — the test side asserts deltas, so
/// every claim about the demo is a counted machine event.
#[derive(Clone, Copy)]
pub struct IpcStats {
    pub calls: u64,
    pub recvs: u64,
    pub replies: u64,
    pub notifies: u64,
    pub waits: u64,
    /// Caps actually installed into a receiving space.
    pub cap_transfers: u64,
    /// Caps dropped because the receiving space was full.
    pub cap_drops: u64,
    /// `block_current` calls from IPC paths (call/recv/wait parking).
    pub blocks: u64,
}

static STATS: SyncCell<IpcStats> = SyncCell::new(IpcStats {
    calls: 0,
    recvs: 0,
    replies: 0,
    notifies: 0,
    waits: 0,
    cap_transfers: 0,
    cap_drops: 0,
    blocks: 0,
});
static ENDPOINTS: SyncCell<[Endpoint; MAX_ENDPOINTS]> = SyncCell::new([EMPTY_EP; MAX_ENDPOINTS]);
static NOTIFS: SyncCell<[Notif; MAX_NOTIFS]> = SyncCell::new([EMPTY_NOTIF; MAX_NOTIFS]);

/// Snapshot of the IPC counters.
pub fn stats() -> IpcStats {
    without_interrupts(|| {
        // SAFETY: single reader under IF=0.
        unsafe { *STATS.get() }
    })
}

macro_rules! bump {
    ($field:ident) => {
        // SAFETY: single writer under IF=0 (SyncCell boot contract; the
        // whole kernel is single-CPU IF=0-disciplined until M5).
        unsafe {
            (*STATS.get()).$field += 1;
        }
    };
}

// ---- object lifecycle (kernel-side API in v1 — user-driven creation is
// ---- M4.5's root-task/spawn-protocol step, ADR-0018) --------------------

/// Mint a live endpoint; returns its `eid` (the cap's `CapObj::Endpoint`
/// index). Fails only when the fixed table is full.
pub fn create_endpoint() -> Result<u32, &'static str> {
    without_interrupts(|| {
        // SAFETY: single writer under IF=0.
        unsafe {
            let eps = &mut *ENDPOINTS.get();
            let Some(i) = eps.iter().position(|e| !e.live) else {
                return Err("endpoint table full (MAX_ENDPOINTS)");
            };
            eps[i] = Endpoint {
                live: true,
                q: [EMPTY_SLOT; QUEUE_DEPTH],
                server: NO_TID,
            };
            Ok(i as u32)
        }
    })
}

/// Retire an endpoint. Refuses while a server is parked or any request
/// is outstanding — destroying live rendezvous state would strand
/// blocked threads, which v1 answers with refusal, not corruption.
pub fn destroy_endpoint(eid: u32) -> Result<(), &'static str> {
    without_interrupts(|| {
        // SAFETY: single writer under IF=0.
        unsafe {
            let eps = &mut *ENDPOINTS.get();
            let Some(ep) = eps.get_mut(eid as usize).filter(|e| e.live) else {
                return Err("destroy_endpoint: no such live endpoint");
            };
            if ep.server != NO_TID || ep.q.iter().any(|s| s.state != SlotState::Empty) {
                return Err("destroy_endpoint: busy (server parked or requests outstanding)");
            }
            ep.live = false;
            Ok(())
        }
    })
}

/// Mint a live notification object; returns its `nid`.
pub fn create_notification() -> Result<u32, &'static str> {
    without_interrupts(|| {
        // SAFETY: single writer under IF=0.
        unsafe {
            let ns = &mut *NOTIFS.get();
            let Some(i) = ns.iter().position(|n| !n.live) else {
                return Err("notification table full (MAX_NOTIFS)");
            };
            ns[i] = EMPTY_NOTIF;
            ns[i].live = true;
            Ok(i as u32)
        }
    })
}

/// Retire a notification object. Refuses while a waiter is parked;
/// an unconsumed pending badge dies with the object (it is data, not a
/// resource — nothing to leak).
pub fn destroy_notification(nid: u32) -> Result<(), &'static str> {
    without_interrupts(|| {
        // SAFETY: single writer under IF=0.
        unsafe {
            let ns = &mut *NOTIFS.get();
            let Some(n) = ns.get_mut(nid as usize).filter(|n| n.live) else {
                return Err("destroy_notification: no such live notification");
            };
            if n.waiter != NO_TID {
                return Err("destroy_notification: busy (waiter parked)");
            }
            n.live = false;
            n.pending = 0;
            Ok(())
        }
    })
}

// ---- the cap-transfer helper ---------------------------------------------

/// Install `cap` into `pid`'s space (first free slot — ADR-0018), counting
/// the transfer or the drop. Returns the landing slot, or `CAP_NONE` when
/// the space was full (the cap is dropped: v1 caps describe, never own,
/// so a drop frees nothing and strands nothing).
fn install_cap(pid: u64, cap: Cap) -> u64 {
    match crate::cap::grant(pid, cap) {
        Ok(slot) => {
            bump!(cap_transfers);
            slot as u64
        }
        Err(_) => {
            bump!(cap_drops);
            CAP_NONE
        }
    }
}

/// Mark queue slot `(eidx, qi)` Delivered to `server_tid` and move its
/// staged send-cap into `server_pid`'s space, recording the landing
/// index. Returns the request words. Three short IF=0 phases — the
/// cap grant (which walks the process table) runs with the ENDPOINTS
/// borrow ended, per the module discipline.
fn take_request(eidx: usize, qi: usize, server_tid: u64, server_pid: u64) -> [u64; 2] {
    // SAFETY: single writer under IF=0; slot indices come from a
    // just-completed scan of the same table.
    let send_cap = without_interrupts(|| unsafe {
        let slot = &mut (*ENDPOINTS.get())[eidx].q[qi];
        slot.state = SlotState::Delivered;
        slot.server = server_tid;
        let c = slot.send_cap;
        slot.send_cap = Cap::EMPTY;
        c
    });
    let landed = if matches!(send_cap.obj, CapObj::None) {
        CAP_NONE
    } else {
        install_cap(server_pid, send_cap)
    };
    // SAFETY: as above.
    without_interrupts(|| unsafe {
        let slot = &mut (*ENDPOINTS.get())[eidx].q[qi];
        slot.landed_cap = landed;
        slot.words
    })
}

// ---- the five operations (ADR-0018) ---------------------------------------

/// Synchronous call: enqueue `[w0, w1]` (+ optional staged cap) on
/// endpoint `eid`, block until the server replies, and return the reply
/// words plus the landing slot of the reply cap (installed into the
/// CALLER's space here, in the caller's own resumed context —
/// `CAP_NONE` when the server sent none).
///
/// `pid` is the caller's process (its space supplied `send_cap` and
/// receives the reply cap). Errors: `STATUS_BAD_ARG` (dead eid),
/// `STATUS_BUSY` (queue full — the caller does NOT block).
pub fn call(
    pid: u64,
    eid: u32,
    words: [u64; 2],
    send_cap: Option<Cap>,
) -> Result<([u64; 2], u64), Status> {
    // Phase 1 — enqueue; note a parked server (borrow ends here).
    // SAFETY: single writer under IF=0.
    let (eidx, qi, parked) = without_interrupts(|| -> Result<(usize, usize, u64), Status> {
        bump!(calls);
        unsafe {
            let eps = &mut *ENDPOINTS.get();
            let eidx = eid as usize;
            let Some(ep) = eps.get_mut(eidx).filter(|e| e.live) else {
                return Err(STATUS_BAD_ARG);
            };
            let Some(qi) = ep.q.iter().position(|s| s.state == SlotState::Empty) else {
                return Err(STATUS_BUSY);
            };
            ep.q[qi] = CallSlot {
                state: SlotState::Waiting,
                caller: sched::current_thread_id(),
                words,
                send_cap: send_cap.unwrap_or(Cap::EMPTY),
                ..EMPTY_SLOT
            };
            let parked = ep.server;
            if parked != NO_TID {
                ep.server = NO_TID;
            }
            Ok((eidx, qi, parked))
        }
    })?;

    // Phase 2 — a parked server takes the request now (delivery in the
    // caller's context; ENDPOINTS borrow ended before grant/wake).
    if parked != NO_TID {
        let server_pid = sched::proc_id_of(parked).unwrap_or(0);
        if server_pid != 0 {
            take_request(eidx, qi, parked, server_pid);
        } else {
            // Unreachable in v1 (only a cap-holding process thread can
            // park in recv) — refuse to guess; the machine is wrong.
            error!("ipc", "call: parked server {} has no process", parked);
            crate::halt::halt_machine("ipc: server without a process");
        }
        if let Err(e) = sched::wake(parked) {
            error!("ipc", "call: wake(server {parked}) failed: {e}");
            crate::halt::halt_machine("ipc: delivered to a server that cannot wake");
        }
    }

    // Phase 3 — block until the reply stages and wakes us.
    bump!(blocks);
    sched::block_current();

    // Phase 4 — resumed in our own context: take the reply, free the
    // slot, install the reply cap into our own space.
    // SAFETY: single writer under IF=0.
    let (reply_words, reply_cap) = without_interrupts(|| unsafe {
        let slot = &mut (*ENDPOINTS.get())[eidx].q[qi];
        if slot.state != SlotState::Replied {
            error!(
                "ipc",
                "call: resumed without a staged reply (eid={eid} slot={qi})"
            );
            crate::halt::halt_machine("ipc: caller woken without a reply");
        }
        let w = slot.reply_words;
        let c = slot.reply_cap;
        *slot = EMPTY_SLOT;
        (w, c)
    });
    let landed = if matches!(reply_cap.obj, CapObj::None) {
        CAP_NONE
    } else {
        install_cap(pid, reply_cap)
    };
    Ok((reply_words, landed))
}

/// Server side: take the oldest request on endpoint `eid`, blocking
/// while the queue is empty. Returns the request words and the landing
/// slot of the transferred cap in THIS process's space (`CAP_NONE` when
/// none came). The slot stays Delivered until `reply`.
///
/// Errors: `STATUS_BAD_ARG` (dead eid), `STATUS_BUSY` (a second server
/// on one endpoint — v1 is single-server, ADR-0018).
pub fn recv(pid: u64, eid: u32) -> Result<([u64; 2], u64), Status> {
    // Phase 1 — either a Waiting request exists (self-deliver) or park.
    // SAFETY: single writer under IF=0.
    let queued = without_interrupts(|| -> Result<Option<usize>, Status> {
        bump!(recvs);
        unsafe {
            let eps = &mut *ENDPOINTS.get();
            let Some(ep) = eps.get_mut(eid as usize).filter(|e| e.live) else {
                return Err(STATUS_BAD_ARG);
            };
            if let Some(qi) = ep.q.iter().position(|s| s.state == SlotState::Waiting) {
                return Ok(Some(qi));
            }
            if ep.server != NO_TID {
                return Err(STATUS_BUSY);
            }
            ep.server = sched::current_thread_id();
            Ok(None)
        }
    })?;

    let tid = sched::current_thread_id();
    let qi = match queued {
        Some(qi) => {
            // A caller was already blocked: deliver to ourselves (no
            // wake — we ARE the server thread).
            take_request(eid as usize, qi, tid, pid);
            qi
        }
        None => {
            // Park; the wake comes from a future call's phase 2, which
            // has already run take_request by then.
            bump!(blocks);
            sched::block_current();
            // SAFETY: single reader under IF=0.
            let found = without_interrupts(|| unsafe {
                (*ENDPOINTS.get())[eid as usize]
                    .q
                    .iter()
                    .position(|s| s.state == SlotState::Delivered && s.server == tid)
            });
            let Some(qi) = found else {
                error!("ipc", "recv: resumed with no delivered request (eid={eid})");
                crate::halt::halt_machine("ipc: recv woken without a delivery");
            };
            qi
        }
    };

    // Phase 2 — copy the request facts out (the slot stays Delivered
    // until reply; the words are immutable after call staged them).
    // SAFETY: single reader under IF=0.
    let (words, landed) = without_interrupts(|| unsafe {
        let slot = &(*ENDPOINTS.get())[eid as usize].q[qi];
        (slot.words, slot.landed_cap)
    });
    Ok((words, landed))
}

/// Server side: stage the reply `[w0, w1]` (+ optional cap for the
/// caller's space) into this server's delivered slot and wake the
/// caller. Errors: `STATUS_BAD_ARG` (dead eid, or no request delivered
/// to this thread — a reply without a recv is a server bug, answered
/// with a typed status, not a fault).
pub fn reply(eid: u32, words: [u64; 2], send_cap: Option<Cap>) -> Result<(), Status> {
    // Phase 1 — find our Delivered slot, stage the reply (borrow ends).
    // SAFETY: single writer under IF=0.
    let caller = without_interrupts(|| -> Result<u64, Status> {
        bump!(replies);
        unsafe {
            let tid = sched::current_thread_id();
            let eps = &mut *ENDPOINTS.get();
            let Some(ep) = eps.get_mut(eid as usize).filter(|e| e.live) else {
                return Err(STATUS_BAD_ARG);
            };
            let Some(slot) =
                ep.q.iter_mut()
                    .find(|s| s.state == SlotState::Delivered && s.server == tid)
            else {
                return Err(STATUS_BAD_ARG);
            };
            slot.reply_words = words;
            slot.reply_cap = send_cap.unwrap_or(Cap::EMPTY);
            slot.state = SlotState::Replied;
            Ok(slot.caller)
        }
    })?;

    // Phase 2 — wake the caller (no borrow live).
    if let Err(e) = sched::wake(caller) {
        error!("ipc", "reply: wake(caller {caller}) failed: {e}");
        crate::halt::halt_machine("ipc: reply could not wake the caller");
    }
    Ok(())
}

/// Non-blocking: OR a nonzero `badge` into the notification's pending
/// word and wake a parked waiter (the badge STAYS pending — the woken
/// waiter consumes it). Errors: `STATUS_BAD_ARG` (zero badge — merged
/// flags need a bit to merge; dead nid).
pub fn notify(nid: u32, badge: u64) -> Result<(), Status> {
    if badge == 0 {
        return Err(STATUS_BAD_ARG);
    }
    // SAFETY: single writer under IF=0.
    let waiter = without_interrupts(|| -> Result<u64, Status> {
        bump!(notifies);
        unsafe {
            let ns = &mut *NOTIFS.get();
            let Some(n) = ns.get_mut(nid as usize).filter(|n| n.live) else {
                return Err(STATUS_BAD_ARG);
            };
            n.pending |= badge;
            let w = n.waiter;
            n.waiter = NO_TID;
            Ok(w)
        }
    })?;
    if waiter != NO_TID {
        if let Err(e) = sched::wake(waiter) {
            error!("ipc", "notify: wake(waiter {waiter}) failed: {e}");
            crate::halt::halt_machine("ipc: notify could not wake its waiter");
        }
    }
    Ok(())
}

/// Blocking: take the pending badge word (clearing it) — parking while
/// it is zero. Returns the badge as a positive payload (ABI v1's status
/// domain). Errors: `STATUS_BAD_ARG` (dead nid), `STATUS_BUSY` (a
/// second waiter — v1 is single-waiter, ADR-0018).
pub fn wait(nid: u32) -> Result<u64, Status> {
    // SAFETY: single writer under IF=0.
    let immediate = without_interrupts(|| -> Result<Option<u64>, Status> {
        bump!(waits);
        unsafe {
            let ns = &mut *NOTIFS.get();
            let Some(n) = ns.get_mut(nid as usize).filter(|n| n.live) else {
                return Err(STATUS_BAD_ARG);
            };
            if n.pending != 0 {
                let p = n.pending;
                n.pending = 0;
                return Ok(Some(p));
            }
            if n.waiter != NO_TID {
                return Err(STATUS_BUSY);
            }
            n.waiter = sched::current_thread_id();
            Ok(None)
        }
    })?;
    if let Some(p) = immediate {
        return Ok(p);
    }
    bump!(blocks);
    sched::block_current();
    // Resumed: only notify wakes a waiter, so pending must be nonzero.
    // SAFETY: single writer under IF=0.
    without_interrupts(|| unsafe {
        let n = &mut (*NOTIFS.get())[nid as usize];
        if n.pending == 0 {
            error!("ipc", "wait: resumed with no pending badge (nid={nid})");
            crate::halt::halt_machine("ipc: wait woken without a badge");
        }
        let p = n.pending;
        n.pending = 0;
        Ok(p)
    })
}
