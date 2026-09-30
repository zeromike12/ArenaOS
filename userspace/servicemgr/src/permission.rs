//! ADR-0048/0049: manager-owned broker/app, exact live-cap plan and
//! two-grant success+exit+deadline readiness on the EXISTING private notif.
//! No policy persistence is claimed by this volatile integration path.
use arena_servicemgr::inventory::{self, NamedSlot, SyscallProbe};
use arena_servicemgr::manifest::{self, Dependency, External, Key, Kind, Request, Service, Step};
#[path = "../../abi.rs"]
mod abi;
use abi::*;

pub const BROKER_EXIT: u64 = 1 << 9;
pub const APP_EXIT: u64 = 1 << 10;
pub const APP_REQUEST: u64 = 1 << 11; // shared wake HINT; never approval
const EVENTS: u64 = 1;
const RESTART: u64 = 7;
const PROBE_IMAGE: u64 = 9;
const BROKER_IMAGE: u8 = 12;
const APP_IMAGE: u8 = 13;
const FS: u8 = 14;
const MED: u8 = 15;
const MARKER: u8 = 16;
const RNG: u8 = 5;
const PROBE_DEADLINE_US: u64 = 2_000_000;
const I_BROKER: Key = Key(24);
const I_APP: Key = Key(25);
const K_FS: Key = Key(1);
const K_RNG: Key = Key(2);
const K_MED: Key = Key(3);
const K_MARKER: Key = Key(4);
const SLOTS: [NamedSlot; 6] = [
    NamedSlot { key: I_BROKER, slot: BROKER_IMAGE },
    NamedSlot { key: I_APP, slot: APP_IMAGE },
    NamedSlot { key: K_FS, slot: FS },
    NamedSlot { key: K_RNG, slot: RNG },
    NamedSlot { key: K_MED, slot: MED },
    NamedSlot { key: K_MARKER, slot: MARKER },
];
const B_GRANTS: [Request; 4] = [
    Request { key: K_FS, kind: Kind::Endpoint, rights: manifest::WRITE, child_slot: 0 },
    Request { key: K_RNG, kind: Kind::Endpoint, rights: manifest::WRITE, child_slot: 1 },
    Request { key: K_MED, kind: Kind::Endpoint, rights: manifest::READ, child_slot: 2 },
    Request { key: K_MARKER, kind: Kind::Notification, rights: manifest::READ, child_slot: 3 },
];
const A_GRANTS: [Request; 1] = [Request {
    key: K_MED, kind: Kind::Endpoint, rights: manifest::WRITE | manifest::COPY, child_slot: 0,
}];
const B_DEPS: [Dependency; 2] = [Dependency::External(1), Dependency::External(2)];
const A_DEPS: [Dependency; 1] = [Dependency::Managed(24)];
const SERVICES: [Service<'static>; 2] = [
    Service { id: 24, image: I_BROKER, image_id: 24, grants: &B_GRANTS,
        dependencies: &B_DEPS, restart_limit: 2, backoff_us: 1000 },
    Service { id: 25, image: I_APP, image_id: 25, grants: &A_GRANTS,
        dependencies: &A_DEPS, restart_limit: 2, backoff_us: 1000 },
];
#[derive(Clone, Copy)]
struct Child { pid: u64, slot: u8 }
pub struct State {
    broker: Child,
    app: Option<Child>,
    broker_restarts: u8,
    broker_online: bool,
    app_starts: u8,
    plan: [Step; 2],
}
fn log(s: &str) {
    let _ = unsafe { syscall2(SYS_DEBUG_WRITE, s.as_ptr() as u64, s.len() as u64) };
}
fn probe_boot() -> Result<[Step; 2], ()> {
    let p = SyscallProbe;
    let root = inventory::collect(&SLOTS, &p).map_err(|_| ())?;
    let r = root.as_slice();
    if r[0].kind != Kind::Image || r[0].object != 24 || r[0].rights != manifest::READ
        || r[1].kind != Kind::Image || r[1].object != 25 || r[1].rights != manifest::READ
        || r[2].kind != Kind::Endpoint || r[2].rights != manifest::WRITE | manifest::COPY
        || r[3].kind != Kind::Endpoint || r[3].rights & (manifest::WRITE | manifest::COPY)
            != manifest::WRITE | manifest::COPY
        || r[4].kind != Kind::Endpoint || r[4].rights != manifest::READ | manifest::WRITE | manifest::COPY
        || r[5].kind != Kind::Notification || r[5].rights != manifest::READ | manifest::COPY
        || [r[2].object, r[3].object].contains(&r[4].object)
    { return Err(()); }
    let mut private = [0u64; 3];
    if unsafe { syscall2(SYS_CAP_DESCRIBE, RESTART, private.as_mut_ptr() as u64) } != 0
        || private[0] != 3 || private[2] & (RIGHTS_READ | RIGHTS_WRITE | RIGHTS_COPY)
            != RIGHTS_READ | RIGHTS_WRITE | RIGHTS_COPY
        || r[5].object == private[1]
    { return Err(()); }
    // netd/rngd live device probe completed before this policy path.
    let plan = inventory::plan(&SLOTS, &p, &SERVICES,
        &[External { id: 1, ready: true }, External { id: 2, ready: true }]).map_err(|_| ())?;
    if plan.count != 2 { return Err(()); }
    let b = plan.steps[0].ok_or(())?;
    let a = plan.steps[1].ok_or(())?;
    if b.service_id != 24 || a.service_id != 25 || b.count != 4 || a.count != 1 {
        return Err(());
    }
    Ok([b, a])
}
fn spawn_step(step: Step, badge: u64) -> Result<Child, ()> {
    let mut spec = [(0u64, 0u64); 5];
    for i in 0..step.count as usize {
        let g = step.grants[i].ok_or(())?;
        if g.child_slot != i as u8 { return Err(()); }
        spec[i] = (g.source_slot as u64, g.rights as u64);
    }
    let pid = unsafe { syscall5(SYS_SPAWN, step.image_slot as u64,
        spec.as_ptr() as u64, step.count as u64, EVENTS, badge) };
    if pid <= 0 { return Err(()); }
    let slot = inventory::child_handle(pid as u64, &SyscallProbe).map_err(|_| ())?;
    Ok(Child { pid: pid as u64, slot })
}
fn finish(child: Child, mut live: bool) -> Result<(), ()> {
    if inventory::child_handle(child.pid, &SyscallProbe) != Ok(child.slot) { return Err(()); }
    let mut r = inventory::finish(child.slot, live);
    if r == Err(STATUS_BUSY) {
        // Only swap modes after proving the observed liveness changed.
        if let Some(now_live) = child_live(child.pid) {
            if now_live != live { live = now_live; r = inventory::finish(child.slot, live); }
        }
    }
    r.map_err(|_| ())
}
fn child_live(pid: u64) -> Option<bool> {
    let mut data = [0u64; 64];
    let n = unsafe { syscall2(SYS_PROC_LIST, data.as_mut_ptr() as u64, 32) };
    if !(0..=32).contains(&n) { return None; }
    for pair in data[..2 * n as usize].chunks_exact(2) {
        if pair[0] == pid { return Some(pair[1] != 0); }
    }
    None
}
/// A failed probe returns its held worker handle to the caller; the
/// caller, NOT this function, orders broker/worker teardown. No Process
/// handle is discarded merely because a deadline or reply failed.
fn ping_with_deadline() -> Result<(), Option<Child>> {
    // Image20 distinguishes mediator mode from netd/rngd mode by slot1.
    let grants = [(MED as u64, RIGHTS_WRITE), (RESTART, RIGHTS_WRITE)];
    let pid = unsafe { syscall5(SYS_SPAWN, PROBE_IMAGE, grants.as_ptr() as u64,
        2, RESTART, MGR_BADGE_PERM_PROBE_EXIT) };
    if pid <= 0 { return Err(None); }
    let worker = Child { pid: pid as u64,
        slot: match inventory::child_handle(pid as u64, &SyscallProbe) {
            Ok(h) => h,
            Err(_) => { log("servicemgr: FATAL PING worker lost Process cap\r\n"); return Err(None); }
        } };
    let timer = unsafe { syscall3(SYS_TIMER_ARM, RESTART,
        MGR_BADGE_PERM_PROBE_DEADLINE, PROBE_DEADLINE_US) };
    if timer < 0 { return Err(Some(worker)); }
    let mut bits = 0u64;
    loop {
        let seen = unsafe { syscall1(SYS_WAIT, RESTART) };
        if seen < 0 { break; }
        bits |= seen as u64;
        if bits & MGR_BADGE_PERM_PROBE_DEADLINE != 0
            || bits & !(MGR_BADGE_PERM_PROBE_OK | MGR_BADGE_PERM_PROBE_EXIT | MGR_BADGE_PERM_PROBE_DEADLINE) != 0
            || bits & (MGR_BADGE_PERM_PROBE_OK | MGR_BADGE_PERM_PROBE_EXIT)
                == (MGR_BADGE_PERM_PROBE_OK | MGR_BADGE_PERM_PROBE_EXIT)
        { break; }
    }
    let _ = unsafe { syscall1(SYS_TIMER_CANCEL, timer as u64) };
    let pending = unsafe { syscall1(SYS_TRY_WAIT, RESTART) };
    let good = pending == 0
        && bits == (MGR_BADGE_PERM_PROBE_OK | MGR_BADGE_PERM_PROBE_EXIT)
        && child_live(worker.pid) == Some(false);
    if !good {
        log("servicemgr: permission PING failed/deadline; no READY\r\n");
        return Err(Some(worker));
    }
    if finish(worker, false).is_err() {
        log("servicemgr: PING worker reap refused; no READY\r\n");
        return Err(Some(worker));
    }
    log("servicemgr: permission PING result + exit before deadline; worker reaped\r\n");
    Ok(())
}

