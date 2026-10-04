//! Milestone 11.0 boot suite: kernel event plumbing (ADR-0071/0072).
//!
//! Runs after m7, before any production resident exists, on the real
//! kernel objects and scheduler. Every object it mints is destroyed
//! again, so the production endpoint/notification budgets are untouched.
//!
//! 1. `bound_signal` — a CALL queued while no server is parked in RECV
//!    ORs the binding's badge into the bound notification; the queued
//!    request is then served normally (exact words round trip).
//! 2. `notif_destroy_unbinds` — destroying the bound notification
//!    clears the binding, and a notification re-minted at the SAME index
//!    is never signalled by the old binding (generation pinned).
//! 3. `endpoint_destroy_unbinds` — a destroyed endpoint's binding dies
//!    with it; an endpoint re-minted at the same index starts unbound.
//! 4. `timer_quota` — `MAX_TIMERS_PER_PROCESS` arms succeed per owner,
//!    the next one is refused as `Quota` while another owner still arms,
//!    and releasing the owner restores exactly its capacity.
//! 5. `handoff_order` — a handoff wake runs before an earlier ordinary
//!    wake (front of the ready ring), without disturbing FIFO otherwise.
//!
//! Markers: `m11:test:<name>`, `m11: RESULT`.

use crate::ipc;
use crate::log::{log_error as error, log_info as info, write_marker};
use crate::sched;
use crate::sync::SyncCell;
use crate::timer;
use core::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

type Res = Result<(), &'static str>;

pub fn run_suite() -> bool {
    let checks: [(&str, fn() -> Res); 5] = [
        ("bound_signal", test_bound_signal),
        ("notif_destroy_unbinds", test_notif_destroy_unbinds),
        ("endpoint_destroy_unbinds", test_endpoint_destroy_unbinds),
        ("timer_quota", test_timer_quota),
        ("handoff_order", test_handoff_order),
    ];
    let mut passed = 0u32;
    for (name, test) in checks {
        match test() {
            Ok(()) => {
                passed += 1;
                write_marker(format_args!("m11:test:{name}: PASS"));
            }
            Err(reason) => {
                error!("m11", "test {name} failed: {reason}");
                write_marker(format_args!("m11:test:{name}: FAIL ({reason})"));
            }
        }
    }
    let total = checks.len() as u32;
    if passed == total {
        write_marker(format_args!("m11: RESULT PASS ({passed}/{total})"));
        true
    } else {
        write_marker(format_args!("m11: RESULT FAIL ({passed}/{total})"));
        false
    }
}

/// Yield until only the boot thread is live (bounded), then one more pass
/// so the last zombie is reaped.
fn drain(max_yields: usize) -> Res {
    let mut used = 0;
    while sched::live_threads() > 1 {
        if used >= max_yields {
            return Err("threads still live after the yield bound");
        }
        sched::yield_now();
        used += 1;
    }
    sched::yield_now();
    Ok(())
}

// ---- a kernel-thread caller ------------------------------------------------

static CALL_EID: AtomicUsize = AtomicUsize::new(0);
static CALL_REPLY: AtomicU64 = AtomicU64::new(0);
const REQUEST: [u64; 2] = [0x11A0_0001, 0x11A0_0002];

/// Kernel-thread client: one CALL on `CALL_EID`, records reply word 0.
fn caller_entry(_: usize) {
    let eid = CALL_EID.load(Ordering::Relaxed) as u32;
    let reply = match ipc::call(0, eid, REQUEST, None, [0; ipc::MSG_BYTES]) {
        Ok((words, _, _)) => words[0],
        Err(_) => u64::MAX,
    };
    CALL_REPLY.store(reply, Ordering::Relaxed);
}

/// Spawn a caller on `eid` and yield until its request is queued (the
/// caller is blocked inside CALL).
fn queue_call(eid: u32) -> Res {
    CALL_EID.store(eid as usize, Ordering::Relaxed);
    CALL_REPLY.store(0, Ordering::Relaxed);
    let tid = sched::spawn("m11-caller", caller_entry, 0)?;
    for _ in 0..16 {
        sched::yield_now();
        if sched::thread_blocked(tid) {
            return Ok(());
        }
    }
    Err("the caller never blocked in CALL")
}

