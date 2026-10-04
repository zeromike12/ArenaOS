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
//! 4. `orphan_unbinds` — when the process holding the endpoint's serve
//!    side is destroyed the endpoint is orphaned and its binding removed;
//!    a later server taking the endpoint over is not signalled through
//!    the dead server's notification.
//! 5. `timer_quota` — `MAX_TIMERS_PER_PROCESS` arms succeed per owner,
//!    the next one is refused as `Quota` while another owner still arms,
//!    and releasing the owner restores exactly its capacity.
//! 6. `handoff_order` — a handoff wake runs before threads other code made
//!    ready earlier (front of the ready ring), but never before a thread
//!    the calling thread itself woke earlier in its current run (causal
//!    order: a STOP notified before a CALL is handled first).
//! 7. `badged_endpoint` (ADR-0074) — only the serve side mints; badges are
//!    preserved by transfer and attenuated copy, never amplified; a call
//!    through a badged cap delivers its badge; badged caps never serve; a
//!    destroyed and re-minted endpoint refuses earlier badged caps; process
//!    teardown is frame-exact.
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
    let checks: [(&str, fn() -> Res); 7] = [
        ("bound_signal", test_bound_signal),
        ("notif_destroy_unbinds", test_notif_destroy_unbinds),
        ("endpoint_destroy_unbinds", test_endpoint_destroy_unbinds),
        ("orphan_unbinds", test_orphan_unbinds),
        ("timer_quota", test_timer_quota),
        ("handoff_order", test_handoff_order),
        ("badged_endpoint", test_badged_endpoint),
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
static CALL_BADGE: AtomicU64 = AtomicU64::new(0);
const REQUEST: [u64; 2] = [0x11A0_0001, 0x11A0_0002];

/// Kernel-thread client: one CALL on `CALL_EID`, records reply word 0.
fn caller_entry(_: usize) {
    let eid = CALL_EID.load(Ordering::Relaxed) as u32;
    let badge = CALL_BADGE.load(Ordering::Relaxed) as u32;
    let reply = match ipc::call_badged(0, eid, badge, REQUEST, None, [0; ipc::MSG_BYTES]) {
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

// ---- 4. orphan_unbinds -------------------------------------------------------

fn test_orphan_unbinds() -> Res {
    let frames_before = crate::frames::free_frames();
    let eid = ipc::create_endpoint()?;
    let nid = ipc::create_notification()?;
    // A real (thread-less) process holding the serve side by capability:
    // `fail_calls_for_server` discovers served endpoints from caps alone.
    let pid = crate::proc::create("m11-server")?;
    crate::cap::issue(
        pid,
        0,
        crate::cap::Cap {
            obj: crate::cap::CapObj::Endpoint { eid },
            rights: crate::cap::RIGHTS_READ,
        },
    )?;
    ipc::bind(eid, nid, 0x200).map_err(|_| "bind refused")?;
    crate::proc::destroy(pid)?;
    let left = ipc::binding_of(eid);
    // Takeover: the boot thread takes up the serve side (its first
    // receive clears the orphan state; nothing is queued yet). A call
    // queued afterwards must not reach the dead server's notification.
    if ipc::try_recv(0, eid).map(|_| ()) != Err(crate::arch::x86_64::syscall::STATUS_BUSY) {
        return Err("takeover receive did not find an empty, re-opened endpoint");
    }
    queue_call(eid)?;
    let stray = ipc::poll_pending(nid);
    serve_one(eid)?;
    ipc::destroy_endpoint(eid)?;
    ipc::destroy_notification(nid)?;
    if left.is_some() {
        return Err("orphaning the endpoint left the dead server's binding");
    }
    if stray != 0 {
        return Err("a call after takeover signalled the dead server's notification");
    }
    if crate::frames::free_frames() != frames_before {
        return Err("the server process did not tear down frame-exactly");
    }
    info!(
        "m11",
        "orphan_unbinds: server pid {pid} destroyed; binding removed; takeover not signalled"
    );
    Ok(())
}

// ---- 5. timer_quota ------------------------------------------------------------

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

// ---- 6. handoff_order -----------------------------------------------------------

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

fn waker_entry(tid: usize) {
    let _ = sched::wake(tid as u64);
}

/// Spawn two sleepers (tags 1 and 2) and wait until both are blocked.
fn sleepers() -> Result<(u64, u64), &'static str> {
    ORDER_LEN.store(0, Ordering::Relaxed);
    let a = sched::spawn("m11-wake", sleeper_entry, 1)?;
    let b = sched::spawn("m11-handoff", sleeper_entry, 2)?;
    for _ in 0..16 {
        if sched::thread_blocked(a) && sched::thread_blocked(b) {
            return Ok((a, b));
        }
        sched::yield_now();
    }
    Err("sleepers never blocked")
}

fn ran() -> (usize, [u64; 4]) {
    // SAFETY: the writers are gone; single reader.
    (ORDER_LEN.load(Ordering::Relaxed), unsafe { *ORDER.get() })
}

fn test_handoff_order() -> Res {
    // Front of the ring: A was made ready by another thread before this
    // one was switched in; the handoff-woken B still runs first.
    let (a, b) = sleepers()?;
    sched::spawn("m11-waker", waker_entry, a as usize)?;
    for _ in 0..16 {
        if !sched::thread_blocked(a) {
            break;
        }
        sched::yield_now();
    }
    if sched::thread_blocked(a) {
        return Err("helper never woke the first sleeper");
    }
    sched::wake_handoff(b)?;
    drain(16)?;
    let (n, order) = ran();
    if n != 2 || order[..2] != [2, 1] {
        return Err("the handoff wake did not run before an earlier wake by another thread");
    }
    // Causal order: this thread wakes A itself, then hands off to B in
    // the same run. A was caused first and must run first.
    let (a, b) = sleepers()?;
    sched::wake(a)?;
    sched::wake_handoff(b)?;
    drain(16)?;
    let (n, order) = ran();
    if n != 2 || order[..2] != [1, 2] {
        return Err("a handoff overtook the caller's own earlier wake");
    }
    info!(
        "m11",
        "handoff_order: handoff ran ahead of another thread's earlier wake; never ahead of the caller's own earlier wake"
    );
    Ok(())
}

// ---- 7. badged_endpoint (ADR-0074) --------------------------------------------

fn test_badged_endpoint() -> Res {
    use crate::cap::{self, Cap, CapObj, RIGHTS_COPY as C, RIGHTS_READ as R, RIGHTS_WRITE as W};
    let frames_before = crate::frames::free_frames();
    let eid = ipc::create_endpoint()?;
    let server = crate::proc::create("m11-badge-server")?;
    let client = crate::proc::create("m11-badge-client")?;
    cap::issue(
        server,
        0,
        Cap {
            obj: CapObj::Endpoint { eid },
            rights: R | W | C,
        },
    )?;
    cap::issue(
        client,
        0,
        Cap {
            obj: CapObj::Endpoint { eid },
            rights: W | C,
        },
    )?;
    // Minting authority: serve side only; badge 0 reserved; never READ.
    if cap::mint_badged(client, 0, 7, W).is_ok()
        || cap::mint_badged(server, 0, 0, W).is_ok()
        || cap::mint_badged(server, 0, 7, W | R).is_ok()
        || cap::mint_badged(server, 0, 7, C).is_ok()
    {
        return Err("a badged cap was minted without serve-side authority or with bad rights");
    }
    let minted = cap::mint_badged(server, 0, 0x0B0B, W | C)?;
    // Transfer into the client exactly as IPC landing installs caps.
    let landed = cap::grant(client, cap::read(server, minted)?)?;
    // Attenuated copy keeps the badge; amplification and re-delegation
    // without COPY are refused.
    cap::copy(client, landed, client, 20, W)?;
    if cap::copy(client, 20, client, 21, W).is_ok()
        || cap::copy(client, landed, client, 22, W | R).is_ok()
    {
        return Err("badged cap delegated without COPY or amplified");
    }
    if cap::call_target(client, 0) != Ok((eid, 0))
        || cap::call_target(client, landed) != Ok((eid, 0x0B0B))
        || cap::call_target(client, 20) != Ok((eid, 0x0B0B))
    {
        return Err("call targets do not carry exactly the minted badge");
    }
    // A badged cap is never the serve side, even in the server's own space.
    let server_badged = cap::mint_badged(server, 0, 0x0C0C, W)?;
    if cap::read(server, server_badged)?.rights & R != 0 {
        return Err("a badged cap carries READ");
    }
    if cap::serves_endpoint(client, eid) {
        return Err("a client holding only badged/WRITE caps counts as a server");
    }
    // Delivery: the server receives the badge of the invoked capability.
    CALL_BADGE.store(0x0B0B, Ordering::Relaxed);
    queue_call(eid)?;
    let (words, _, _, badge) = ipc::recv_badged(0, eid, true).map_err(|_| "recv refused")?;
    ipc::reply(eid, [words[0] + 1, 0], None, [0; ipc::MSG_BYTES]).map_err(|_| "reply refused")?;
    drain(16)?;
    CALL_BADGE.store(0, Ordering::Relaxed);
    if badge != 0x0B0B || CALL_REPLY.load(Ordering::Relaxed) != REQUEST[0] + 1 {
        return Err("the server did not receive the caller's badge");
    }
    // Stale object reuse: destroy and re-mint the endpoint index.
    ipc::destroy_endpoint(eid)?;
    let again = ipc::create_endpoint()?;
    let stale = cap::call_target(client, landed).is_ok() || cap::call_target(client, 20).is_ok();
    crate::proc::destroy(server)?;
    crate::proc::destroy(client)?;
    ipc::destroy_endpoint(again)?;
    if again != eid {
        return Err("the endpoint index was not reused (test precondition)");
    }
    if stale {
        return Err("a badged cap reached a re-minted endpoint at the same index");
    }
    if crate::frames::free_frames() != frames_before {
        return Err("badged-endpoint processes did not tear down frame-exactly");
    }
    info!(
        "m11",
        "badged_endpoint: serve-side mint only; badge 0x0B0B delivered; copy/transfer kept it; stale after re-mint"
    );
    Ok(())
}
