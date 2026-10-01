//! ADR-0053 independent managed receiver lifecycle. Live-cap inventory,
//! literal five-grant attenuation, exact PING+exit+deadline readiness.
//! A shared event badge is a hint; only the held Process cap can reap it.
use arena_servicemgr::inventory::{self, NamedSlot, Probe, SyscallProbe};
use arena_servicemgr::manifest::{self, Dependency, External, Key, Kind, Request, Service, Step};
#[path = "../../abi.rs"]
mod abi;
use abi::*;

const FS: u8 = 14;
const IMAGE: u8 = 17;
const ENDPOINT: u8 = 18;
const MARKER: u8 = 19;
const REGISTRAR: u8 = 20;
const LIFECYCLE: u8 = 21;
const PROBE_IMAGE: u8 = 9;
const EVENTS: u8 = 1;
const PRIVATE: u8 = 7;
const TIMEOUT_US: u64 = 2_000_000;
const K_IMAGE: Key = Key(26);
const K_FS: Key = Key(27);
const K_ENDPOINT: Key = Key(28);
const K_MARKER: Key = Key(29);
const K_REGISTRAR: Key = Key(30);
const K_LIFECYCLE: Key = Key(31);
const SLOTS: [NamedSlot; 6] = [
    NamedSlot {
        key: K_IMAGE,
        slot: IMAGE,
    },
    NamedSlot {
        key: K_FS,
        slot: FS,
    },
    NamedSlot {
        key: K_ENDPOINT,
        slot: ENDPOINT,
    },
    NamedSlot {
        key: K_MARKER,
        slot: MARKER,
    },
    NamedSlot {
        key: K_REGISTRAR,
        slot: REGISTRAR,
    },
    NamedSlot {
        key: K_LIFECYCLE,
        slot: LIFECYCLE,
    },
];
const GRANTS: [Request; 5] = [
    Request {
        key: K_FS,
        kind: Kind::Endpoint,
        rights: manifest::WRITE,
        child_slot: 0,
    },
    Request {
        key: K_ENDPOINT,
        kind: Kind::Endpoint,
        rights: manifest::READ,
        child_slot: 1,
    },
    Request {
        key: K_MARKER,
        kind: Kind::Notification,
        rights: manifest::READ,
        child_slot: 2,
    },
    Request {
        key: K_REGISTRAR,
        kind: Kind::ImageRegistrar,
        rights: manifest::WRITE,
        child_slot: 3,
    },
    Request {
        key: K_LIFECYCLE,
        kind: Kind::Notification,
        rights: manifest::READ,
        child_slot: 4,
    },
];
const DEPS: [Dependency; 1] = [Dependency::External(1)];
const SERVICES: [Service<'static>; 1] = [Service {
    id: 26,
    image: K_IMAGE,
    image_id: 26,
    grants: &GRANTS,
    dependencies: &DEPS,
    restart_limit: 2,
    backoff_us: 1000,
}];
fn log(s: &str) {
    let _ = unsafe { syscall2(SYS_DEBUG_WRITE, s.as_ptr() as u64, s.len() as u64) };
}
#[derive(Clone, Copy)]
struct Child {
    pid: u64,
    slot: u8,
}
pub struct State {
    receiver: Child,
    step: Step,
    restarts: u8,
    online: bool,
}

