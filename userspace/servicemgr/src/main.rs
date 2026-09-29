//! Phase 8.0 ring-3 service manager (ADR-0037–0047).
//! This program restarts the production stack with a bounded policy;
//! destructive operations need receiving-service diagnostic authority.
//! No manifest request becomes authority merely because this program
//! boots: the kernel installed literal caps, queried by SyscallProbe.
#![no_std]
#![no_main]

use arena_servicemgr::inventory::{self, NamedSlot, Probe, SyscallProbe};
use arena_servicemgr::manifest::{self, Dependency, External, Key, Kind, Request, Service, Step};
use arena_servicemgr::readiness::Gate;
use arena_servicemgr::restart::{Refusal as RestartRefusal, Restart};
use core::panic::PanicInfo;

#[path = "../../abi.rs"]
mod abi;
use abi::*;

const SLOT_IMAGE: u8 = 0;
const SLOT_EVENTS: u8 = 1;
const SLOT_NETD: u8 = 2;
const SLOT_STACK: u8 = 3;
const SLOT_BACKOFF: u8 = 4;
const SLOT_RNGD: u8 = 5;
const SLOT_RNG_READY: u8 = 6;
const SLOT_RESTART: u8 = 7; // private timer object, NOT shared with any driver
const SLOT_ADMIN: u8 = 8; // READ-only private shell-to-manager request
const SLOT_PROBE_IMAGE: u8 = 9; // Image20/READ, full fixture only
const SLOT_RNG_DIAG: u8 = 10; // diagnostic marker, never a data-plane cap
const SLOT_STACK_DIAG: u8 = 11; // inherited READ-only by production stack
const READY_DEADLINE_US: u64 = 2_000_000;

const IMAGE: Key = Key(0);
const NETD: Key = Key(1);
const STACK: Key = Key(2);
const BACKOFF: Key = Key(3);
const RNGD: Key = Key(4);
const STACK_DIAG: Key = Key(5);
const SLOTS: [NamedSlot; 6] = [
    NamedSlot {
        key: IMAGE,
        slot: SLOT_IMAGE,
    },
    NamedSlot {
        key: NETD,
        slot: SLOT_NETD,
    },
    NamedSlot {
        key: STACK,
        slot: SLOT_STACK,
    },
    NamedSlot {
        key: BACKOFF,
        slot: SLOT_BACKOFF,
    },
    NamedSlot {
        key: RNGD,
        slot: SLOT_RNGD,
    },
    NamedSlot {
        key: STACK_DIAG,
        slot: SLOT_STACK_DIAG,
    },
];
const GRANTS: [Request; 5] = [
    Request {
        key: NETD,
        kind: Kind::Endpoint,
        rights: manifest::WRITE,
        child_slot: 0,
    },
    Request {
        key: STACK,
        kind: Kind::Endpoint,
        rights: manifest::READ,
        child_slot: 1,
    },
    Request {
        key: BACKOFF,
        kind: Kind::Notification,
        rights: manifest::READ | manifest::WRITE,
        child_slot: 2,
    },
    Request {
        key: RNGD,
        kind: Kind::Endpoint,
        rights: manifest::WRITE,
        child_slot: 3,
    },
    Request {
        key: STACK_DIAG,
        kind: Kind::Notification,
        rights: manifest::READ,
        child_slot: 4,
    },
];
const DEPS: [Dependency; 2] = [Dependency::External(1), Dependency::External(2)];
const SERVICE: Service<'static> = Service {
    id: 17,
    image: IMAGE,
    image_id: 17,
    grants: &GRANTS,
    dependencies: &DEPS,
    restart_limit: 3,
    backoff_us: 50_000,
};

fn log(msg: &str) {
    // SAFETY: static string; the kernel validates the user buffer and
    // its 256-byte diagnostic bound. All messages below fit.
    let _ = unsafe { syscall2(SYS_DEBUG_WRITE, msg.as_ptr() as u64, msg.len() as u64) };
}

