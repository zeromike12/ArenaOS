//! IPC v1.1 (M4.4 + M5.3, ADR-0018/0023): endpoints, synchronous
//! call/reply, badged merged notifications, capability transfer, and a
//! 64-byte inline message buffer for small payloads (names, dirents)
//! that two words cannot carry.
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

use crate::arch::x86_64::syscall::{STATUS_BAD_ARG, STATUS_BUSY, STATUS_SERVICE_GONE, Status};
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

/// Inline message payload size, v1.1 (ADR-0023): CALL snapshots the
/// caller's buffer into the request slot, RECV hands it to the server,
/// REPLY stages the server's buffer, and the resumed CALL copies it
/// back to the caller's SAME buffer. Null pointer = absent payload
/// (zero-filled) — v1.0 callers stay wire-compatible.
pub const MSG_BYTES: usize = 64;
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
    /// The server process died holding (or owing) this request. The
    /// caller is woken and `call` returns `STATUS_SERVICE_GONE`
    /// instead of a reply (M6.5, ADR-0028).
    Failed,
}

#[derive(Clone, Copy)]
struct CallSlot {
    state: SlotState,
    /// Blocked caller's thread id (drives the reply wake).
    caller: u64,
    words: [u64; 2],
    /// Inline request payload, v1.1 (ADR-0023): snapshotted from the
    /// caller's buffer at `call` time.
    msg: [u8; MSG_BYTES],
    /// Staged at `call`, moved into the server's space at delivery
    /// (then `Cap::EMPTY` again).
    send_cap: Cap,
    /// The server thread the request was delivered to.
    server: u64,
    /// Where `send_cap` landed in the server's space (`CAP_NONE` =
    /// none sent, or dropped into a full space).
    landed_cap: u64,
    reply_words: [u64; 2],
    /// Inline reply payload, v1.1: staged by `reply`, copied back to
    /// the caller's (same) buffer when the caller resumes.
    reply_msg: [u8; MSG_BYTES],
    /// Staged by `reply`, installed into the caller's space when the
    /// caller resumes (owner-context discipline).
    reply_cap: Cap,
}

const EMPTY_SLOT: CallSlot = CallSlot {
    state: SlotState::Empty,
    caller: 0,
    words: [0; 2],
    msg: [0; MSG_BYTES],
    send_cap: Cap::EMPTY,
    server: NO_TID,
    landed_cap: CAP_NONE,
    reply_words: [0; 2],
    reply_msg: [0; MSG_BYTES],
    reply_cap: Cap::EMPTY,
};

#[derive(Clone, Copy)]
struct Endpoint {
    live: bool,
    /// Nobody is serving this endpoint: the process that held its
    /// serve side was destroyed (M7.1b).
    ///
    /// ADR-0028 gave a typed answer to callers who were IN FLIGHT when
    /// a server died, and stopped there. A call sent AFTERWARDS simply
    /// queued on an endpoint nobody would ever read and blocked
    /// forever — the same hang, one instant later, and the one a
    /// network stack actually hits: it discovers its driver is gone by
    /// CALLING it. So an orphaned endpoint refuses new calls with
    /// `STATUS_SERVICE_GONE` until somebody takes up the serve side
    /// again, which a restarted service does by its first `recv`.
    orphaned: bool,
    q: [CallSlot; QUEUE_DEPTH],
    /// Thread parked in `recv` (`NO_TID` = none). v1: one server per
    /// endpoint — a second `recv` gets `STATUS_BUSY` (ADR-0018).
    server: u64,
}

const EMPTY_EP: Endpoint = Endpoint {
    live: false,
    orphaned: false,
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
    /// Calls answered with `STATUS_SERVICE_GONE` because the serving
    /// process was destroyed under them (M6.5, ADR-0028).
    pub service_gone: u64,
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
    service_gone: 0,
});
static ENDPOINTS: SyncCell<[Endpoint; MAX_ENDPOINTS]> = SyncCell::new([EMPTY_EP; MAX_ENDPOINTS]);
static NOTIFS: SyncCell<[Notif; MAX_NOTIFS]> = SyncCell::new([EMPTY_NOTIF; MAX_NOTIFS]);