/// Broker-first on an actual timeout: fail the live caller with typed
/// SERVICE_GONE, let it exit before a second bounded deadline, then reap.
/// The opt-in caller-first branch proves the GENERAL kernel sweep by
/// reproducing the previously halting teardown order on a blocked call.
fn failed_probe_teardown(broker: Child, worker: Option<Child>, caller_first: bool) -> bool {
    if caller_first {
        if let Some(w) = worker {
            if finish(w, child_live(w.pid).unwrap_or(true)).is_err() {
                log("servicemgr: FATAL caller-first PING worker teardown refused\r\n");
                return false;
            }
            log("servicemgr: diagnostic caller-first worker stopped before broker\r\n");
        }
    }
    if finish(broker, child_live(broker.pid).unwrap_or(true)).is_err() {
        log("servicemgr: FATAL permission broker Process-cap teardown REFUSED\r\n");
        return false;
    }
    if !caller_first {
        if let Some(w) = worker {
            if child_live(w.pid) == Some(true) {
                let timer = unsafe { syscall3(SYS_TIMER_ARM, RESTART,
                    MGR_BADGE_PERM_PROBE_DEADLINE, PROBE_DEADLINE_US) };
                if timer >= 0 {
                    let mut seen = 0u64;
                    while seen & (MGR_BADGE_PERM_PROBE_EXIT | MGR_BADGE_PERM_PROBE_DEADLINE) == 0 {
                        let b = unsafe { syscall1(SYS_WAIT, RESTART) };
                        if b <= 0 { break; }
                        seen |= b as u64;
                    }
                    let _ = unsafe { syscall1(SYS_TIMER_CANCEL, timer as u64) };
                    let _ = unsafe { syscall1(SYS_TRY_WAIT, RESTART) };
                }
            }
            if finish(w, child_live(w.pid).unwrap_or(true)).is_err() {
                log("servicemgr: FATAL permission PING worker teardown refused\r\n");
                return false;
            }
            log("servicemgr: broker-first failed blocked PING; worker reaped/stopped\r\n");
        }
    }
    true
}
fn start_broker(step: Step) -> Result<Child, ()> {
    let broker = spawn_step(step, BROKER_EXIT)?;
    if let Err(worker) = ping_with_deadline() {
        if !failed_probe_teardown(broker, worker, false) {
            log("servicemgr: FATAL permission startup teardown incomplete\r\n");
        }
        return Err(());
    }
    log("servicemgr: permissiond READY (volatile policy; default DENY)\r\n");
    Ok(broker)
}
pub fn start() -> Result<State, ()> {
    let plan = probe_boot()?;
    let broker = start_broker(plan[0])?;
    let app = match spawn_step(plan[1], APP_EXIT) {
        Ok(app) => app,
        Err(()) => {
            let _ = finish(broker, child_live(broker.pid).unwrap_or(true));
            return Err(());
        }
    };
    log("servicemgr: permission app SPAWNED (not grant/approval)\r\n");
    Ok(State { broker, app: Some(app), broker_restarts: 0, broker_online: true,
        app_starts: 1, plan })
}
impl State {
    /// Explicit private admin proof: re-probe the *current* broker.
    /// A failed result or deadline MUST NOT report READY; stop the
    /// receiver through its held Process cap, never by trusting a pid.
    pub fn test_probe(&mut self, caller_first: bool) {
        if !self.broker_online { return; }
        if let Err(worker) = ping_with_deadline() {
            self.broker_online = false;
            if !failed_probe_teardown(self.broker, worker, caller_first) {
                log("servicemgr: FATAL permission diagnostic teardown incomplete; no READY\r\n");
                return;
            }
            log("servicemgr: permission diagnostic broker stopped; no false READY\r\n");
            if self.broker_restarts < 2 {
                self.broker_restarts += 1;
                if let Ok(new) = start_broker(self.plan[0]) {
                    self.broker = new;
                    self.broker_online = true;
                    log("servicemgr: diagnostic broker replacement ready on original endpoint\r\n");
                } else {
                    log("servicemgr: diagnostic broker replacement OFFLINE\r\n");
                }
            }
        }
    }
    pub fn event(&mut self, bits: u64) {
        if bits & BROKER_EXIT != 0 && self.broker_online {
            if child_live(self.broker.pid) == Some(false) {
                if finish(self.broker, false).is_err() {
                    log("servicemgr: permission broker Process-cap reap REFUSED\r\n");
                    return;
                }
                self.broker_online = false;
                log("servicemgr: permission broker reaped; old volatile grant retired\r\n");
                if self.broker_restarts < 2 {
                    self.broker_restarts += 1;
                    match start_broker(self.plan[0]) {
                        Ok(new) => { self.broker = new; self.broker_online = true; },
                        Err(()) => log("servicemgr: permission broker OFFLINE after failed restart\r\n"),
                    }
                } else { log("servicemgr: permission broker restart budget exhausted\r\n"); }
            } else { log("servicemgr: ignored forged permission broker exit hint\r\n"); }
        }
        if bits & APP_EXIT != 0 {
            if let Some(child) = self.app {
                if child_live(child.pid) == Some(false) && finish(child, false).is_ok() {
                    self.app = None;
                    log("servicemgr: permission app reaped through held Process cap\r\n");
                }
            }
        }
        // The shared wake is not approval. A forged request can at most
        // cause ONE bounded app attempt; receiver still checks ALLOW.
        if bits & APP_REQUEST != 0 && self.broker_online && self.app.is_none() && self.app_starts < 3 {
            if let Ok(app) = spawn_step(self.plan[1], APP_EXIT) {
                self.app = Some(app);
                self.app_starts += 1;
                log("servicemgr: permission app re-spawned on untrusted wake (broker checks authority)\r\n");
            }
        }
    }
}