fn plan() -> Result<Step, ()> {
    let p = SyscallProbe;
    let image = p.describe(IMAGE).map_err(|_| ())?;
    let fs = p.describe(FS).map_err(|_| ())?;
    let endpoint = p.describe(ENDPOINT).map_err(|_| ())?;
    let marker = p.describe(MARKER).map_err(|_| ())?;
    let registrar = p.describe(REGISTRAR).map_err(|_| ())?;
    let lifecycle = p.describe(LIFECYCLE).map_err(|_| ())?;
    let perm_ep = p.describe(15).map_err(|_| ())?;
    let perm_marker = p.describe(16).map_err(|_| ())?;
    let private = p.describe(PRIVATE).map_err(|_| ())?;
    if image.kind != 1
        || image.object != 26
        || image.rights != RIGHTS_READ
        || fs.kind != 2
        || fs.rights != RIGHTS_WRITE | RIGHTS_COPY
        || endpoint.kind != 2
        || endpoint.rights != RIGHTS_READ | RIGHTS_WRITE | RIGHTS_COPY
        || marker.kind != 3
        || marker.rights != RIGHTS_READ | RIGHTS_COPY
        || registrar.kind != 5
        || registrar.object != 0
        || registrar.rights != RIGHTS_WRITE | RIGHTS_COPY
        || lifecycle.kind != 3
        || lifecycle.rights != RIGHTS_READ | RIGHTS_COPY
        || lifecycle.object == marker.object
        || lifecycle.object == perm_marker.object
        || lifecycle.object == private.object
        || private.kind != 3
        || private.rights != RIGHTS_READ | RIGHTS_WRITE | RIGHTS_COPY
        || perm_ep.kind != 2
        || perm_ep.object == endpoint.object
        || perm_marker.kind != 3
        || marker.object == perm_marker.object
        || marker.object == private.object
        || marker.object == endpoint.object
        || endpoint.object == fs.object
        || endpoint.object == perm_ep.object
    {
        return Err(());
    }
    let result = inventory::plan(&SLOTS, &p, &SERVICES, &[External { id: 1, ready: true }])
        .map_err(|_| ())?;
    if result.count != 1 {
        return Err(());
    }
    let step = result.steps[0].ok_or(())?;
    if step.service_id != 26 || step.count != 5 || step.image_slot != IMAGE {
        return Err(());
    }
    Ok(step)
}
fn spawn(step: Step) -> Result<Child, ()> {
    let mut spec = [(0u64, 0u64); 5];
    for i in 0..5 {
        let g = step.grants[i].ok_or(())?;
        if g.child_slot != i as u8 {
            return Err(());
        }
        spec[i] = (g.source_slot as u64, g.rights as u64);
    }
    let pid = unsafe {
        syscall5(
            SYS_SPAWN,
            step.image_slot as u64,
            spec.as_ptr() as u64,
            5,
            EVENTS as u64,
            MGR_BADGE_PKG_EXIT,
        )
    };
    if pid <= 0 {
        return Err(());
    }
    let slot = inventory::child_handle(pid as u64, &SyscallProbe).map_err(|_| ())?;
    Ok(Child {
        pid: pid as u64,
        slot,
    })
}
fn alive(child: Child) -> Result<bool, ()> {
    if inventory::child_handle(child.pid, &SyscallProbe) != Ok(child.slot) {
        return Err(());
    }
    let mut pairs = [0u64; 64];
    let n = unsafe { syscall2(SYS_PROC_LIST, pairs.as_mut_ptr() as u64, 32) };
    if !(0..=32).contains(&n) {
        return Err(());
    }
    pairs[..(n as usize) * 2]
        .chunks_exact(2)
        .find(|p| p[0] == child.pid)
        .map(|p| p[1] != 0)
        .ok_or(())
}
fn finish(child: Child) -> Result<(), ()> {
    let live = alive(child)?;
    inventory::finish(child.slot, live).map_err(|_| ())
}
fn probe(receiver: Child) -> Result<(), Option<Child>> {
    if alive(receiver) != Ok(true) {
        return Err(None);
    }
    // Image20 has exact Endpoint/WRITE + EXISTING PRIVATE Notif/WRITE.
    let grants = [
        (ENDPOINT as u64, RIGHTS_WRITE),
        (PRIVATE as u64, RIGHTS_WRITE),
    ];
    let pid = unsafe {
        syscall5(
            SYS_SPAWN,
            PROBE_IMAGE as u64,
            grants.as_ptr() as u64,
            2,
            PRIVATE as u64,
            MGR_BADGE_PKG_PROBE_EXIT,
        )
    };
    if pid <= 0 {
        return Err(None);
    }
    let worker = Child {
        pid: pid as u64,
        slot: inventory::child_handle(pid as u64, &SyscallProbe).map_err(|_| None)?,
    };
    let timer = unsafe {
        syscall3(
            SYS_TIMER_ARM,
            PRIVATE as u64,
            MGR_BADGE_PKG_PROBE_DEADLINE,
            TIMEOUT_US,
        )
    };
    if timer < 0 {
        return Err(Some(worker));
    }
    let mut bits = 0u64;
    loop {
        let n = unsafe { syscall1(SYS_WAIT, PRIVATE as u64) };
        if n < 0 {
            break;
        }
        bits |= n as u64;
        if bits & MGR_BADGE_PKG_PROBE_DEADLINE != 0
            || bits
                & !(MGR_BADGE_PKG_PROBE_OK
                    | MGR_BADGE_PKG_PROBE_EXIT
                    | MGR_BADGE_PKG_PROBE_DEADLINE)
                != 0
            || bits & (MGR_BADGE_PKG_PROBE_OK | MGR_BADGE_PKG_PROBE_EXIT)
                == MGR_BADGE_PKG_PROBE_OK | MGR_BADGE_PKG_PROBE_EXIT
        {
            break;
        }
    }
    let _ = unsafe { syscall1(SYS_TIMER_CANCEL, timer as u64) };
    let pending = unsafe { syscall1(SYS_TRY_WAIT, PRIVATE as u64) };
    if pending != 0
        || bits != MGR_BADGE_PKG_PROBE_OK | MGR_BADGE_PKG_PROBE_EXIT
        || alive(worker) != Ok(false)
    {
        return Err(Some(worker));
    }
    finish(worker).map_err(|_| Some(worker))?;
    Ok(())
}
fn ready(step: Step) -> Result<Child, ()> {
    let receiver = spawn(step)?;
    match probe(receiver) {
        Ok(()) => {
            log("servicemgr: packaged READY (full boot scan; exact PING + exit + deadline)\r\n");
            Ok(receiver)
        }
        Err(worker) => {
            // Receiver FIRST unblocks a worker parked in IPC_CALL. Reap the
            // worker only after transport has observed receiver teardown.
            if finish(receiver).is_err() {
                log("servicemgr: FATAL packaged teardown refused\r\n");
            }
            if let Some(worker) = worker {
                if finish(worker).is_err() {
                    log("servicemgr: FATAL package probe teardown refused\r\n");
                }
            }
            Err(())
        }
    }
}
pub fn start() -> Result<State, ()> {
    let step = plan()?;
    let receiver = ready(step)?;
    Ok(State {
        receiver,
        step,
        restarts: 0,
        online: true,
    })
}
impl State {
    /// Private admin channel only: never a package endpoint opcode or a
    /// forged shared wake. Manager owns the Process/DESTROY cap; client
    /// keeps the SAME endpoint and must recheck durable AFS1 policy.
    pub fn test_restart(&mut self) {
        if !self.online || self.restarts >= 2 {
            return;
        }
        if finish(self.receiver).is_err() {
            self.online = false;
            log("servicemgr: packaged OFFLINE (authorized live stop refused)\r\n");
            return;
        }
        self.online = false;
        self.restarts += 1;
        log("servicemgr: packaged reaped through held Process cap; no old verification state\r\n");
        match ready(self.step) {
            Ok(child) => {
                self.receiver = child;
                self.online = true;
                log("servicemgr: packaged replacement ready on original endpoint\r\n");
            }
            Err(()) => log("servicemgr: packaged OFFLINE (replacement refused)\r\n"),
        }
    }
    pub fn event(&mut self, bits: u64) {
        if !self.online || bits & MGR_BADGE_PKG_EXIT == 0 {
            return;
        }
        // An untrusted sender can assert this bit, not kill the child:
        // a live Process cap cannot be reaped by a mode-0 exit hint.
        if alive(self.receiver) != Ok(false) {
            return;
        }
        if finish(self.receiver).is_err() {
            self.online = false;
            log("servicemgr: packaged OFFLINE (Process-cap reap refused)\r\n");
            return;
        }
        self.online = false;
        if self.restarts >= 2 {
            log("servicemgr: packaged OFFLINE (restart budget exhausted)\r\n");
            return;
        }
        self.restarts += 1;
        // Every replacement still scans the actual disk before PING.
        match ready(self.step) {
            Ok(child) => {
                self.receiver = child;
                self.online = true;
            }
            Err(()) => log("servicemgr: packaged OFFLINE (replacement refused)\r\n"),
        }
    }
}