/// Serve exactly one queued request on `eid` from the boot thread and let
/// the caller finish; its reply must be the request word plus one.
fn serve_one(eid: u32) -> Res {
    let (words, landed, _) = ipc::recv(0, eid).map_err(|_| "recv refused")?;
    if words != REQUEST || landed != ipc::CAP_NONE {
        return Err("served request differs from what was called");
    }
    ipc::reply(eid, [words[0] + 1, 0], None, [0; ipc::MSG_BYTES]).map_err(|_| "reply refused")?;
    drain(16)?;
    if CALL_REPLY.load(Ordering::Relaxed) != REQUEST[0] + 1 {
        return Err("the caller did not receive the exact reply");
    }
    Ok(())
}

// ---- 1. bound_signal --------------------------------------------------------

fn test_bound_signal() -> Res {
    let eid = ipc::create_endpoint()?;
    let nid = ipc::create_notification()?;
    let before = ipc::stats();
    ipc::bind(eid, nid, 0x40).map_err(|_| "bind refused")?;
    if ipc::binding_of(eid) != Some((nid, 0x40)) {
        return Err("binding not recorded");
    }
    if ipc::poll_pending(nid) != 0 {
        return Err("binding an idle endpoint signalled");
    }
    queue_call(eid)?;
    if ipc::poll_pending(nid) != 0x40 {
        return Err("a queued call did not raise the bound badge");
    }
    serve_one(eid)?;
    if ipc::stats().bound_signals != before.bound_signals + 1 {
        return Err("bound signal count is not exactly one");
    }
    ipc::unbind(eid).map_err(|_| "unbind refused")?;
    if ipc::binding_of(eid).is_some() {
        return Err("unbind left the binding");
    }
    ipc::destroy_endpoint(eid)?;
    ipc::destroy_notification(nid)?;
    info!(
        "m11",
        "bound_signal: queued CALL raised badge 0x40 on notification {nid}; request served exactly"
    );
    Ok(())
}

// ---- 2. notif_destroy_unbinds ------------------------------------------------

fn test_notif_destroy_unbinds() -> Res {
    let eid = ipc::create_endpoint()?;
    let nid = ipc::create_notification()?;
    ipc::bind(eid, nid, 0x80).map_err(|_| "bind refused")?;
    ipc::destroy_notification(nid)?;
    if ipc::binding_of(eid).is_some() {
        return Err("destroying the notification left the endpoint bound");
    }
    // The table hands out the lowest free index, so the next mint reuses
    // this one: exactly the situation a stale binding would exploit.
    let reused = ipc::create_notification()?;
    if reused != nid {
        let _ = ipc::destroy_notification(reused);
        let _ = ipc::destroy_endpoint(eid);
        return Err("the notification index was not reused (test precondition)");
    }
    queue_call(eid)?;
    let stray = ipc::poll_pending(reused);
    serve_one(eid)?;
    ipc::destroy_endpoint(eid)?;
    ipc::destroy_notification(reused)?;
    if stray != 0 {
        return Err("a re-minted notification received the old binding's badge");
    }
    info!(
        "m11",
        "notif_destroy_unbinds: binding cleared at destroy; re-minted index {nid} never signalled"
    );
    Ok(())
}

// ---- 3. endpoint_destroy_unbinds ---------------------------------------------

fn test_endpoint_destroy_unbinds() -> Res {
    let eid = ipc::create_endpoint()?;
    let nid = ipc::create_notification()?;
    ipc::bind(eid, nid, 0x100).map_err(|_| "bind refused")?;
    ipc::destroy_endpoint(eid)?;
    let again = ipc::create_endpoint()?;
    if again != eid {
        let _ = ipc::destroy_endpoint(again);
        let _ = ipc::destroy_notification(nid);
        return Err("the endpoint index was not reused (test precondition)");
    }
    if ipc::binding_of(again).is_some() {
        return Err("a re-minted endpoint inherited its predecessor's binding");
    }
    queue_call(again)?;
    let stray = ipc::poll_pending(nid);
    serve_one(again)?;
    ipc::destroy_endpoint(again)?;
    ipc::destroy_notification(nid)?;
    if stray != 0 {
        return Err("a destroyed endpoint's binding still signalled");
    }
    Ok(())
}