// A single debug-write preserves the three observed endpoint ids on
// one line even when kernel/driver logging is concurrent.
fn report_ids(netd: u64, stack: u64, rngd: u64) {
    const PREFIX: &[u8] = b"servicemgr: live caps image17 netd=";
    let mut buf = [0u8; 128];
    let mut n = 0;
    buf[..PREFIX.len()].copy_from_slice(PREFIX);
    n += PREFIX.len();
    for (idx, value) in [netd, stack, rngd].iter().enumerate() {
        if idx != 0 {
            let name: &[u8] = if idx == 1 { b" stack=" } else { b" rngd=" };
            buf[n..n + name.len()].copy_from_slice(name);
            n += name.len();
        }
        let mut digits = [0u8; 20];
        let mut len = 0;
        let mut v = *value;
        loop {
            digits[len] = b'0' + (v % 10) as u8;
            len += 1;
            v /= 10;
            if v == 0 {
                break;
            }
        }
        while len != 0 {
            len -= 1;
            buf[n] = digits[len];
            n += 1;
        }
    }
    buf[n..n + 2].copy_from_slice(b"\r\n");
    n += 2;
    // SAFETY: stack-owned buffer for this synchronous syscall.
    let _ = unsafe { syscall2(SYS_DEBUG_WRITE, buf.as_ptr() as u64, n as u64) };
}

/// Reject a silent or wrong-kind bootstrap grant before waiting on it.
fn boot_inventory() -> Result<(u64, u64, u64), ()> {
    let probe = SyscallProbe;
    let notify = probe.describe(SLOT_EVENTS).map_err(|_| ())?;
    let rng_ready = probe.describe(SLOT_RNG_READY).map_err(|_| ())?;
    let restart = probe.describe(SLOT_RESTART).map_err(|_| ())?;
    let admin = probe.describe(SLOT_ADMIN).map_err(|_| ())?;
    let image = probe.describe(SLOT_PROBE_IMAGE).map_err(|_| ())?;
    let rng_diag = probe.describe(SLOT_RNG_DIAG).map_err(|_| ())?;
    let stack_diag = probe.describe(SLOT_STACK_DIAG).map_err(|_| ())?;
    if rng_diag.kind != inventory::NOTIFICATION_KIND
        || stack_diag.kind != inventory::NOTIFICATION_KIND
        || rng_diag.rights != RIGHTS_READ | RIGHTS_COPY | RIGHTS_DESTROY
        || stack_diag.rights != RIGHTS_READ | RIGHTS_COPY | RIGHTS_DESTROY
        || rng_diag.object == stack_diag.object
        || rng_diag.object == restart.object || stack_diag.object == restart.object
        || rng_diag.object == admin.object || stack_diag.object == admin.object
    { return Err(()); }
    if image.kind != inventory::IMAGE_KIND
        || image.object != 20
        || image.rights != manifest::READ as u64
    {
        return Err(());
    }
    if admin.kind != inventory::NOTIFICATION_KIND
        || admin.rights != manifest::READ as u64
        || restart.rights & manifest::COPY as u64 == 0
    {
        return Err(());
    }
    for event in [notify, rng_ready, restart] {
        if event.kind != inventory::NOTIFICATION_KIND
            || event.rights & (manifest::READ | manifest::WRITE) as u64
                != (manifest::READ | manifest::WRITE) as u64
        {
            return Err(());
        }
    }
    // Distinct objects, not just disjoint badge labels. A WRITE
    // capability to one must not be able to signal the other.
    if notify.object == rng_ready.object
        || notify.object == restart.object
        || rng_ready.object == restart.object
        || admin.object == notify.object
        || admin.object == rng_ready.object
        || admin.object == restart.object
    {
        return Err(());
    }
    let inventory = inventory::collect(&SLOTS, &probe).map_err(|_| ())?;
    let held = inventory.as_slice();
    if held[0].kind != Kind::Image || held[0].object != 17 || held[0].rights & manifest::READ == 0 {
        return Err(());
    }
    if held[1].kind != Kind::Endpoint
        || held[2].kind != Kind::Endpoint
        || held[3].kind != Kind::Notification
        || held[4].kind != Kind::Endpoint
        || held[5].kind != Kind::Notification
        || held[5].object != stack_diag.object
        || held[3].object == restart.object
        || held[3].object == admin.object
    {
        return Err(());
    }
    Ok((held[1].object, held[2].object, held[4].object))
}

