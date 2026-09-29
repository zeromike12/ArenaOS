//! Phase 8.0 ring-3 bootstrap/inventory/readiness checkpoint (ADR-0037).
//! This program does NOT yet spawn netstackd or supervise a child.
//! No manifest request becomes authority merely because this program
//! boots: the kernel installed literal caps, queried by SyscallProbe.
#![no_std]
#![no_main]

use arena_servicemgr::inventory::{self, NamedSlot, Probe, SyscallProbe};
use arena_servicemgr::manifest::{self, Dependency, External, Key, Kind, Request, Service};
use arena_servicemgr::readiness::Gate;
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
const READY_DEADLINE_US: u64 = 2_000_000;

const IMAGE: Key = Key(0);
const NETD: Key = Key(1);
const STACK: Key = Key(2);
const BACKOFF: Key = Key(3);
const RNGD: Key = Key(4);
const SLOTS: [NamedSlot; 5] = [
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
];
const GRANTS: [Request; 4] = [
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
    for event in [notify, rng_ready] {
        if event.kind != inventory::NOTIFICATION_KIND
            || event.rights & (manifest::READ | manifest::WRITE) as u64
                != (manifest::READ | manifest::WRITE) as u64
        {
            return Err(());
        }
    }
    // Distinct objects, not just disjoint badge labels. A WRITE
    // capability to one must not be able to signal the other.
    if notify.object == rng_ready.object {
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
    {
        return Err(());
    }
    Ok((held[1].object, held[2].object, held[4].object))
}

fn wait_one(slot: u8, badge: u64) -> Result<(), ()> {
    // Bound EACH driver separately. Since WRITE authority is on two
    // distinct notification objects, neither can forge the other's
    // ready event by sending its badge bit on a shared channel.
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

#[unsafe(no_mangle)]
pub extern "C" fn _start() -> ! {
    log("servicemgr: ring-3 boot (Phase 8.0 substrate)\r\n");
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
    let external = [
        External { id: 1, ready: true },
        External { id: 2, ready: true },
    ];
    let plan = match inventory::plan(&SLOTS, &SyscallProbe, &[SERVICE], &external) {
        Ok(plan) => plan,
        Err(_) => {
            log("servicemgr: OFFLINE — live cap/policy mismatch; no child spawned\r\n");
            park();
        }
    };
    if plan.count != 1 || plan.steps[0].unwrap().count != 4 {
        log("servicemgr: OFFLINE — unexpected plan shape; no child spawned\r\n");
        park();
    }
    log(
        "servicemgr: policy validated from live caps and ready drivers; spawn/restart NOT YET CONNECTED\r\n",
    );
    park();
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