/// Snapshot of the IPC counters.
/// Is endpoint `eid` still a live kernel object? (M6.5, ADR-0028: a
/// server dying must not take its endpoint with it — the clients'
/// capabilities name the endpoint, not the process, which is what
/// lets a restarted service pick the serve side back up.)
pub fn endpoint_live(eid: u32) -> bool {
    without_interrupts(|| {
        // SAFETY: single reader under IF=0.
        unsafe { (*ENDPOINTS.get()).get(eid as usize).is_some_and(|e| e.live) }
    })
}

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
                orphaned: false,
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
///
/// An Untyped cap LANDING through IPC is forced LENT (`owned: false`,
/// ADR-0022): the sender keeps the single owning cap, so the receiver
/// can aim a device at the frame (zero-copy DMA via `SYS_CAP_PHYS`) and
/// discard the reference when the request completes — freeing or mapping
/// through a landed cap is structurally impossible.
fn install_cap(pid: u64, cap: Cap) -> u64 {
    let cap = match cap.obj {
        CapObj::Untyped { phys, .. } => Cap {
            obj: CapObj::Untyped { phys, owned: false },
            rights: cap.rights,
        },
        _ => cap,
    };
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
fn take_request(
    eidx: usize,
    qi: usize,
    server_tid: u64,
    server_pid: u64,
) -> ([u64; 2], [u8; MSG_BYTES]) {
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
        (slot.words, slot.msg)
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
    msg: [u8; MSG_BYTES],
) -> Result<([u64; 2], u64, [u8; MSG_BYTES]), Status> {
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
            if ep.orphaned {
                // Nobody is serving this. Answer now rather than
                // enqueue into silence (M7.1b). Counted directly
                // rather than through `bump!`, which brings its own
                // `unsafe` and would nest inside this one.
                (*STATS.get()).service_gone += 1;
                return Err(STATUS_SERVICE_GONE);
            }
            let Some(qi) = ep.q.iter().position(|s| s.state == SlotState::Empty) else {
                return Err(STATUS_BUSY);
            };
            ep.q[qi] = CallSlot {
                state: SlotState::Waiting,
                caller: sched::current_thread_id(),
                words,
                msg,
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
            let _ = take_request(eidx, qi, parked, server_pid);
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
    // slot, install the reply cap into our own space. A server that
    // died holding this request leaves the slot Failed instead, and
    // the caller gets a typed status rather than a reply (ADR-0028).
    // SAFETY: single writer under IF=0.
    let failed = without_interrupts(|| unsafe {
        let slot = &mut (*ENDPOINTS.get())[eidx].q[qi];
        if slot.state == SlotState::Failed {
            *slot = EMPTY_SLOT;
            true
        } else {
            false
        }
    });
    if failed {
        bump!(service_gone);
        return Err(STATUS_SERVICE_GONE);
    }
    let (reply_words, reply_cap, reply_msg) = without_interrupts(|| unsafe {
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
        let m = slot.reply_msg;
        *slot = EMPTY_SLOT;
        (w, c, m)
    });
    let landed = if matches!(reply_cap.obj, CapObj::None) {
        CAP_NONE
    } else {
        install_cap(pid, reply_cap)
    };
    Ok((reply_words, landed, reply_msg))
}

/// Server side: take the oldest request on endpoint `eid`, blocking
/// while the queue is empty. Returns the request words and the landing
/// slot of the transferred cap in THIS process's space (`CAP_NONE` when
/// none came). The slot stays Delivered until `reply`.
///
/// Errors: `STATUS_BAD_ARG` (dead eid), `STATUS_BUSY` (a second server
/// on one endpoint — v1 is single-server, ADR-0018).
pub fn recv(pid: u64, eid: u32) -> Result<([u64; 2], u64, [u8; MSG_BYTES]), Status> {
    // Phase 1 — either a Waiting request exists (self-deliver) or park.
    // SAFETY: single writer under IF=0.
    let queued = without_interrupts(|| -> Result<Option<usize>, Status> {
        bump!(recvs);
        unsafe {
            let eps = &mut *ENDPOINTS.get();
            let Some(ep) = eps.get_mut(eid as usize).filter(|e| e.live) else {
                return Err(STATUS_BAD_ARG);
            };
            // Somebody is serving this endpoint again (M7.1b). A
            // restarted service announces itself simply by asking for
            // work — no re-registration step, and no way for the
            // orphan flag to outlive the situation it describes.
            ep.orphaned = false;
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
            // wake — we ARE the server thread). The slot carries words
            // + msg; phase 2 reads them back out.
            let _ = take_request(eid as usize, qi, tid, pid);
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
    let (words, landed, msg) = without_interrupts(|| unsafe {
        let slot = &(*ENDPOINTS.get())[eid as usize].q[qi];
        (slot.words, slot.landed_cap, slot.msg)
    });
    Ok((words, landed, msg))
}

/// Server side: stage the reply `[w0, w1]` (+ optional cap for the
/// caller's space) into this server's delivered slot and wake the
/// caller. Errors: `STATUS_BAD_ARG` (dead eid, or no request delivered
/// to this thread — a reply without a recv is a server bug, answered
/// with a typed status, not a fault).
pub fn reply(
    eid: u32,
    words: [u64; 2],
    send_cap: Option<Cap>,
    msg: [u8; MSG_BYTES],
) -> Result<(), Status> {
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
            slot.reply_msg = msg;
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

/// Fail every call that `pid` owed an answer to, and wake the callers
/// (M6.5, ADR-0028). Called from `proc::destroy` before the process
/// is gone for good.
///
/// "Owed an answer" is decided by CAPABILITY, not by bookkeeping: the
/// endpoints this process serves are exactly the ones it holds an
/// `Endpoint` cap with READ rights for. Nothing else in the kernel
/// records that association — and nothing else needs to, because the
/// cap IS the authority to serve.
///
/// Both queue states count. A request already `Delivered` was in the
/// server's hands, and a request still `Waiting` was addressed to a
/// service that no longer exists; in neither case is a reply ever
/// coming. `Replied` slots are left alone — that answer is real and
/// the caller is entitled to it even though the server has since
/// died.
///
/// Returns how many callers were failed, for the destroy log.
pub fn fail_calls_for_server(pid: u64) -> usize {
    // Phase 1 (IF=0): mark the slots and collect the callers to wake.
    // The wake happens outside the borrow, exactly as `call` does it.
    let mut wake_list = [NO_TID; QUEUE_DEPTH * MAX_ENDPOINTS];
    let mut n = 0usize;
    without_interrupts(|| {
        // SAFETY: single writer under IF=0.
        unsafe {
            let eps = &mut *ENDPOINTS.get();
            for (eidx, ep) in eps.iter_mut().enumerate() {
                if !ep.live {
                    continue;
                }
                if !crate::cap::serves_endpoint(pid, eidx as u32) {
                    continue;
                }
                // The server is dying: it can no longer be parked here.
                if ep.server != NO_TID && sched::proc_id_of(ep.server).unwrap_or(0) == pid {
                    ep.server = NO_TID;
                }
                // Whoever held the serve side is dying: until somebody
                // takes it up again, calls here have nowhere to land.
                ep.orphaned = true;
                for slot in ep.q.iter_mut() {
                    if matches!(slot.state, SlotState::Waiting | SlotState::Delivered) {
                        slot.state = SlotState::Failed;
                        if slot.caller != NO_TID && n < wake_list.len() {
                            wake_list[n] = slot.caller;
                            n += 1;
                        }
                    }
                }
            }
            (*STATS.get()).service_gone += n as u64;
        }
    });
    // Phase 2: wake them. A caller that cannot be woken is a broken
    // machine, not a recoverable condition — the same rule `call` and
    // `notify` already apply.
    for &tid in &wake_list[..n] {
        if let Err(e) = sched::wake(tid) {
            error!(
                "ipc",
                "fail_calls_for_server: wake(caller {tid}) failed: {e}"
            );
            crate::halt::halt_machine("ipc: a failed caller could not be woken");
        }
    }
    n
}

/// Drop every kernel-side reference to threads belonging to `pid`
/// (M6.5, ADR-0028): the endpoint server slots it is parked in and
/// the notification waiter slots it is parked on.
///
/// This MUST run before `sched::kill_threads_of`. Those slots hold
/// raw thread ids, and both `call` and `notify` treat "I have a
/// parked thread id and cannot wake it" as a halting offence — which
/// is the right rule, and exactly why the ids must be gone before the
/// threads are. A dead driver parked in `SYS_IPC_RECV` would
/// otherwise be woken by the next client that called its endpoint.
///
/// Returns (endpoint server slots cleared, notification waiters cleared).
pub fn release_blocked_of(pid: u64) -> (usize, usize) {
    without_interrupts(|| {
        // SAFETY: single writer under IF=0.
        unsafe {
            let mut servers = 0;
            let mut waiters = 0;
            for ep in (*ENDPOINTS.get()).iter_mut() {
                if ep.live
                    && ep.server != NO_TID
                    && sched::proc_id_of(ep.server).unwrap_or(0) == pid
                {
                    ep.server = NO_TID;
                    servers += 1;
                }
            }
            for n in (*NOTIFS.get()).iter_mut() {
                if n.live && n.waiter != NO_TID && sched::proc_id_of(n.waiter).unwrap_or(0) == pid {
                    n.waiter = NO_TID;
                    waiters += 1;
                }
            }
            (servers, waiters)
        }
    })
}

/// Take the pending badge word WITHOUT blocking (0 = nothing yet).
///
/// The suites need this and production does not, for a reason worth
/// writing down: the boot thread must never park while its children
/// are waiting on devices. `block_current` treats "nothing runnable"
/// as a deadlock (ADR-0018) — correctly, because it cannot know an
/// interrupt is coming — and in production the boot thread becomes
/// the idle thread precisely so that situation is unreachable. A
/// suite that blocks on a notification its children have not signalled
/// yet halts the machine, which is exactly what M7.1b's handshake did
/// on its first boot.
///
/// Every other suite gets away with `wait` because it only calls it
/// AFTER a drain has confirmed the children are gone, so the badge is
/// already pending and the wait returns immediately. A handshake with
/// a LIVE child needs this instead, inside a yield loop.
pub fn poll_pending(nid: u32) -> u64 {
    without_interrupts(|| {
        // SAFETY: single writer under IF=0.
        unsafe {
            let ns = &mut *NOTIFS.get();
            match ns.get_mut(nid as usize).filter(|n| n.live) {
                Some(n) => {
                    let b = n.pending;
                    n.pending = 0;
                    b
                }
                None => 0,
            }
        }
    })
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