fn wait_one(slot: u8, badge: u64) -> Result<(), ()> {
    // Bound each event on its specific notification object. Driver
    // readiness and the private restart timer have distinct WRITE
    // holders; badge bits alone do not establish origin.
    let timer = unsafe {
        syscall3(
            SYS_TIMER_ARM,
            slot as u64,
            MGR_BADGE_DEADLINE,
            READY_DEADLINE_US,
        )
    };
    if timer < 0 {
        return Err(());
    }
    let mut gate = Gate::new(badge, MGR_BADGE_DEADLINE).map_err(|_| ())?;
    loop {
        let observed = unsafe { syscall1(SYS_WAIT, slot as u64) };
        if observed < 0 {
            return Err(());
        }
        if gate.observe(observed as u64).map_err(|_| ())? {
            let _ = unsafe { syscall1(SYS_TIMER_CANCEL, timer as u64) };
            return Ok(());
        }
    }
}
fn wait_ready() -> Result<(), ()> {
    wait_one(SLOT_EVENTS, MGR_BADGE_NETD_READY)?;
    wait_one(SLOT_RNG_READY, MGR_BADGE_RNGD_READY)
}

/// Cross the real SYS_SPAWN boundary only with a fully validated plan.
/// A positive pid is an observation; the new Process cap is the only
/// lifecycle authority. The stack must signal ready on the SAME
/// notification it later uses for backoff, before we report service up.
fn launch(step: Step) -> Result<(u64, u8), ()> {
    if step.count != 5 {
        return Err(());
    }
    let mut spec = [(0u64, 0u64); 5];
    for (i, item) in spec.iter_mut().enumerate() {
        let grant = step.grants[i].ok_or(())?;
        if grant.child_slot != i as u8 {
            return Err(());
        }
        *item = (grant.source_slot as u64, grant.rights as u64);
    }
    let pid = unsafe {
        syscall5(
            SYS_SPAWN,
            step.image_slot as u64,
            spec.as_ptr() as u64,
            step.count as u64,
            SLOT_EVENTS as u64,
            MGR_BADGE_STACK_EXIT,
        )
    };
    if pid <= 0 {
        return Err(());
    }
    let handle = inventory::child_handle(pid as u64, &SyscallProbe).map_err(|_| ())?;
    if wait_one(SLOT_BACKOFF, MGR_BADGE_STACK_READY).is_err() {
        // Bounded startup: a child that never reaches serve must not
        // remain a live, unowned server on the shared endpoint.
        if inventory::finish(handle, true).is_err() && inventory::finish(handle, false).is_err() {
            log("servicemgr: FAILED to finish a startup-refused child\r\n");
        }
        return Err(());
    }
    Ok((pid as u64, handle))
}

fn report_child(pid: u64) {
    const PREFIX: &[u8] = b"servicemgr: production netstackd READY pid ";
    let mut buf = [0u8; 80];
    let mut n = PREFIX.len();
    buf[..n].copy_from_slice(PREFIX);
    let mut digits = [0u8; 20];
    let mut v = pid;
    let mut len = 0;
    loop {
        digits[len] = b'0' + (v % 10) as u8;
        len += 1;
        v /= 10;
        if v == 0 {
            break;
        }
    }
    while len != 0 {
        len -= 1;
        buf[n] = digits[len];
        n += 1;
    }
    buf[n..n + 2].copy_from_slice(b"\r\n");
    n += 2;
    // SAFETY: own live stack buffer for this synchronous syscall.
    let _ = unsafe { syscall2(SYS_DEBUG_WRITE, buf.as_ptr() as u64, n as u64) };
}