// ---- 4. timer_quota ------------------------------------------------------------

/// Owner ids that no process can have (pids are small and monotonic).
const OWNER_A: u64 = u64::MAX - 0x11;
const OWNER_B: u64 = u64::MAX - 0x12;

fn test_timer_quota() -> Res {
    let nid = ipc::create_notification()?;
    let before = timer::stats();
    let far = 3_600_000_000; // never fires during the test
    let mut ok = true;
    for _ in 0..timer::MAX_TIMERS_PER_PROCESS {
        ok &= timer::arm(OWNER_A, nid, 1, far).is_ok();
    }
    let over = timer::arm(OWNER_A, nid, 1, far);
    let other = timer::arm(OWNER_B, nid, 1, far);
    let held_a = timer::held_by(OWNER_A);
    let held_b = timer::held_by(OWNER_B);
    let released = timer::release_by_owner(OWNER_A);
    let after_release = timer::arm(OWNER_A, nid, 1, far);
    let held_after = timer::held_by(OWNER_A);
    timer::release_by_owner(OWNER_A);
    timer::release_by_owner(OWNER_B);
    ipc::destroy_notification(nid)?;
    if !ok {
        return Err("an arm within the quota was refused");
    }
    if over != Err(timer::ArmError::Quota) {
        return Err("the over-quota arm was not refused as Quota");
    }
    if other.is_err() || held_b != 1 {
        return Err("another owner could not arm while one was at quota");
    }
    if held_a != timer::MAX_TIMERS_PER_PROCESS || released != held_a {
        return Err("quota accounting does not match the table");
    }
    if after_release.is_err() || held_after != 1 {
        return Err("releasing the owner did not restore its quota exactly");
    }
    let s = timer::stats();
    if s.quota_refused != before.quota_refused + 1 || s.armed_now != before.armed_now {
        return Err("quota statistics or table occupancy did not return exactly");
    }
    info!(
        "m11",
        "timer_quota: {} per process; fifth refused as Quota; other owner armed; release restored",
        timer::MAX_TIMERS_PER_PROCESS
    );
    Ok(())
}

// ---- 5. handoff_order -----------------------------------------------------------

static ORDER: SyncCell<[u64; 4]> = SyncCell::new([0; 4]);
static ORDER_LEN: AtomicUsize = AtomicUsize::new(0);

fn sleeper_entry(tag: usize) {
    sched::block_current();
    let n = ORDER_LEN.fetch_add(1, Ordering::Relaxed);
    if n < 4 {
        // SAFETY: single CPU; each writer stores a distinct index.
        unsafe { (*ORDER.get())[n] = tag as u64 };
    }
}

fn test_handoff_order() -> Res {
    ORDER_LEN.store(0, Ordering::Relaxed);
    let a = sched::spawn("m11-wake", sleeper_entry, 1)?;
    let b = sched::spawn("m11-handoff", sleeper_entry, 2)?;
    for _ in 0..16 {
        if sched::thread_blocked(a) && sched::thread_blocked(b) {
            break;
        }
        sched::yield_now();
    }
    if !(sched::thread_blocked(a) && sched::thread_blocked(b)) {
        return Err("sleepers never blocked");
    }
    // A is woken first, the ordinary way; B second, as an IPC handoff.
    sched::wake(a)?;
    sched::wake_handoff(b)?;
    drain(16)?;
    // SAFETY: both writers are gone; single reader.
    let order = unsafe { *ORDER.get() };
    if ORDER_LEN.load(Ordering::Relaxed) != 2 || order[..2] != [2, 1] {
        return Err("the handoff wake did not run before the earlier ordinary wake");
    }
    info!(
        "m11",
        "handoff_order: handoff-woken thread ran first; ordinary wake followed"
    );
    Ok(())
}