/// A separate user process can block in real driver IPC while the manager
/// still observes a deadline. Success AND exit must precede the timer.
fn probe_dependencies(fixture: u8) -> Result<(), ()> {
    let spec = [
        (
            SLOT_NETD as u64,
            RIGHTS_WRITE | if fixture == 1 { RIGHTS_COPY } else { 0 },
        ),
        (
            SLOT_RNGD as u64,
            RIGHTS_WRITE | if fixture == 2 { RIGHTS_COPY } else { 0 },
        ),
        (SLOT_RESTART as u64, RIGHTS_WRITE),
        (SLOT_STACK_DIAG as u64, RIGHTS_READ | RIGHTS_COPY | RIGHTS_DESTROY), // wrong-object negative
        (SLOT_RNG_DIAG as u64, RIGHTS_READ | RIGHTS_COPY | RIGHTS_DESTROY), // real proof
    ];
    let child = unsafe {
        syscall5(
            SYS_SPAWN,
            SLOT_PROBE_IMAGE as u64,
            spec.as_ptr() as u64,
            if fixture == 0 { 3 } else { 5 },
            SLOT_RESTART as u64,
            MGR_BADGE_PROBE_EXIT,
        )
    };
    if child <= 0 {
        return Err(());
    }
    let handle = inventory::child_handle(child as u64, &SyscallProbe).map_err(|_| ())?;
    let timer = unsafe {
        syscall3(
            SYS_TIMER_ARM,
            SLOT_RESTART as u64,
            MGR_BADGE_PROBE_DEADLINE,
            READY_DEADLINE_US,
        )
    };
    if timer < 0 {
        if inventory::finish(handle, true).is_err() {
            let _ = inventory::finish(handle, false);
        }
        return Err(());
    }
    let mut seen = 0u64;
    loop {
        let b = unsafe { syscall1(SYS_WAIT, SLOT_RESTART as u64) };
        if b <= 0 {
            break;
        }
        seen |= b as u64;
        if seen & MGR_BADGE_PROBE_DEADLINE != 0 {
            break;
        }
        if seen & MGR_BADGE_PROBE_EXIT != 0 {
            break;
        }
        if seen & !(MGR_BADGE_PROBE_OK | MGR_BADGE_PROBE_EXIT) != 0 {
            break;
        }
    }
    let _ = unsafe { syscall1(SYS_TIMER_CANCEL, timer as u64) };
    // If a timeout raced with an exit, timeout wins. A failed worker
    // cannot forge success: only image20 receives private WRITE.
    if seen & MGR_BADGE_PROBE_DEADLINE == 0
        && seen & (MGR_BADGE_PROBE_OK | MGR_BADGE_PROBE_EXIT)
            == (MGR_BADGE_PROBE_OK | MGR_BADGE_PROBE_EXIT)
        && inventory::finish(handle, false).is_ok()
    {
        log("servicemgr: active netd MAC and rngd entropy probes passed; worker reaped\r\n");
        return Ok(());
    }
    // Terminate even a worker stuck in a driver call. Never launch a
    // new production child after a failed or unbounded dependency.
    if inventory::finish(handle, true).is_err() && inventory::finish(handle, false).is_err() {
        log("servicemgr: OFFLINE — dependency probe worker teardown REFUSED\r\n");
        return Err(());
    }
    if seen & MGR_BADGE_PROBE_DEADLINE != 0 {
        log("servicemgr: dependency probe DEADLINE; blocked worker stopped, no child launched\r\n");
    } else {
        log("servicemgr: dependency probe FAILED; worker reaped, no child launched\r\n");
    }
    Err(())
}

fn planned_step(fixture: u8) -> Result<Step, ()> {
    // Probe the actual driver service/device protocols BEFORE making a
    // ready dependency assertion, both on boot and on every restart.
    probe_dependencies(fixture)?;
    // Inspect our REAL caps again on each restart. The static manifest
    // never mints replacement authority.
    let external = [
        External { id: 1, ready: true },
        External { id: 2, ready: true },
    ];
    let plan = inventory::plan(&SLOTS, &SyscallProbe, &[SERVICE], &external).map_err(|_| ())?;
    if plan.count != 1 {
        return Err(());
    }
    plan.steps[0].ok_or(())
}

fn backoff(delay: u64) -> Result<(), ()> {
    // The manager owns this timer: neither an unsolicited driver badge
    // nor a guessed pid grants permission to skip the backoff.
    let timer = unsafe {
        syscall3(
            SYS_TIMER_ARM,
            SLOT_RESTART as u64,
            MGR_BADGE_STACK_BACKOFF,
            delay,
        )
    };
    if timer < 0 {
        return Err(());
    }
    let result = wait_one(SLOT_RESTART, MGR_BADGE_STACK_BACKOFF);
    if result.is_err() {
        let _ = unsafe { syscall1(SYS_TIMER_CANCEL, timer as u64) };
    }
    result
}

/// A diagnostic snapshot distinguishes a forged early exit hint from
/// a BUSY teardown failure. It never grants lifecycle authority: only
/// the held Process cap may authorize SYS_PROC_FINISH.
fn child_still_live(pid: u64) -> Result<bool, ()> {
    let mut entries = [0u64; 64];
    let n = unsafe { syscall2(SYS_PROC_LIST, entries.as_mut_ptr() as u64, 32) };
    if !(0..=32).contains(&n) {
        return Err(());
    }
    for i in 0..n as usize {
        if entries[2 * i] == pid {
            return Ok(entries[2 * i + 1] != 0);
        }
    }
    Err(())
}

fn monitor(mut pid: u64, mut handle: u8, step: Step) -> ! {
    let mut next_probe_fixture = 0u8;
    let mut policy = match Restart::new(pid, step.restart_limit, step.backoff_us) {
        Ok(p) => p,
        Err(_) => {
            log("servicemgr: OFFLINE — invalid restart policy\r\n");
            park();
        }
    };
    loop {
        let badge = unsafe { syscall1(SYS_WAIT, SLOT_EVENTS as u64) };
        if badge < 0 {
            log("servicemgr: OFFLINE — event wait refused\r\n");
            park();
        }
        // Shared events can be forged by netd. STOP authority is the
        // separate private notification (shell/W, manager/R), taken
        // nonblocking so a forged wake cannot stall the monitor.
        let mut force_live = false;
        if badge as u64 & MGR_BADGE_ADMIN_WAKE != 0 {
            let request = unsafe { syscall1(SYS_TRY_WAIT, SLOT_ADMIN as u64) };
            if request < 0 {
                log("servicemgr: OFFLINE — private admin channel refused\r\n");
                park();
            }
            if request as u64 == MGR_BADGE_ADMIN_STOP
                || request as u64 == MGR_BADGE_ADMIN_DEPFAIL
                || request as u64 == MGR_BADGE_ADMIN_DEPSTALL
            {
                let fixture = if request as u64 == MGR_BADGE_ADMIN_DEPFAIL {
                    1
                } else if request as u64 == MGR_BADGE_ADMIN_DEPSTALL {
                    2
                } else {
                    0
                };
                match child_still_live(pid) {
                    Ok(true) => {
                        force_live = true;
                        next_probe_fixture = fixture;
                    }
                    Ok(false) => {
                        // Natural exit won the race. Mode 0 only;
                        // never call a dead reap a forced live stop.
                        log("servicemgr: admin request raced natural child exit\r\n");
                    }
                    Err(()) => {
                        log("servicemgr: OFFLINE — no observable held child for admin stop\r\n");
                        park();
                    }
                }
            } else if request != 0 {
                log("servicemgr: refused unknown private admin request\r\n");
            } else {
                log("servicemgr: ignored unauthenticated shared wake hint\r\n");
            }
        }
        if !force_live && badge as u64 & MGR_BADGE_STACK_EXIT == 0 {
            continue;
        }
        // A badge is a hint, NEVER authority. Drivers sharing this
        // notification can assert the bit; mode 0 MUST reject a live
        // child even when the caller really holds the Process cap.
        if inventory::child_handle(pid, &SyscallProbe) != Ok(handle) {
            log("servicemgr: OFFLINE — unique child Process cap missing\r\n");
            park();
        }
        let mut finished = inventory::finish(handle, force_live);
        if force_live && finished == Err(STATUS_BUSY) && child_still_live(pid) == Ok(false) {
            // Single CPU but the child may exit between the user's
            // diagnostic and the mode-1 syscall. Kernel mode 1 refuses
            // dead children; fall back to the authorized mode-0 reap.
            force_live = false;
            finished = inventory::finish(handle, false);
        }
        match finished {
            Err(STATUS_BUSY) => {
                if force_live {
                    log("servicemgr: OFFLINE — forced live stop BUSY\r\n");
                    park();
                }
                if child_still_live(pid) == Ok(true) {
                    log("servicemgr: ignored exit hint: held child is still live\r\n");
                    continue;
                }
                log("servicemgr: OFFLINE — child dead but Process-cap finish BUSY\r\n");
                park();
            }
            Err(_) => {
                log("servicemgr: OFFLINE — Process-cap reap REFUSED\r\n");
                park();
            }
            Ok(()) => {
                if force_live {
                    log(
                        "servicemgr: forcibly stopped LIVE production child through held Process cap\r\n",
                    );
                }
            }
        }
        let delay = match policy.exited(pid) {
            Ok(delay) => delay,
            Err(RestartRefusal::BudgetExhausted) => {
                log("servicemgr: OFFLINE — bounded restart budget exhausted\r\n");
                park();
            }
            Err(_) => {
                log("servicemgr: OFFLINE — child lifecycle mismatch\r\n");
                park();
            }
        };
        log("servicemgr: production child reaped through Process cap; bounded backoff\r\n");
        if backoff(delay).is_err() {
            log("servicemgr: OFFLINE — restart backoff deadline/refusal\r\n");
            park();
        }
        let next = match planned_step(next_probe_fixture) {
            Ok(s) if s.service_id == step.service_id => s,
            _ => {
                log("servicemgr: OFFLINE — restart authority/dependency plan refused\r\n");
                park();
            }
        };
        match launch(next) {
            Ok((new_pid, new_handle)) => {
                if policy.ready(new_pid).is_err() {
                    log("servicemgr: OFFLINE — restart state mismatch\r\n");
                    park();
                }
                pid = new_pid;
                handle = new_handle;
                log("servicemgr: restarted production netstackd on original endpoint\r\n");
                report_child(pid);
            }
            Err(()) => {
                log("servicemgr: OFFLINE — replacement spawn/readiness refused\r\n");
                park();
            }
        }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn _start() -> ! {
    log("servicemgr: ring-3 boot (Phase 8.0 bounded-restart substrate)\r\n");
    let ids = match boot_inventory() {
        Ok(ids) => ids,
        Err(()) => {
            log("servicemgr: OFFLINE — missing or invalid boot grants; no child spawned\r\n");
            park();
        }
    };
    report_ids(ids.0, ids.1, ids.2);
    if wait_ready().is_err() {
        log("servicemgr: OFFLINE — device readiness deadline/refusal; no child spawned\r\n");
        park();
    }
    let step = match planned_step(0) {
        Ok(step) => step,
        Err(()) => {
            log("servicemgr: OFFLINE — live cap/policy mismatch; no child spawned\r\n");
            park();
        }
    };
    if step.service_id != 17 || step.count != 5 {
        log("servicemgr: OFFLINE — unexpected plan shape; no child spawned\r\n");
        park();
    }
    log("servicemgr: policy validated from live caps and ready drivers\r\n");
    match launch(step) {
        Ok((pid, handle)) => {
            report_child(pid);
            monitor(pid, handle, step);
        }
        Err(()) => {
            log("servicemgr: OFFLINE — spawn or stack readiness refused\r\n");
            park();
        }
    }
}

fn park() -> ! {
    loop {
        // SAFETY: the manager's own READ notification; a dead or
        // refused object cannot grant any capabilities or launch a child.
        let status = unsafe { syscall1(SYS_WAIT, SLOT_EVENTS as u64) };
        if status < 0 {
            log("servicemgr: wait refused — stopping\r\n");
            unsafe { syscall2(SYS_THREAD_EXIT, 98, 0) };
        }
    }
}

#[panic_handler]
fn panic(_info: &PanicInfo<'_>) -> ! {
    log("servicemgr: PANIC\r\n");
    unsafe { syscall2(SYS_THREAD_EXIT, 99, 0) };
    loop {
        core::hint::spin_loop();
    }
}
