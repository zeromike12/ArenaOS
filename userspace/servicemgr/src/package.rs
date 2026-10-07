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
const INSTALL_AUTH: u8 = 127;
const _: () = assert!(CAP_SLOTS == 128);
const _: () = assert!(INSTALL_AUTH as usize == CAP_SLOTS - 1);
const BADGED_ENDPOINT_KIND: u64 = 12;
const REGISTRAR: u8 = 20;
const LIFECYCLE: u8 = 21;
const PROBE_IMAGE: u8 = 9;
const EVENTS: u8 = 1;
const PRIVATE: u8 = 7;
const TIMEOUT_US: u64 = 2_000_000;
// Diagnostic SELECT executes concurrent signed FS clients on a full
// 32-object platter. The production readiness/restart budget is unchanged.
const SELECT_CHILD_TIMEOUT_US: u64 = 15_000_000;
// Process-list snapshots are large enough to exhaust this service's single
// initial ring-3 stack when nested inside its signed-package control path.
// Keep the bounded query buffers in the manager's writable image instead.
static mut PROCESS_ROWS_QUERY: [u64; 128] = [0; 128];
static mut PROCESS_ROWS_BEFORE: [u64; 128] = [0; 128];
static mut PROCESS_ROWS_AFTER: [u64; 128] = [0; 128];
static mut IMAGE_CAPS_BEFORE: [u64; CAP_SLOTS * 3] = [0; CAP_SLOTS * 3];
static mut IMAGE_CAPS_AFTER: [u64; CAP_SLOTS * 3] = [0; CAP_SLOTS * 3];
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
/// Preserve the historical low32 manager-cap receipt used by Process-cap
/// capacity proofs, and separately report the out-of-band late APB1 slot.
fn observed_caps(label: &str) {
    let mut count = 0u64;
    for slot in 0..32u64 {
        let mut d = [0u64; 3];
        if unsafe { syscall2(SYS_CAP_DESCRIBE, slot, d.as_mut_ptr() as u64) } == 0 {
            count += 1;
        }
    }
    log_line(|o| {
        o.str("servicemgr: observed cap occupancy ");
        o.str(label);
        o.str("=");
        o.u64(count);
        o.crlf();
    });
    let mut upper = 0u64;
    for slot in 32..INSTALL_AUTH {
        let mut d = [0u64; 3];
        if unsafe { syscall2(SYS_CAP_DESCRIBE, slot as u64, d.as_mut_ptr() as u64) } == 0 {
            upper += 1;
        }
    }
    log_line(|o| {
        o.str("servicemgr: extended cap occupancy [32,127)=");
        o.u64(upper);
        o.crlf();
    });
    let mut install = [0u64; 3];
    let present = unsafe {
        syscall2(
            SYS_CAP_DESCRIBE,
            INSTALL_AUTH as u64,
            install.as_mut_ptr() as u64,
        )
    } == 0;
    log_line(|o| {
        o.str("servicemgr: reserved APB1 slot127 descriptor=");
        o.u64(if present { 1 } else { 0 });
        o.str(" kind=");
        o.u64(if present { install[0] } else { 0 });
        o.str(" rights=");
        o.u64(if present { install[2] } else { 0 });
        o.crlf();
    });
}
fn process_list(buffer: u64) -> Result<usize, ()> {
    let count = unsafe { syscall2(SYS_PROC_LIST, buffer, 64) };
    if !(0..=64).contains(&count) {
        return Err(());
    }
    Ok(count as usize)
}

/// Take an exact inventory of all 128 manager cap slots without using the
/// one-page service stack for the snapshot. Empty-slot describe must preserve
/// its caller buffer, just like the stale-generation probes.
fn cap_snapshot(destination: *mut u64) -> Result<(), ()> {
    for slot in 0..CAP_SLOTS {
        let descriptor = unsafe { destination.add(slot * 3) };
        for word in 0..3 {
            unsafe { core::ptr::write_volatile(descriptor.add(word), u64::MAX) };
        }
        let rc = unsafe { syscall2(SYS_CAP_DESCRIBE, slot as u64, descriptor as u64) };
        if rc == -2 {
            if (0..3)
                .any(|word| unsafe { core::ptr::read_volatile(descriptor.add(word)) } != u64::MAX)
            {
                return Err(());
            }
        } else if rc != 0 {
            return Err(());
        }
    }
    Ok(())
}

fn cap_snapshots_equal() -> bool {
    unsafe {
        core::slice::from_raw_parts(
            core::ptr::addr_of!(IMAGE_CAPS_BEFORE).cast::<u64>(),
            CAP_SLOTS * 3,
        ) == core::slice::from_raw_parts(
            core::ptr::addr_of!(IMAGE_CAPS_AFTER).cast::<u64>(),
            CAP_SLOTS * 3,
        )
    }
}

fn retire_image_cap(image: ImageCap) -> bool {
    let revoke_status =
        unsafe { syscall6(SYS_IMAGE_REVOKE, REGISTRAR as u64, image.id, 0, 0, 0, 0) };
    let drop_status = unsafe { syscall1(SYS_CAP_DESTROY, image.slot as u64) };
    if revoke_status != 0 || drop_status != 0 {
        log_line(|o| {
            o.str("servicemgr: Image cleanup id=");
            o.u64(image.id);
            o.str(" slot=");
            o.u64(image.slot as u64);
            o.str(" revoke=");
            o.u64(revoke_status as i64 as u64);
            o.str(" drop=");
            o.u64(drop_status as i64 as u64);
            o.crlf();
        });
    }
    revoke_status == 0 && drop_status == 0
}

fn discard_returned_image_cap(slot: u64) {
    if slot >= CAP_SLOTS as u64 {
        return;
    }
    let mut descriptor = [0u64; 3];
    if unsafe { syscall2(SYS_CAP_DESCRIBE, slot, descriptor.as_mut_ptr() as u64) } == 0
        && descriptor[0] == 1
    {
        let _ = unsafe {
            syscall6(
                SYS_IMAGE_REVOKE,
                REGISTRAR as u64,
                descriptor[1],
                0,
                0,
                0,
                0,
            )
        };
    }
    let _ = unsafe { syscall1(SYS_CAP_DESTROY, slot) };
}

fn cleanup_image_table_refs() -> bool {
    let entries = core::ptr::addr_of_mut!(IMAGE_TABLE_REFS).cast::<Option<ImageCap>>();
    let mut clean = true;
    for index in 0..MAX_DYNAMIC_IMAGES {
        if let Some(image) = unsafe { core::ptr::read_volatile(entries.add(index)) } {
            clean &= retire_image_cap(image);
            unsafe { core::ptr::write_volatile(entries.add(index), None) };
        }
    }
    clean
}

fn cleanup_launched_image_refs(second: ImageCap) -> bool {
    let extras_clean = cleanup_image_table_refs();
    if !extras_clean {
        log("servicemgr: Image cleanup failed for full-table extras\r\n");
    }
    let second_clean = retire_image_cap(second);
    if !second_clean {
        log("servicemgr: Image cleanup failed for second Image cap\r\n");
    }
    extras_clean && second_clean
}

/// Fill the dynamic Image quota with real ring-3 children, then prove the next
/// spawn refuses without changing the 128-slot cap table or 64-process table.
fn capacity_refuses(image: u64, already_unretired: usize) -> Result<(), ()> {
    let extra = MAX_DYNAMIC_CHILDREN
        .checked_sub(already_unretired)
        .filter(|count| *count > 0)
        .ok_or(())?;
    const BADGE: u64 = 1 << 60;
    // This is the full native cap table. Historical manager receipts still
    // count low32 separately; this proof must see children above that window.
    fn caps() -> usize {
        (0..CAP_SLOTS)
            .filter(|s| {
                let mut d = [0u64; 3];
                (unsafe { syscall2(SYS_CAP_DESCRIBE, *s as u64, d.as_mut_ptr() as u64) }) == 0
            })
            .count()
    }
    let initial_caps = caps();
    let children = core::ptr::addr_of_mut!(CAPACITY_CHILDREN).cast::<Option<Child>>();
    for index in 0..extra {
        unsafe { core::ptr::write_volatile(children.add(index), None) };
    }
    let mut failed = false;
    let grants = [(ENDPOINT as u64, RIGHTS_WRITE)];
    for index in 0..extra {
        let pid = unsafe { syscall5(SYS_SPAWN, image, grants.as_ptr() as u64, 1, CAP_NONE, 0) };
        if pid <= 0 {
            log_line(|o| {
                o.str("servicemgr: dynamic capacity spawn failed at extra=");
                o.u64(index as u64);
                o.str(" status=");
                o.u64(pid as i64 as u64);
                o.crlf();
            });
            failed = true;
            break;
        }
        let slot = inventory::child_handle(pid as u64, &SyscallProbe).map_err(|_| ())?;
        unsafe {
            core::ptr::write_volatile(
                children.add(index),
                Some(Child {
                    pid: pid as u64,
                    slot,
                }),
            )
        };
    }
    let mut pending = 0u64;
    if !failed {
        let timer = unsafe { syscall3(SYS_TIMER_ARM, PRIVATE as u64, BADGE, 20_000) };
        if timer < 0 {
            failed = true;
        } else {
            loop {
                let bits = unsafe { syscall1(SYS_WAIT, PRIVATE as u64) };
                if bits < 0 {
                    failed = true;
                    break;
                }
                pending |= bits as u64 & !BADGE;
                if bits as u64 & BADGE != 0 {
                    break;
                }
            }
            let _ = unsafe { syscall1(SYS_TIMER_CANCEL, timer as u64) };
        }
    }
    if !failed {
        let resident_caps = caps();
        if resident_caps >= CAP_SLOTS {
            failed = true;
        } // refusal must have a free Process-cap slot
        let before_count = process_list(core::ptr::addr_of_mut!(PROCESS_ROWS_BEFORE) as u64)?;
        let attempt = unsafe { syscall5(SYS_SPAWN, image, grants.as_ptr() as u64, 1, CAP_NONE, 0) };
        if attempt != STATUS_BUSY {
            failed = true;
            if attempt > 0 {
                let slot =
                    inventory::child_handle(attempt as u64, &SyscallProbe).map_err(|_| ())?;
                let _ = finish(Child {
                    pid: attempt as u64,
                    slot,
                });
            }
        }
        let after_count = process_list(core::ptr::addr_of_mut!(PROCESS_ROWS_AFTER) as u64)?;
        let processes_unchanged = before_count == after_count
            && unsafe {
                core::slice::from_raw_parts(
                    core::ptr::addr_of!(PROCESS_ROWS_BEFORE).cast::<u64>(),
                    before_count * 2,
                ) == core::slice::from_raw_parts(
                    core::ptr::addr_of!(PROCESS_ROWS_AFTER).cast::<u64>(),
                    after_count * 2,
                )
            };
        if caps() != resident_caps || !processes_unchanged {
            failed = true
        }
        let mut live = 0u64;
        for index in 0..extra {
            if let Some(child) = unsafe { core::ptr::read_volatile(children.add(index)) }
                && alive(child) == Ok(true)
            {
                live += 1
            }
        }
        log_line(|o| {
            o.str("servicemgr: dynamic capacity real extra children alive=");
            o.u64(live);
            o.str("; resident caps=");
            o.u64(resident_caps as u64);
            o.str("; unretired children=");
            o.u64(MAX_DYNAMIC_CHILDREN as u64);
            o.str("; next refused; caps/processes unchanged");
            o.crlf();
        });
    }
    for index in 0..extra {
        if let Some(child) = unsafe { core::ptr::read_volatile(children.add(index)) } {
            if finish(child).is_err() {
                failed = true
            }
        }
    }
    for index in 0..extra {
        unsafe { core::ptr::write_volatile(children.add(index), None) };
    }
    if caps() != initial_caps {
        failed = true
    }
    if pending != 0 && unsafe { syscall2(SYS_NOTIFY, PRIVATE as u64, pending) } != 0 {
        failed = true
    }
    if !failed {
        let next = unsafe { syscall5(SYS_SPAWN, image, grants.as_ptr() as u64, 1, CAP_NONE, 0) };
        if next <= 0 {
            failed = true;
        } else {
            let child = Child {
                pid: next as u64,
                slot: inventory::child_handle(next as u64, &SyscallProbe).map_err(|_| ())?,
            };
            if finish(child).is_err() {
                failed = true
            }
        }
    }
    if caps() != initial_caps {
        failed = true
    }
    if failed { Err(()) } else { Ok(()) }
}

#[derive(Clone, Copy)]
struct Child {
    pid: u64,
    slot: u8,
}
static mut CAPACITY_CHILDREN: [Option<Child>; MAX_DYNAMIC_CHILDREN] = [None; MAX_DYNAMIC_CHILDREN];
#[derive(Clone, Copy)]
struct ImageCap {
    id: u64,
    slot: u8,
}
static mut IMAGE_TABLE_REFS: [Option<ImageCap>; MAX_DYNAMIC_IMAGES] = [None; MAX_DYNAMIC_IMAGES];
#[derive(Clone, Copy)]
struct CapacityImage {
    id: u64,
    slot: u8,
}
pub struct State {
    receiver: Child,
    step: Step,
    restarts: u8,
    online: bool,
    install_handed_off: bool,
    test_last_active: Option<[u8; 32]>,
    test_old_image: Option<(u64, u64)>,
    test_old_child: Option<Child>,
    test_maximal: bool,
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
        || lifecycle.rights != RIGHTS_READ | RIGHTS_COPY | RIGHTS_DESTROY
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
    let n = process_list(core::ptr::addr_of_mut!(PROCESS_ROWS_QUERY) as u64)?;
    if n > 64 {
        return Err(());
    }
    unsafe {
        core::slice::from_raw_parts(core::ptr::addr_of!(PROCESS_ROWS_QUERY).cast::<u64>(), n * 2)
    }
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

/// Exercise the actual packaged receiver with a hostile handoff shape.
/// The wrong-kind Notification is transferred directly from an existing
/// held manager slot, so the sender creates no untracked temporary cap.
/// Neither numeric slot 127 nor a descriptor word is used as authority.
fn expect_handoff_denied(receiver: Child, source: u8) -> Result<(u64, u64), ()> {
    if alive(receiver) != Ok(true) {
        return Err(());
    }
    let source_desc = SyscallProbe.describe(source).map_err(|_| ())?;
    if source_desc.rights & RIGHTS_COPY == 0 {
        return Err(());
    }
    let mut message = [0u8; 64];
    let mut out = [0, 0, CAP_NONE];
    let rc = unsafe {
        syscall6(
            SYS_IPC_CALL,
            ENDPOINT as u64,
            PKG_OP_INSTALL_AUTH_HANDOFF,
            source_desc.object,
            source as u64,
            out.as_mut_ptr() as u64,
            message.as_mut_ptr() as u64,
        )
    };
    if out[2] != CAP_NONE {
        let _ = unsafe { syscall1(SYS_CAP_DESTROY, out[2]) };
    }
    if rc != 0 || out != [PKG_DENY, 0, CAP_NONE] || SyscallProbe.describe(source) != Ok(source_desc)
    {
        log_line(|o| {
            o.str("servicemgr: APB1 refusal probe failed kind=");
            o.u64(source_desc.kind);
            o.str(" rights=");
            o.u64(source_desc.rights);
            o.str(" rc=");
            o.i64(rc);
            o.str(" reply=");
            o.u64(out[0]);
            o.str("/");
            o.u64(out[1]);
            o.str(" cap=");
            o.u64(out[2]);
            o.crlf();
        });
        return Err(());
    }
    Ok((source_desc.kind, source_desc.rights))
}

fn prove_handoff_refusals(receiver: Child) -> Result<(), ()> {
    let wrong_kind = expect_handoff_denied(receiver, MARKER)?;
    if wrong_kind.0 == BADGED_ENDPOINT_KIND {
        return Err(());
    }
    log_line(|o| {
        o.str("servicemgr: APB1 wrong-kind handoff refused kind=");
        o.u64(wrong_kind.0);
        o.str(" rights=");
        o.u64(wrong_kind.1);
        o.crlf();
    });
    Ok(())
}

fn handoff_install_authority(receiver: Child) -> Result<bool, ()> {
    if alive(receiver) != Ok(true) {
        return Err(());
    }
    let occupied = unsafe { syscall6(SYS_CAP_OCCUPIED, INSTALL_AUTH as u64, 0, 0, 0, 0, 0) };
    if occupied == 0 {
        return Ok(false);
    }
    if occupied != 1 {
        return Err(());
    }
    let authority = SyscallProbe.describe(INSTALL_AUTH).map_err(|_| ())?;
    if authority.kind != BADGED_ENDPOINT_KIND || authority.rights != RIGHTS_WRITE | RIGHTS_COPY {
        return Err(());
    }
    // Record the late authority only after the live held capability has been
    // described and its exact kind/rights accepted. The historical low-32
    // metric remains the same, while slot127 is separately visible.
    observed_caps("apb1-authority-held");
    prove_handoff_refusals(receiver)?;
    let mut message = [0u8; 64];
    let mut out = [0, 0, CAP_NONE];
    let rc = unsafe {
        syscall6(
            SYS_IPC_CALL,
            ENDPOINT as u64,
            PKG_OP_INSTALL_AUTH_HANDOFF,
            authority.object,
            INSTALL_AUTH as u64,
            out.as_mut_ptr() as u64,
            message.as_mut_ptr() as u64,
        )
    };
    if rc != 0 || out[0] != PKG_OK || out[1] != 0 || out[2] != CAP_NONE {
        if out[2] != CAP_NONE {
            let _ = unsafe { syscall1(SYS_CAP_DESTROY, out[2]) };
        }
        return Err(());
    }
    log("servicemgr: APB1 install-only BadgedEndpoint handed to READY packaged in late slot5\r\n");
    Ok(true)
}

fn ready(step: Step) -> Result<(Child, bool), ()> {
    let receiver = spawn(step)?;
    match probe(receiver) {
        Ok(()) => {
            log("servicemgr: packaged READY (full boot scan; exact PING + exit + deadline)\r\n");
            match handoff_install_authority(receiver) {
                Ok(handed_off) => Ok((receiver, handed_off)),
                Err(()) => {
                    if finish(receiver).is_err() {
                        log("servicemgr: FATAL packaged failure after APB1 handoff refused\r\n");
                    }
                    Err(())
                }
            }
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
    let (receiver, install_handed_off) = ready(step)?;
    observed_caps("packaged-ready");
    Ok(State {
        receiver,
        step,
        restarts: 0,
        online: true,
        install_handed_off,
        test_last_active: None,
        test_old_image: None,
        test_old_child: None,
        test_maximal: false,
    })
}
impl State {
    /// Trusted serial diagnostic selects a real signed Image through the
    /// unchanged package transaction, then delegates that held executable to
    /// the ordinary desktop broker. It grants no application function scope.
    pub fn test_graphical(&mut self, revoke: bool) {
        let result = (|| -> Result<(), ()> {
            let mut ep = [0; 3];
            if !self.online
                || alive(self.receiver) != Ok(true)
                || unsafe { syscall2(SYS_CAP_DESCRIBE, 31, ep.as_mut_ptr() as u64) } != 0
                || ep[0] != 2
                || ep[2] != RIGHTS_WRITE | RIGHTS_COPY
            {
                return Err(());
            }
            if self.test_old_image.is_none() {
                if revoke {
                    return Err(());
                }
                fn package(
                    op: u64,
                    arg: u64,
                    digest: &[u8; 32],
                ) -> Result<([u64; 3], [u8; 64]), ()> {
                    let mut msg = [0u8; 64];
                    msg[..8].copy_from_slice(b"app.test");
                    msg[32..].copy_from_slice(digest);
                    let mut out = [0, 0, CAP_NONE];
                    if unsafe {
                        syscall6(
                            SYS_IPC_CALL,
                            ENDPOINT as u64,
                            op,
                            arg,
                            if op == PKG_OP_QUERY {
                                CAP_NONE
                            } else {
                                LIFECYCLE as u64
                            },
                            out.as_mut_ptr() as u64,
                            msg.as_mut_ptr() as u64,
                        )
                    } != 0
                    {
                        return Err(());
                    }
                    Ok((out, msg))
                }
                fn no_cap(out: [u64; 3]) -> Result<(), ()> {
                    if out[2] == CAP_NONE {
                        Ok(())
                    } else {
                        let _ = unsafe { syscall1(SYS_CAP_DESTROY, out[2]) };
                        Err(())
                    }
                }
                let (out, msg) = package(PKG_OP_QUERY, 0, &[0; 32])?;
                no_cap(out)?;
                if out[0] != PKG_ELIGIBLE || out[1] == 0 || msg[..32] == [0; 32] {
                    return Err(());
                }
                let digest: [u8; 32] = msg[..32].try_into().map_err(|_| ())?;
                let (out, msg) = package(PKG_OP_INSTALL, 1, &digest)?;
                no_cap(out)?;
                if out[0] != PKG_INSTALLED
                    || out[1] != 1
                    || msg[..32] != digest
                    || msg[32..] == [0; 32]
                {
                    return Err(());
                }
                let install: [u8; 32] = msg[32..].try_into().map_err(|_| ())?;
                let (out, msg) = package(PKG_OP_SELECT_PREPARE, 1, &install)?;
                no_cap(out)?;
                if out[0] != PKG_PREPARED
                    || out[1] & 255 != 1
                    || msg[..32] != digest
                    || msg[32..] != install
                {
                    return Err(());
                }
                let token = out[1];
                let (out, msg) = package(PKG_OP_SELECT_COMMIT, token, &install)?;
                if out[2] == CAP_NONE {
                    return Err(());
                }
                let mut image = [0; 3];
                if unsafe { syscall2(SYS_CAP_DESCRIBE, out[2], image.as_mut_ptr() as u64) } != 0
                    || image[0] != 1
                    || image[1] < 27
                    || image[2] != RIGHTS_READ | RIGHTS_COPY | RIGHTS_DESTROY
                    || out[0] != PKG_ACTIVE
                    || out[1] >> 8 != image[1]
                    || out[1] & 255 != 1
                    || msg[..32] != digest
                    || msg[32..] == [0; 32]
                {
                    let _ = unsafe { syscall1(SYS_CAP_DESTROY, out[2]) };
                    return Err(());
                }
                self.test_old_image = Some((image[1], out[2]));
                self.test_last_active = Some(msg[32..].try_into().map_err(|_| ())?);
            }
            let (id, image) = self.test_old_image.ok_or(())?;
            if revoke {
                if unsafe { syscall6(SYS_IMAGE_REVOKE, REGISTRAR as u64, id, 0, 0, 0, 0) } != 0 {
                    return Err(());
                }
                log(
                    "servicemgr: graphical dynamic Image full-ID revoked; existing child copied pages retained\r\n",
                );
            }
            let mut msg = [0u8; 64];
            msg[..4].copy_from_slice(b"ASVC");
            msg[4] = 1;
            msg[5] = 10;
            let mut out = [0, 0, CAP_NONE];
            let rc = unsafe {
                syscall6(
                    SYS_IPC_CALL,
                    31,
                    0,
                    0,
                    image,
                    out.as_mut_ptr() as u64,
                    msg.as_mut_ptr() as u64,
                )
            };
            if out[2] != CAP_NONE {
                let _ = unsafe { syscall1(SYS_CAP_DESTROY, out[2]) };
                return Err(());
            }
            if revoke {
                if rc >= 0 && out[0] == 0 {
                    return Err(());
                }
                log("servicemgr: revoked graphical Image cannot spawn again PASS\r\n");
            } else if rc != 0 || out[0] != 0 {
                log("servicemgr: graphical Image launch bounded refusal\r\n");
            } else {
                log("servicemgr: real signed Image delegated for broker-owned graphical spawn\r\n");
            }
            Ok(())
        })();
        if result.is_err() {
            log("servicemgr: GRAPHICALTEST refused\r\n");
        }
    }
    /// Test fixture only: Power-shell private manager request, single public
    /// app.test namespace, exact signed QUERY digest then receiver-verified
    /// lifecycle-marker INSTALL. This is not a generic package picker or a
    /// shortcut from STAGE to Image/activation. No extra endpoint exists.
    pub fn test_install_fixture(&mut self) {
        if !self.online || alive(self.receiver) != Ok(true) {
            log("servicemgr: INSTALLTEST refused: packaged not live\r\n");
            return;
        }
        let mut marker = [0u64; 3];
        let mut old = [0u64; 3];
        if unsafe {
            syscall2(
                SYS_CAP_DESCRIBE,
                LIFECYCLE as u64,
                marker.as_mut_ptr() as u64,
            )
        } != 0
            || marker != [3, marker[1], RIGHTS_READ | RIGHTS_COPY | RIGHTS_DESTROY]
            || unsafe { syscall2(SYS_CAP_DESCRIBE, MARKER as u64, old.as_mut_ptr() as u64) } != 0
            || old[0] != 3
            || old[1] == marker[1]
        {
            log("servicemgr: INSTALLTEST refused: lifecycle bearer inventory\r\n");
            return;
        }
        let mut msg = [0u8; MSG_BYTES];
        msg[..8].copy_from_slice(b"app.test");
        let mut reply = [0u64, 0, CAP_NONE];
        let rc = unsafe {
            syscall6(
                SYS_IPC_CALL,
                ENDPOINT as u64,
                PKG_OP_QUERY,
                0,
                CAP_NONE,
                reply.as_mut_ptr() as u64,
                msg.as_mut_ptr() as u64,
            )
        };
        if reply[2] != CAP_NONE {
            let _ = unsafe { syscall1(SYS_CAP_DESTROY, reply[2]) };
            log("servicemgr: INSTALLTEST refused: unexpected QUERY cap\r\n");
            return;
        }
        if rc != 0
            || reply[0] != PKG_ELIGIBLE
            || reply[1] == 0
            || msg[..32] == [0; 32]
            || msg[32..] != [0; 32]
        {
            log("servicemgr: INSTALLTEST refused: signed staged QUERY\r\n");
            return;
        }
        let mut digest = [0u8; 32];
        digest.copy_from_slice(&msg[..32]);
        let mut first_hash = [0u8; 32];
        for attempt in 0..2 {
            msg.fill(0);
            msg[..8].copy_from_slice(b"app.test");
            msg[32..64].copy_from_slice(&digest);
            reply = [0, 0, CAP_NONE];
            let rc = unsafe {
                syscall6(
                    SYS_IPC_CALL,
                    ENDPOINT as u64,
                    PKG_OP_INSTALL,
                    1,
                    LIFECYCLE as u64,
                    reply.as_mut_ptr() as u64,
                    msg.as_mut_ptr() as u64,
                )
            };
            if reply[2] != CAP_NONE {
                let _ = unsafe { syscall1(SYS_CAP_DESTROY, reply[2]) };
                log("servicemgr: INSTALLTEST refused: unexpected Image cap\r\n");
                return;
            }
            if rc != 0
                || reply[0] != PKG_INSTALLED
                || reply[1] != 1
                || msg[..32] != digest
                || msg[32..64] == [0; 32]
            {
                log("servicemgr: INSTALLTEST refused: AINS receipt\r\n");
                return;
            }
            if attempt == 0 {
                first_hash.copy_from_slice(&msg[32..64]);
            } else if msg[32..64] != first_hash {
                log("servicemgr: INSTALLTEST refused: replay changed AINS\r\n");
                return;
            }
        }
        log(
            "servicemgr: Phase 8.5 signed app.test INSTALL committed and exact replay idempotent; no Image cap\r\n",
        );
    }
    /// Test-only live-cutover setup: signed v7 stays running and is owned by
    /// our held Process cap when v8 PREPARE registers the second Image.
    pub fn test_start_old_live(&mut self) {
        let result = (|| -> Result<(), ()> {
            let active = self.test_last_active.ok_or(())?;
            if self.test_old_child.is_some()
                || self.test_old_image.is_none()
                || !self.online
                || alive(self.receiver) != Ok(true)
            {
                return Err(());
            }
            let (old_id, old_slot) = self.test_old_image.ok_or(())?;
            let mut desc = [0; 3];
            if unsafe { syscall2(SYS_CAP_DESCRIBE, old_slot, desc.as_mut_ptr() as u64) } != 0
                || desc != [1, old_id, RIGHTS_READ | RIGHTS_COPY | RIGHTS_DESTROY]
            {
                return Err(());
            }
            let grant = [(ENDPOINT as u64, RIGHTS_WRITE)];
            let pid = unsafe {
                syscall5(
                    SYS_SPAWN,
                    old_slot,
                    grant.as_ptr() as u64,
                    1,
                    PRIVATE as u64,
                    MGR_BADGE_PKG_PROBE_EXIT,
                )
            };
            if pid <= 0 {
                return Err(());
            }
            let child = Child {
                pid: pid as u64,
                slot: inventory::child_handle(pid as u64, &SyscallProbe).map_err(|_| ())?,
            };
            self.test_old_child = Some(child);
            observed_caps("unretired-child");
            let _ = active;
            Ok(())
        })();
        log(if result.is_ok() {
            "servicemgr: old signed v7 dynamic child spawned, Process cap held for live v8 cutover\r\n"
        } else {
            "servicemgr: old signed v7 live child setup REFUSED\r\n"
        });
    }
    /// Real maximal-platter second immutable install: a signed v8 stage is
    /// independently verified by packaged; no Image is minted by INSTALL.
    pub fn test_install_two(&mut self) {
        let result = (|| -> Result<(), ()> {
            if !self.online || alive(self.receiver) != Ok(true) {
                return Err(());
            }
            let mut msg = [0u8; MSG_BYTES];
            msg[..8].copy_from_slice(b"app.test");
            let mut r = [0u64, 0, CAP_NONE];
            if unsafe {
                syscall6(
                    SYS_IPC_CALL,
                    ENDPOINT as u64,
                    PKG_OP_QUERY,
                    0,
                    CAP_NONE,
                    r.as_mut_ptr() as u64,
                    msg.as_mut_ptr() as u64,
                )
            } != 0
                || r[0] != PKG_ELIGIBLE
                || r[1] != 8
                || r[2] != CAP_NONE
                || msg[..32] == [0; 32]
            {
                return Err(());
            }
            let mut req = [0u8; MSG_BYTES];
            req[..8].copy_from_slice(b"app.test");
            req[32..].copy_from_slice(&msg[..32]);
            r = [0, 0, CAP_NONE];
            if unsafe {
                syscall6(
                    SYS_IPC_CALL,
                    ENDPOINT as u64,
                    PKG_OP_INSTALL,
                    2,
                    LIFECYCLE as u64,
                    r.as_mut_ptr() as u64,
                    req.as_mut_ptr() as u64,
                )
            } != 0
                || r != [PKG_INSTALLED, 2, CAP_NONE]
                || req[..32] != msg[..32]
                || req[32..] == [0; 32]
            {
                return Err(());
            }
            Ok(())
        })();
        log(if result.is_ok() {
            "servicemgr: maximal signed AINS2 committed without Image cap\r\n"
        } else {
            "servicemgr: maximal historical platter second AINS refused\r\n"
        });
    }
    /// At the approved 19+8+2+3=32 object bound, fourth AACT is a
    /// pre-CREATE NO_SPACE. After refusal the prior signed v7 remains
    /// launchable through its still-valid active decision and held ID.
    pub fn test_fourth(&mut self) {
        let result = (|| -> Result<(), ()> {
            let active = self.test_last_active.ok_or(())?;
            if !self.online || alive(self.receiver) != Ok(true) {
                return Err(());
            }
            let mut msg = [0u8; MSG_BYTES];
            msg[..8].copy_from_slice(b"app.test");
            msg[32..].copy_from_slice(&active);
            let mut r = [0u64, 0, CAP_NONE];
            if unsafe {
                syscall6(
                    SYS_IPC_CALL,
                    ENDPOINT as u64,
                    PKG_OP_DEACTIVATE,
                    4,
                    LIFECYCLE as u64,
                    r.as_mut_ptr() as u64,
                    msg.as_mut_ptr() as u64,
                )
            } != 0
                || r != [PKG_NO_SPACE, 0, CAP_NONE]
                || msg != [0; MSG_BYTES]
            {
                return Err(());
            }
            log("servicemgr: maximal AACT4 typed NO_SPACE before CREATE\r\n");
            msg.fill(0);
            msg[..8].copy_from_slice(b"app.test");
            msg[32..].copy_from_slice(&active);
            r = [0, 0, CAP_NONE];
            if unsafe {
                syscall6(
                    SYS_IPC_CALL,
                    ENDPOINT as u64,
                    PKG_OP_LAUNCH,
                    3,
                    LIFECYCLE as u64,
                    r.as_mut_ptr() as u64,
                    msg.as_mut_ptr() as u64,
                )
            } != 0
                || r[0] != PKG_LAUNCH_READY
                || r[1] & 255 != 3
                || r[2] == CAP_NONE
                || msg[32..] != active
            {
                return Err(());
            }
            let mut desc = [0; 3];
            if unsafe { syscall2(SYS_CAP_DESCRIBE, r[2], desc.as_mut_ptr() as u64) } != 0
                || desc != [1, r[1] >> 8, RIGHTS_READ | RIGHTS_COPY | RIGHTS_DESTROY]
            {
                return Err(());
            }
            let grants = [(ENDPOINT as u64, RIGHTS_WRITE)];
            let pid = unsafe {
                syscall5(
                    SYS_SPAWN,
                    r[2],
                    grants.as_ptr() as u64,
                    1,
                    PRIVATE as u64,
                    MGR_BADGE_PKG_PROBE_EXIT,
                )
            };
            if pid <= 0 {
                return Err(());
            }
            let child = Child {
                pid: pid as u64,
                slot: inventory::child_handle(pid as u64, &SyscallProbe).map_err(|_| ())?,
            };
            let timer = unsafe {
                syscall3(
                    SYS_TIMER_ARM,
                    PRIVATE as u64,
                    MGR_BADGE_PKG_PROBE_DEADLINE,
                    15_000_000,
                )
            };
            if timer < 0 {
                return Err(());
            }
            let mut bits = 0u64;
            while bits & (MGR_BADGE_PKG_PROBE_DEADLINE | MGR_BADGE_PKG_PROBE_EXIT) == 0 {
                let n = unsafe { syscall1(SYS_WAIT, PRIVATE as u64) };
                if n < 0 {
                    return Err(());
                }
                bits |= n as u64;
            }
            let _ = unsafe { syscall1(SYS_TIMER_CANCEL, timer as u64) };
            if bits != MGR_BADGE_PKG_PROBE_DEADLINE
                || alive(child) != Ok(true)
                || finish(child).is_err()
                || unsafe { syscall6(SYS_IMAGE_REVOKE, REGISTRAR as u64, desc[1], 0, 0, 0, 0) } != 0
                || unsafe { syscall1(SYS_CAP_DESTROY, r[2]) } != 0
            {
                return Err(());
            }
            Ok(())
        })();
        log(if result.is_ok() {
            "servicemgr: maximal platter fourth refusal preserved usable signed v7 LAUNCH\r\n"
        } else {
            "servicemgr: maximal platter fourth refusal or prior LAUNCH FAILED\r\n"
        });
    }
    /// Second independently signed ELF, version 8, in the same namespace.
    /// The prior version-7 child and ID were retired before reboot; this
    /// proves durable AINS/AACT linkage and new execution, NOT live overlap.
    pub fn test_upgrade_fixture(&mut self) {
        if !self.online || alive(self.receiver) != Ok(true) {
            log("servicemgr: UPGRADETEST refused: receiver unavailable\r\n");
            return;
        }
        log(if self.upgrade_fixture_inner().is_ok() {
            "servicemgr: Phase 8.5 second distinct signed ELF version 8 installed, selected and ran in ring 3; old version retired before reboot\r\n"
        } else {
            "servicemgr: UPGRADETEST refused: signed upgrade receipt or child invariant\r\n"
        });
    }
    fn upgrade_fixture_inner(&mut self) -> Result<(), ()> {
        fn call(op: u64, arg: u64, hash: &[u8; 32]) -> Result<([u64; 3], [u8; 64]), ()> {
            let mut msg = [0u8; MSG_BYTES];
            msg[..8].copy_from_slice(b"app.test");
            msg[32..].copy_from_slice(hash);
            let mut reply = [0u64, 0, CAP_NONE];
            let rc = unsafe {
                syscall6(
                    SYS_IPC_CALL,
                    ENDPOINT as u64,
                    op,
                    arg,
                    if op == PKG_OP_QUERY {
                        CAP_NONE
                    } else {
                        LIFECYCLE as u64
                    },
                    reply.as_mut_ptr() as u64,
                    msg.as_mut_ptr() as u64,
                )
            };
            if rc != 0 {
                if reply[2] != CAP_NONE {
                    let _ = unsafe { syscall1(SYS_CAP_DESTROY, reply[2]) };
                }
                return Err(());
            }
            Ok((reply, msg))
        }
        log("servicemgr: upgrade stage begin\r\n");
        fn expected_no_cap(r: &[u64; 3]) -> Result<(), ()> {
            if r[2] == CAP_NONE {
                Ok(())
            } else {
                let _ = unsafe { syscall1(SYS_CAP_DESTROY, r[2]) };
                Err(())
            }
        }
        // Same-boot cutover keeps v7 registered while signed v8 enters the
        // second registry slot. A rebooted upgrade has no volatile hint.
        let old = if let Some(existing) = self.test_old_image {
            Some(existing)
        } else if let Some(active) = self.test_last_active {
            let (r, msg) = call(PKG_OP_LAUNCH, 3, &active)?;
            if r[2] == CAP_NONE
                || r[0] != PKG_LAUNCH_READY
                || r[1] & 255 != 3
                || msg[32..] != active
            {
                if r[2] != CAP_NONE {
                    let _ = unsafe { syscall1(SYS_CAP_DESTROY, r[2]) };
                }
                return Err(());
            }
            let mut desc = [0; 3];
            if unsafe { syscall2(SYS_CAP_DESCRIBE, r[2], desc.as_mut_ptr() as u64) } != 0
                || desc != [1, r[1] >> 8, RIGHTS_READ | RIGHTS_COPY | RIGHTS_DESTROY]
            {
                let _ = unsafe { syscall1(SYS_CAP_DESTROY, r[2]) };
                return Err(());
            }
            Some((desc[1], r[2]))
        } else {
            None
        };
        log("servicemgr: upgrade stage old Image resolved\r\n");
        let (r, msg) = call(PKG_OP_QUERY, 0, &[0; 32]).map_err(|_| {
            log("servicemgr: upgrade stage QUERY IPC failed\r\n");
        })?;
        expected_no_cap(&r)?;
        if r[0] != PKG_ELIGIBLE || r[1] != 8 || msg[..32] == [0; 32] {
            return Err(());
        }
        let mut digest = [0; 32];
        digest.copy_from_slice(&msg[..32]);
        log("servicemgr: upgrade stage QUERY verified\r\n");
        let (r, msg) = call(PKG_OP_INSTALL, 2, &digest).map_err(|_| {
            log("servicemgr: upgrade stage INSTALL IPC failed\r\n");
        })?;
        expected_no_cap(&r)?;
        if r[0] != PKG_INSTALLED || r[1] != 2 || msg[..32] != digest || msg[32..] == [0; 32] {
            return Err(());
        }
        let mut installed = [0; 32];
        installed.copy_from_slice(&msg[32..]);
        log("servicemgr: upgrade stage INSTALL verified\r\n");
        // On this boot v7 already owns one signed Image ID. Independently
        // register the same verified selection repeatedly until the bounded
        // 16-entry native Image registry is full; a seventeenth request must
        // refuse before minting any ID or cap. Retire every temporary ID
        // before PREPARE admits signed v8 beside the still-live old ID.
        if let Some((first_id, _)) = old {
            let active = self.test_last_active.ok_or(())?;
            let (second, reply) = call(PKG_OP_LAUNCH, 3, &active)?;
            if second[0] != PKG_LAUNCH_READY
                || second[2] == CAP_NONE
                || second[2] >= CAP_SLOTS as u64
                || second[1] & 255 != 3
                || reply[32..] != active
            {
                return Err(());
            }
            let mut desc = [0; 3];
            if unsafe { syscall2(SYS_CAP_DESCRIBE, second[2], desc.as_mut_ptr() as u64) } != 0
                || desc
                    != [
                        1,
                        second[1] >> 8,
                        RIGHTS_READ | RIGHTS_COPY | RIGHTS_DESTROY,
                    ]
                || desc[1] == first_id
            {
                return Err(());
            }
            log("servicemgr: upgrade stage second signed Image ready\r\n");
            observed_caps("two-live-images");
            let second_image = ImageCap {
                id: desc[1],
                slot: second[2] as u8,
            };
            let table_refs = core::ptr::addr_of_mut!(IMAGE_TABLE_REFS).cast::<Option<ImageCap>>();
            for index in 0..MAX_DYNAMIC_IMAGES {
                unsafe { core::ptr::write_volatile(table_refs.add(index), None) };
            }
            let mut issued_ids = [0u64; MAX_DYNAMIC_IMAGES];
            issued_ids[0] = first_id;
            issued_ids[1] = second_image.id;
            let mut fill_failed = false;
            for image_index in 2..MAX_DYNAMIC_IMAGES {
                let result = call(PKG_OP_LAUNCH, 3, &active);
                let (image_reply, image_msg) = match result {
                    Ok(value) => value,
                    Err(()) => {
                        fill_failed = true;
                        break;
                    }
                };
                if image_reply[2] == CAP_NONE || image_reply[2] >= CAP_SLOTS as u64 {
                    if image_reply[2] != CAP_NONE {
                        discard_returned_image_cap(image_reply[2]);
                    }
                    fill_failed = true;
                    break;
                }
                let mut image_desc = [0u64; 3];
                let described = unsafe {
                    syscall2(
                        SYS_CAP_DESCRIBE,
                        image_reply[2],
                        image_desc.as_mut_ptr() as u64,
                    )
                } == 0;
                let unique = !issued_ids[..image_index].contains(&image_desc[1]);
                if described && image_desc[0] == 1 && image_desc[1] >= 27 && !unique {
                    let _ = unsafe { syscall1(SYS_CAP_DESTROY, image_reply[2]) };
                    fill_failed = true;
                    break;
                }
                let good = described
                    && image_desc[0] == 1
                    && image_desc[1] >= 27
                    && image_desc[2] == RIGHTS_READ | RIGHTS_COPY | RIGHTS_DESTROY
                    && image_reply[0] == PKG_LAUNCH_READY
                    && image_reply[1] >> 8 == image_desc[1]
                    && image_reply[1] & 255 == 3
                    && image_msg[32..] == active
                    && unique;
                if described && image_desc[0] == 1 && image_desc[1] >= 27 {
                    issued_ids[image_index] = image_desc[1];
                    unsafe {
                        core::ptr::write_volatile(
                            table_refs.add(image_index - 2),
                            Some(ImageCap {
                                id: image_desc[1],
                                slot: image_reply[2] as u8,
                            }),
                        )
                    };
                } else {
                    discard_returned_image_cap(image_reply[2]);
                    fill_failed = true;
                    break;
                }
                if !good {
                    fill_failed = true;
                    break;
                }
            }
            if fill_failed {
                let _ = cleanup_launched_image_refs(second_image);
                log("servicemgr: signed Image table fill failed before its bound\r\n");
                return Err(());
            }

            let caps_before = core::ptr::addr_of_mut!(IMAGE_CAPS_BEFORE).cast::<u64>();
            if cap_snapshot(caps_before).is_err() {
                let _ = cleanup_launched_image_refs(second_image);
                return Err(());
            }
            let process_count_before =
                match process_list(core::ptr::addr_of_mut!(PROCESS_ROWS_BEFORE) as u64) {
                    Ok(count) => count,
                    Err(()) => {
                        let _ = cleanup_launched_image_refs(second_image);
                        return Err(());
                    }
                };
            let (busy, empty) = match call(PKG_OP_LAUNCH, 3, &active) {
                Ok(reply) => reply,
                Err(()) => {
                    let _ = cleanup_launched_image_refs(second_image);
                    return Err(());
                }
            };
            let busy_exact = busy == [PKG_BUSY, 0, CAP_NONE] && empty == [0; 64];
            if busy[2] != CAP_NONE {
                discard_returned_image_cap(busy[2]);
            }
            let caps_after = core::ptr::addr_of_mut!(IMAGE_CAPS_AFTER).cast::<u64>();
            let cap_unchanged = cap_snapshot(caps_after).is_ok() && cap_snapshots_equal();
            let process_count_after =
                match process_list(core::ptr::addr_of_mut!(PROCESS_ROWS_AFTER) as u64) {
                    Ok(count) => count,
                    Err(()) => {
                        let _ = cleanup_launched_image_refs(second_image);
                        return Err(());
                    }
                };
            let process_unchanged = process_count_before == process_count_after
                && unsafe {
                    core::slice::from_raw_parts(
                        core::ptr::addr_of!(PROCESS_ROWS_BEFORE).cast::<u64>(),
                        process_count_before * 2,
                    ) == core::slice::from_raw_parts(
                        core::ptr::addr_of!(PROCESS_ROWS_AFTER).cast::<u64>(),
                        process_count_after * 2,
                    )
                };
            if !busy_exact || !cap_unchanged || !process_unchanged {
                let _ = cleanup_launched_image_refs(second_image);
                log("servicemgr: 17th signed Image launch mutated authority or was not BUSY\r\n");
                return Err(());
            }
            observed_caps("full-image-table");
            if !cleanup_image_table_refs() {
                let _ = retire_image_cap(second_image);
                return Err(());
            }

            if unsafe { syscall3(SYS_CAP_COPY, second[2], 31, RIGHTS_READ | RIGHTS_DESTROY) } != 0 {
                return Err(());
            }
            if unsafe {
                syscall6(
                    SYS_IMAGE_REVOKE,
                    REGISTRAR as u64,
                    second_image.id,
                    0,
                    0,
                    0,
                    0,
                )
            } != 0
            {
                let _ = unsafe { syscall1(SYS_CAP_DESTROY, second[2] as u64) };
                let _ = unsafe { syscall1(SYS_CAP_DESTROY, 31) };
                return Err(());
            }
            if unsafe { syscall1(SYS_CAP_DESTROY, second[2] as u64) } != 0 {
                let _ = unsafe { syscall1(SYS_CAP_DESTROY, 31) };
                return Err(());
            }
            let mut stale = [u64::MAX; 3];
            if unsafe { syscall2(SYS_CAP_DESCRIBE, 31, stale.as_mut_ptr() as u64) } != -2
                || stale != [u64::MAX; 3]
                || unsafe { syscall5(SYS_SPAWN, 31, 0, 0, CAP_NONE, 0) } != -2
                || unsafe { syscall1(SYS_CAP_DESTROY, 31) } != 0
            {
                return Err(());
            }
            log(
                "servicemgr: 16 signed Image IDs fill the verified registry; seventeenth is mutation-free BUSY, exact revoke frees slots, stale copied bearer refused PASS\r\n",
            );
        }
        let (r, msg) = call(PKG_OP_SELECT_PREPARE, 2, &installed)?;
        log("servicemgr: upgrade stage PREPARE returned\r\n");
        expected_no_cap(&r)?;
        if r[0] != PKG_PREPARED
            || r[1] >> 8 == 0
            || r[1] & 255 != 4
            || msg[..32] != digest
            || msg[32..] != installed
        {
            return Err(());
        }
        let token = r[1];
        if let Some((old_id, old_slot)) = old {
            // At PREPARE two signed IDs must coexist. A third registration
            // refuses before allocation; then retire any running old child
            // before revoking its ID and committing the new selection.
            if let Some(child) = self.test_old_child {
                let grants = [(ENDPOINT as u64, RIGHTS_WRITE)];
                if alive(child) != Ok(true) {
                    log("servicemgr: live-upgrade failure: old Process not live\r\n");
                    return Err(());
                }
                if capacity_refuses(old_slot, 1).is_err() {
                    log("servicemgr: live-upgrade failure: capacity proof\r\n");
                    return Err(());
                }
                if finish(child).is_err() {
                    log("servicemgr: live-upgrade failure: old Process FINISH\r\n");
                    return Err(());
                }
                self.test_old_child = None;
                observed_caps("after-finish");
                log(
                    "servicemgr: genuinely LIVE v7 child stopped and reaped by held Process cap before v8 COMMIT\r\n",
                );
                // Test-only bounded repeated lifecycle on the unchanged
                // signed old Image. Each STOP+FINISH must release the single
                // dynamic-child bound without retaining a Process bearer.
                observed_caps("cycle-pre");
                for _ in 0..4 {
                    let pid = unsafe {
                        syscall5(
                            SYS_SPAWN,
                            old_slot,
                            grants.as_ptr() as u64,
                            1,
                            PRIVATE as u64,
                            MGR_BADGE_PKG_PROBE_EXIT,
                        )
                    };
                    if pid <= 0 {
                        return Err(());
                    }
                    let next = Child {
                        pid: pid as u64,
                        slot: inventory::child_handle(pid as u64, &SyscallProbe).map_err(|_| ())?,
                    };
                    if capacity_refuses(old_slot, 1).is_err() || finish(next).is_err() {
                        return Err(());
                    }
                }
                observed_caps("cycle-post");
                log(
                    "servicemgr: four repeated signed-child STOP/FINISH cycles and 24-child capacity refusals PASS\r\n",
                );
            }
            let mut desc = [0; 3];
            if unsafe { syscall2(SYS_CAP_DESCRIBE, old_slot, desc.as_mut_ptr() as u64) } != 0
                || desc != [1, old_id, RIGHTS_READ | RIGHTS_COPY | RIGHTS_DESTROY]
                || unsafe { syscall3(SYS_CAP_COPY, old_slot, 30, RIGHTS_READ | RIGHTS_DESTROY) }
                    != 0
                || unsafe { syscall6(SYS_IMAGE_REVOKE, REGISTRAR as u64, old_id, 0, 0, 0, 0) } != 0
                || unsafe { syscall1(SYS_CAP_DESTROY, old_slot) } != 0
            {
                return Err(());
            }
            let mut stale = [u64::MAX; 3];
            if unsafe { syscall2(SYS_CAP_DESCRIBE, 30, stale.as_mut_ptr() as u64) } != -2
                || stale != [u64::MAX; 3]
                || unsafe { syscall5(SYS_SPAWN, 30, 0, 0, CAP_NONE, 0) } != -2
                || unsafe { syscall1(SYS_CAP_DESTROY, 30) } != 0
            {
                return Err(());
            }
            self.test_last_active = None;
            self.test_old_image = None;
            log(
                "servicemgr: distinct signed v7/v8 registry slots live together at PREPARE; old copied ID revoked before COMMIT\r\n",
            );
        }
        let (r, msg) = call(PKG_OP_SELECT_COMMIT, token, &installed)?;
        if r[2] == CAP_NONE {
            return Err(());
        }
        let mut desc = [0; 3];
        let good = unsafe { syscall2(SYS_CAP_DESCRIBE, r[2], desc.as_mut_ptr() as u64) } == 0
            && r[0] == PKG_ACTIVE
            && r[1] & 255 == 4
            && r[1] >> 8 == desc[1]
            && desc == [1, desc[1], RIGHTS_READ | RIGHTS_COPY | RIGHTS_DESTROY]
            && desc[1] >= 27
            && msg[..32] == digest
            && msg[32..] != [0; 32];
        if !good {
            if desc[0] == 1 && desc[1] >= 27 {
                let _ =
                    unsafe { syscall6(SYS_IMAGE_REVOKE, REGISTRAR as u64, desc[1], 0, 0, 0, 0) };
            }
            let _ = unsafe { syscall1(SYS_CAP_DESTROY, r[2]) };
            return Err(());
        }
        let old_id = desc[1];
        let mut act = [0; 32];
        act.copy_from_slice(&msg[32..]);
        if unsafe { syscall6(SYS_IMAGE_REVOKE, REGISTRAR as u64, old_id, 0, 0, 0, 0) } != 0
            || unsafe { syscall1(SYS_CAP_DESTROY, r[2]) } != 0
        {
            return Err(());
        }
        let (r, msg) = call(PKG_OP_LAUNCH, 4, &act)?;
        if r[2] == CAP_NONE {
            return Err(());
        }
        let mut desc = [0; 3];
        let good = unsafe { syscall2(SYS_CAP_DESCRIBE, r[2], desc.as_mut_ptr() as u64) } == 0
            && r[0] == PKG_LAUNCH_READY
            && r[1] & 255 == 4
            && r[1] >> 8 == desc[1]
            && desc == [1, desc[1], RIGHTS_READ | RIGHTS_COPY | RIGHTS_DESTROY]
            && desc[1] != old_id
            && desc[1] >= 27
            && msg[..32] == digest
            && msg[32..] == act;
        if !good {
            if desc[0] == 1 && desc[1] >= 27 {
                let _ =
                    unsafe { syscall6(SYS_IMAGE_REVOKE, REGISTRAR as u64, desc[1], 0, 0, 0, 0) };
            }
            let _ = unsafe { syscall1(SYS_CAP_DESTROY, r[2]) };
            return Err(());
        }
        let new_id = desc[1];
        let mut steady = None;
        for slot in 22..32u64 {
            let mut unused = [0; 3];
            if slot != r[2]
                && unsafe { syscall2(SYS_CAP_DESCRIBE, slot, unused.as_mut_ptr() as u64) } != 0
                && unsafe { syscall3(SYS_CAP_COPY, r[2], slot, RIGHTS_READ | RIGHTS_DESTROY) } == 0
            {
                steady = Some(slot);
                break;
            }
        }
        let Some(steady) = steady else {
            let _ = unsafe { syscall6(SYS_IMAGE_REVOKE, REGISTRAR as u64, new_id, 0, 0, 0, 0) };
            let _ = unsafe { syscall1(SYS_CAP_DESTROY, r[2]) };
            return Err(());
        };
        if unsafe { syscall1(SYS_CAP_DESTROY, r[2]) } != 0 {
            return Err(());
        }
        let grants = [(ENDPOINT as u64, RIGHTS_WRITE)];
        let pid = unsafe {
            syscall5(
                SYS_SPAWN,
                steady,
                grants.as_ptr() as u64,
                1,
                PRIVATE as u64,
                MGR_BADGE_PKG_PROBE_EXIT,
            )
        };
        if pid <= 0 {
            return Err(());
        }
        let child = Child {
            pid: pid as u64,
            slot: inventory::child_handle(pid as u64, &SyscallProbe).map_err(|_| ())?,
        };
        let timer = unsafe {
            syscall3(
                SYS_TIMER_ARM,
                PRIVATE as u64,
                MGR_BADGE_PKG_PROBE_DEADLINE,
                TIMEOUT_US,
            )
        };
        let mut seen = 0u64;
        if timer >= 0 {
            while seen & (MGR_BADGE_PKG_PROBE_EXIT | MGR_BADGE_PKG_PROBE_DEADLINE) == 0 {
                let bits = unsafe { syscall1(SYS_WAIT, PRIVATE as u64) };
                if bits < 0 {
                    break;
                }
                seen |= bits as u64;
            }
            let _ = unsafe { syscall1(SYS_TIMER_CANCEL, timer as u64) };
        }
        let dead = alive(child) == Ok(false);
        let reaped = finish(child).is_ok();
        let revoked =
            unsafe { syscall6(SYS_IMAGE_REVOKE, REGISTRAR as u64, new_id, 0, 0, 0, 0) } == 0;
        let dropped = unsafe { syscall1(SYS_CAP_DESTROY, steady) } == 0;
        if timer < 0 || seen != MGR_BADGE_PKG_PROBE_EXIT || !dead || !reaped || !revoked || !dropped
        {
            return Err(());
        }
        Ok(())
    }

    /// Fatal guest negative control. The manager is *not* recoverable by
    /// merely restarting a Process when it dies owning a LIVE dynamic ID.
    /// Only the Power shell's private manager request can trigger this.
    pub fn test_die_with_provisional(&mut self, fault: bool) {
        if !self.online || alive(self.receiver) != Ok(true) {
            log("servicemgr: death fixture OFFLINE\r\n");
            return;
        }
        if let Some(child) = self.test_old_child {
            let Some((id, slot)) = self.test_old_image else {
                log("servicemgr: death fixture missing held Image\r\n");
                return;
            };
            let mut desc = [0; 3];
            if alive(child) != Ok(true)
                || unsafe { syscall2(SYS_CAP_DESCRIBE, slot, desc.as_mut_ptr() as u64) } != 0
                || desc != [1, id, RIGHTS_READ | RIGHTS_COPY | RIGHTS_DESTROY]
            {
                log("servicemgr: death fixture live child/ID refused\r\n");
                return;
            }
            if fault {
                log(
                    "servicemgr: deliberate manager ring-3 #UD with genuinely LIVE signed v7 child/ID (expect fatal halt)\r\n",
                );
                unsafe { core::arch::asm!("ud2", options(noreturn)) };
            }
            log(
                "servicemgr: deliberate manager last-thread exit with genuinely LIVE signed v7 child/ID (expect fatal halt)\r\n",
            );
            unsafe { syscall1(SYS_THREAD_EXIT, 85) };
            log("servicemgr: FATAL live-child death fixture returned\r\n");
            return;
        }
        let mut msg = [0u8; MSG_BYTES];
        msg[..8].copy_from_slice(b"app.test");
        let mut reply = [0u64, 0, CAP_NONE];
        let rc = unsafe {
            syscall6(
                SYS_IPC_CALL,
                ENDPOINT as u64,
                PKG_OP_QUERY,
                0,
                CAP_NONE,
                reply.as_mut_ptr() as u64,
                msg.as_mut_ptr() as u64,
            )
        };
        if reply[2] != CAP_NONE {
            let _ = unsafe { syscall1(SYS_CAP_DESTROY, reply[2]) };
        }
        if rc != 0 || reply[0] != PKG_ELIGIBLE || reply[1] != 7 || reply[2] != CAP_NONE {
            log("servicemgr: death fixture QUERY refused\r\n");
            return;
        }
        let mut digest = [0; 32];
        digest.copy_from_slice(&msg[..32]);
        msg.fill(0);
        msg[..8].copy_from_slice(b"app.test");
        msg[32..].copy_from_slice(&digest);
        reply = [0, 0, CAP_NONE];
        let rc = unsafe {
            syscall6(
                SYS_IPC_CALL,
                ENDPOINT as u64,
                PKG_OP_INSTALL,
                1,
                LIFECYCLE as u64,
                reply.as_mut_ptr() as u64,
                msg.as_mut_ptr() as u64,
            )
        };
        if reply[2] != CAP_NONE {
            let _ = unsafe { syscall1(SYS_CAP_DESTROY, reply[2]) };
        }
        if rc != 0
            || reply[0] != PKG_INSTALLED
            || reply[1] != 1
            || reply[2] != CAP_NONE
            || msg[..32] != digest
            || msg[32..] == [0; 32]
        {
            log("servicemgr: death fixture INSTALL refused\r\n");
            return;
        }
        let mut hash = [0; 32];
        hash.copy_from_slice(&msg[32..]);
        msg.fill(0);
        msg[..8].copy_from_slice(b"app.test");
        msg[32..].copy_from_slice(&hash);
        reply = [0, 0, CAP_NONE];
        let rc = unsafe {
            syscall6(
                SYS_IPC_CALL,
                ENDPOINT as u64,
                PKG_OP_SELECT_PREPARE,
                1,
                LIFECYCLE as u64,
                reply.as_mut_ptr() as u64,
                msg.as_mut_ptr() as u64,
            )
        };
        if reply[2] != CAP_NONE {
            let _ = unsafe { syscall1(SYS_CAP_DESTROY, reply[2]) };
        }
        if rc != 0
            || reply[0] != PKG_PREPARED
            || reply[1] >> 8 == 0
            || reply[2] != CAP_NONE
            || msg[..32] != digest
            || msg[32..] != hash
        {
            log("servicemgr: death fixture PREPARE refused\r\n");
            return;
        }
        if fault {
            log(
                "servicemgr: deliberate manager ring-3 #UD fault with PREPARED LIVE ID (expect fatal halt)\r\n",
            );
            unsafe { core::arch::asm!("ud2", options(noreturn)) };
        }
        log(
            "servicemgr: deliberate manager last-thread exit with PREPARED LIVE ID (expect fatal halt)\r\n",
        );
        unsafe { syscall1(SYS_THREAD_EXIT, 85) };
        // The kernel must fail-stop before recording the exit or unblocking
        // any caller. Returning would be a serious architecture violation.
        log("servicemgr: FATAL death fixture exit returned\r\n");
    }

    /// Fixed test-only signed ELF fixture; all requests use the receiver's
    /// distinct lifecycle marker, never shell possession of an Image cap.
    pub fn test_select_fixture(&mut self) {
        self.run_select_fixture(true);
    }
    pub fn test_select_lite(&mut self) {
        self.run_select_fixture(false);
    }
    pub fn test_maximal_select(&mut self) {
        self.test_maximal = true;
        self.run_select_fixture(false);
        self.test_maximal = false;
    }
    fn run_select_fixture(&mut self, full_platter: bool) {
        if !self.online || alive(self.receiver) != Ok(true) {
            log("servicemgr: SELECTTEST refused: receiver unavailable\r\n");
            return;
        }
        observed_caps("baseline");
        let result = match self.select_fixture_inner(full_platter) {
            Ok(cap) => {
                let bounded = capacity_refuses(cap.slot as u64, 0).is_ok();
                let revoked =
                    unsafe { syscall6(SYS_IMAGE_REVOKE, REGISTRAR as u64, cap.id, 0, 0, 0, 0) }
                        == 0;
                let dropped = unsafe { syscall1(SYS_CAP_DESTROY, cap.slot as u64) } == 0;
                if bounded && revoked && dropped {
                    log(
                        "servicemgr: 24 dynamic children bounded; exact FINISH permits next spawn PASS\r\n",
                    );
                    Ok(())
                } else {
                    Err(())
                }
            }
            Err(()) => Err(()),
        };
        log(if result.is_ok() {
            if full_platter {
                "servicemgr: Phase 8.5 signed SELECT/LAUNCH/DEACTIVATE/reselect; ring-3 child reaped and Image IDs revoked\r\n"
            } else {
                "servicemgr: Phase 8.5 first signed ELF ran, v8-03 active on small platter\r\n"
            }
        } else {
            "servicemgr: SELECTTEST refused: lifecycle receipt or Image invariant\r\n"
        });
    }
    #[inline(never)]
    fn select_fixture_inner(&mut self, full_platter: bool) -> Result<CapacityImage, ()> {
        fn call(op: u64, arg: u64, hash: &[u8; 32]) -> Result<([u64; 3], [u8; 64]), ()> {
            let mut msg = [0u8; MSG_BYTES];
            msg[..8].copy_from_slice(b"app.test");
            msg[32..].copy_from_slice(hash);
            let mut reply = [0u64, 0, CAP_NONE];
            let rc = unsafe {
                syscall6(
                    SYS_IPC_CALL,
                    ENDPOINT as u64,
                    op,
                    arg,
                    if op == PKG_OP_QUERY {
                        CAP_NONE
                    } else {
                        LIFECYCLE as u64
                    },
                    reply.as_mut_ptr() as u64,
                    msg.as_mut_ptr() as u64,
                )
            };
            if rc != 0 {
                if reply[2] != CAP_NONE {
                    let _ = unsafe { syscall1(SYS_CAP_DESTROY, reply[2]) };
                }
                return Err(());
            }
            Ok((reply, msg))
        }
        fn no_cap(reply: &[u64; 3]) -> Result<(), ()> {
            if reply[2] == CAP_NONE {
                Ok(())
            } else {
                let _ = unsafe { syscall1(SYS_CAP_DESTROY, reply[2]) };
                Err(())
            }
        }
        fn image(
            reply: [u64; 3],
            status: u64,
            generation: u64,
            digest: &[u8; 32],
            msg: &[u8; 64],
        ) -> Result<(u64, u64), ()> {
            if reply[2] == CAP_NONE {
                return Err(());
            }
            let mut found = [0u64; 3];
            let checked =
                unsafe { syscall2(SYS_CAP_DESCRIBE, reply[2], found.as_mut_ptr() as u64) } == 0
                    && reply[0] == status
                    && reply[1] & 255 == generation
                    && reply[1] >> 8 == found[1]
                    && (27..=u32::MAX as u64).contains(&found[1])
                    && found == [1, found[1], RIGHTS_READ | RIGHTS_COPY | RIGHTS_DESTROY]
                    && msg[..32] == *digest
                    && msg[32..] != [0; 32];
            if checked {
                Ok((found[1], reply[2]))
            } else {
                if found[0] == 1 && found[1] >= 27 {
                    let _ = unsafe {
                        syscall6(SYS_IMAGE_REVOKE, REGISTRAR as u64, found[1], 0, 0, 0, 0)
                    };
                }
                let _ = unsafe { syscall1(SYS_CAP_DESTROY, reply[2]) };
                Err(())
            }
        }
        let (r, msg) = call(PKG_OP_QUERY, 0, &[0; 32])?;
        no_cap(&r)?;
        if r[0] != PKG_ELIGIBLE || r[1] != 7 || msg[..32] == [0; 32] {
            return Err(());
        }
        let mut digest = [0; 32];
        digest.copy_from_slice(&msg[..32]);
        let (r, msg) = call(PKG_OP_INSTALL, 1, &digest)?;
        no_cap(&r)?;
        if r[0] != PKG_INSTALLED || r[1] != 1 || msg[..32] != digest || msg[32..] == [0; 32] {
            return Err(());
        }
        let mut ins = [0; 32];
        ins.copy_from_slice(&msg[32..]);
        let (r, msg) = call(PKG_OP_SELECT_PREPARE, 1, &ins)?;
        no_cap(&r)?;
        if r[0] != PKG_PREPARED
            || r[1] >> 8 == 0
            || r[1] & 255 != 1
            || msg[..32] != digest
            || msg[32..] != ins
        {
            return Err(());
        }
        let first = r[1];
        let (r, msg) = call(PKG_OP_SELECT_PREPARE, 1, &ins)?;
        no_cap(&r)?;
        if r[0] != PKG_PREPARED || r[1] != first || msg[..32] != digest || msg[32..] != ins {
            return Err(());
        }
        let (r, msg) = call(PKG_OP_ABORT, first, &ins)?;
        no_cap(&r)?;
        if r != [PKG_OK, 0, CAP_NONE] || msg != [0; 64] {
            return Err(());
        }
        let (r, msg) = call(PKG_OP_ABORT, first, &ins)?;
        no_cap(&r)?;
        if r != [PKG_OK, 0, CAP_NONE] || msg != [0; 64] {
            return Err(());
        }
        let (r, msg) = call(PKG_OP_SELECT_COMMIT, first, &ins)?;
        no_cap(&r)?;
        if r != [PKG_STALE, 0, CAP_NONE] || msg != [0; 64] {
            return Err(());
        }
        let (r, msg) = call(PKG_OP_SELECT_PREPARE, 1, &ins)?;
        no_cap(&r)?;
        if r[0] != PKG_PREPARED
            || r[1] == first
            || r[1] & 255 != 1
            || msg[..32] != digest
            || msg[32..] != ins
        {
            return Err(());
        }
        let token = r[1];
        let (r, msg) = call(PKG_OP_SELECT_COMMIT, token, &ins)?;
        let (image_id, image_slot) = image(r, PKG_ACTIVE, 1, &digest, &msg)?;
        let mut active = [0; 32];
        active.copy_from_slice(&msg[32..]);
        // A genuine LIVE Image travels through the kernel's send queue to
        // packaged. Its unexpected landed copy must be disposed, while the
        // independent sender source is still live. No new endpoint/grant.
        let mut wrong = [0u8; MSG_BYTES];
        wrong[..8].copy_from_slice(b"app.test");
        let mut answer = [0u64, 0, CAP_NONE];
        if unsafe {
            syscall6(
                SYS_IPC_CALL,
                ENDPOINT as u64,
                PKG_OP_QUERY,
                0,
                image_slot,
                answer.as_mut_ptr() as u64,
                wrong.as_mut_ptr() as u64,
            )
        } != 0
            || answer != [PKG_BAD_FORMAT, 0, CAP_NONE]
            || wrong != [0; MSG_BYTES]
        {
            return Err(());
        }
        let mut retained = [0; 3];
        if unsafe { syscall2(SYS_CAP_DESCRIBE, image_slot, retained.as_mut_ptr() as u64) } != 0
            || retained != [1, image_id, RIGHTS_READ | RIGHTS_COPY | RIGHTS_DESTROY]
        {
            return Err(());
        }
        log(
            "servicemgr: LIVE Image IPC send-queue escrow, receiver unexpected-cap disposal and source retention PASS\r\n",
        );
        // Copy possession survives destruction, but an explicit ID revoke
        // makes the copied bearer stale even after another slot is reused.
        if unsafe { syscall3(SYS_CAP_COPY, image_slot, 30, RIGHTS_READ | RIGHTS_DESTROY) } != 0 {
            return Err(());
        }
        // Before a new explicit LAUNCH, retire *every* copy of the old ID;
        // dropping the returned cap alone cannot invalidate copies.
        if unsafe { syscall6(SYS_IMAGE_REVOKE, REGISTRAR as u64, image_id, 0, 0, 0, 0) } != 0
            || unsafe { syscall1(SYS_CAP_DESTROY, image_slot) } != 0
        {
            return Err(());
        }
        let mut stale = [u64::MAX; 3];
        if unsafe { syscall2(SYS_CAP_DESCRIBE, 30, stale.as_mut_ptr() as u64) } != -2
            || stale != [u64::MAX; 3]
            || unsafe { syscall5(SYS_SPAWN, 30, 0, 0, CAP_NONE, 0) } != -2
            || unsafe { syscall1(SYS_CAP_DESTROY, 30) } != 0
        {
            return Err(());
        }
        let (r, msg) = call(PKG_OP_SELECT_COMMIT, token, &ins)?;
        no_cap(&r)?;
        if r != [PKG_ACTIVE, 1, CAP_NONE] || msg[..32] != digest || msg[32..] != active {
            return Err(());
        }
        let (r, msg) = call(PKG_OP_LAUNCH, 1, &active)?;
        let (next_id, next_slot) = image(r, PKG_LAUNCH_READY, 1, &digest, &msg)?;
        if next_id == image_id || msg[32..] != active {
            return Err(());
        }
        // IPC lands a transferable cap. Attenuate locally, then discard
        // that transitional reference; only manager READ|DESTROY remains.
        let mut steady = None;
        for slot in 22..32u64 {
            let mut found = [0u64; 3];
            if slot != next_slot
                && unsafe { syscall2(SYS_CAP_DESCRIBE, slot, found.as_mut_ptr() as u64) } != 0
                && unsafe { syscall3(SYS_CAP_COPY, next_slot, slot, RIGHTS_READ | RIGHTS_DESTROY) }
                    == 0
            {
                steady = Some(slot);
                break;
            }
        }
        let Some(steady) = steady else {
            let _ = unsafe { syscall6(SYS_IMAGE_REVOKE, REGISTRAR as u64, next_id, 0, 0, 0, 0) };
            let _ = unsafe { syscall1(SYS_CAP_DESTROY, next_slot) };
            return Err(());
        };
        if unsafe { syscall1(SYS_CAP_DESTROY, next_slot) } != 0 {
            return Err(());
        }
        let mut desc = [0u64; 3];
        if unsafe { syscall2(SYS_CAP_DESCRIBE, steady, desc.as_mut_ptr() as u64) } != 0
            || desc != [1, next_id, RIGHTS_READ | RIGHTS_DESTROY]
        {
            return Err(());
        }
        let grants = [(ENDPOINT as u64, RIGHTS_WRITE)];
        let pid = unsafe {
            syscall5(
                SYS_SPAWN,
                steady,
                grants.as_ptr() as u64,
                1,
                PRIVATE as u64,
                MGR_BADGE_PKG_PROBE_EXIT,
            )
        };
        if pid <= 0 {
            return Err(());
        }
        let child = Child {
            pid: pid as u64,
            slot: inventory::child_handle(pid as u64, &SyscallProbe).map_err(|_| ())?,
        };
        let mut bad = false;
        let timer = unsafe {
            syscall3(
                SYS_TIMER_ARM,
                PRIVATE as u64,
                MGR_BADGE_PKG_PROBE_DEADLINE,
                SELECT_CHILD_TIMEOUT_US,
            )
        };
        if timer < 0 {
            log("servicemgr: SELECTTEST child deadline timer unavailable\r\n");
            bad = true;
        }
        let mut seen = 0u64;
        if !bad {
            while seen & (MGR_BADGE_PKG_PROBE_EXIT | MGR_BADGE_PKG_PROBE_DEADLINE) == 0 {
                let bits = unsafe { syscall1(SYS_WAIT, PRIVATE as u64) };
                if bits < 0 {
                    log("servicemgr: SELECTTEST child exit wait refused\r\n");
                    bad = true;
                    break;
                }
                seen |= bits as u64;
            }
        }
        if timer >= 0 {
            let _ = unsafe { syscall1(SYS_TIMER_CANCEL, timer as u64) };
        }
        let child_alive = alive(child);
        if seen != MGR_BADGE_PKG_PROBE_EXIT || child_alive != Ok(false) {
            log_line(|o| {
                o.str("servicemgr: select first receipt bits=");
                o.u64(seen);
                o.str(" dead=");
                o.u64(u64::from(child_alive == Ok(false)));
                o.crlf();
            });
            bad = true;
        }
        if finish(child).is_err() {
            bad = true;
        }
        if !bad {
            let again = unsafe {
                syscall5(
                    SYS_SPAWN,
                    steady,
                    grants.as_ptr() as u64,
                    1,
                    PRIVATE as u64,
                    MGR_BADGE_PKG_PROBE_EXIT,
                )
            };
            if again <= 0 {
                bad = true;
            } else {
                let later = Child {
                    pid: again as u64,
                    slot: inventory::child_handle(again as u64, &SyscallProbe).map_err(|_| ())?,
                };
                let timer = unsafe {
                    syscall3(
                        SYS_TIMER_ARM,
                        PRIVATE as u64,
                        MGR_BADGE_PKG_PROBE_DEADLINE,
                        SELECT_CHILD_TIMEOUT_US,
                    )
                };
                let mut bits = 0u64;
                if timer >= 0 {
                    while bits & (MGR_BADGE_PKG_PROBE_EXIT | MGR_BADGE_PKG_PROBE_DEADLINE) == 0 {
                        let n = unsafe { syscall1(SYS_WAIT, PRIVATE as u64) };
                        if n < 0 {
                            break;
                        }
                        bits |= n as u64;
                    }
                    let _ = unsafe { syscall1(SYS_TIMER_CANCEL, timer as u64) };
                }
                // The original Process witness must be retired even when
                // the diagnostic receipt is bad. Never short-circuit FINISH.
                let later_alive = alive(later);
                let later_finished = finish(later);
                if timer < 0
                    || bits != MGR_BADGE_PKG_PROBE_EXIT
                    || later_alive != Ok(false)
                    || later_finished.is_err()
                {
                    log_line(|o| {
                        o.str("servicemgr: select later receipt bits=");
                        o.u64(bits);
                        o.str(" dead=");
                        o.u64(u64::from(later_alive == Ok(false)));
                        o.str(" finished=");
                        o.u64(u64::from(later_finished.is_ok()));
                        o.crlf();
                    });
                    bad = true;
                }
            }
        }
        if bad {
            let _ = unsafe { syscall6(SYS_IMAGE_REVOKE, REGISTRAR as u64, next_id, 0, 0, 0, 0) };
            let _ = unsafe { syscall1(SYS_CAP_DESTROY, steady) };
            return Err(());
        }
        // The old child and all old Image IDs were retired before disabling
        // durable selection. A duplicate request cannot consume v8-03.
        let (r, msg) = call(PKG_OP_DEACTIVATE, 2, &active)?;
        no_cap(&r)?;
        if r != [PKG_DEACTIVATED, 2, CAP_NONE] || msg[..32] == [0; 32] || msg[32..] != [0; 32] {
            return Err(());
        }
        let mut disabled = [0; 32];
        disabled.copy_from_slice(&msg[..32]);
        let (r, msg) = call(PKG_OP_DEACTIVATE, 2, &active)?;
        no_cap(&r)?;
        if r != [PKG_DEACTIVATED, 2, CAP_NONE] || msg[..32] != disabled || msg[32..] != [0; 32] {
            return Err(());
        }
        let (r, msg) = call(PKG_OP_LAUNCH, 1, &active)?;
        no_cap(&r)?;
        if r != [PKG_STALE, 0, CAP_NONE] || msg != [0; 64] {
            return Err(());
        }
        // Re-selecting version 7 after deactivation does not lower the
        // monotonic selection floor. This consumes the THIRD decision.
        let (r, msg) = call(PKG_OP_SELECT_PREPARE, 1, &ins)?;
        no_cap(&r)?;
        if r[0] != PKG_PREPARED
            || r[1] >> 8 == 0
            || r[1] & 255 != 3
            || msg[..32] != digest
            || msg[32..] != ins
        {
            return Err(());
        }
        let (r, msg) = call(PKG_OP_SELECT_COMMIT, r[1], &ins)?;
        let (third_id, third_slot) = image(r, PKG_ACTIVE, 3, &digest, &msg)?;
        let mut third_hash = [0; 32];
        third_hash.copy_from_slice(&msg[32..]);
        if third_id == next_id {
            return Err(());
        }
        if full_platter {
            if unsafe { syscall6(SYS_IMAGE_REVOKE, REGISTRAR as u64, third_id, 0, 0, 0, 0) } != 0
                || unsafe { syscall1(SYS_CAP_DESTROY, third_slot) } != 0
            {
                return Err(());
            }
        } else {
            self.test_last_active = Some(third_hash);
            self.test_old_image = Some((third_id, third_slot));
            log("servicemgr: selected signed v7 Image held LIVE for same-boot v8 PREPARE\r\n");
        }
        if full_platter {
            // At 32/32 the fourth wire decision fits v1 but AFS1 cannot
            // CREATE it. Refuse before mutation, retaining all predecessors.
            let (r, msg) = call(PKG_OP_DEACTIVATE, 4, &third_hash)?;
            no_cap(&r)?;
            if r != [PKG_NO_SPACE, 0, CAP_NONE] || msg != [0; 64] {
                return Err(());
            }
        }
        Ok(CapacityImage {
            id: next_id,
            slot: steady as u8,
        })
    }
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
            Ok((child, install_handed_off)) => {
                self.receiver = child;
                self.install_handed_off = install_handed_off;
                self.online = true;
                log("servicemgr: packaged replacement ready on original endpoint\r\n");
            }
            Err(()) => log("servicemgr: packaged OFFLINE (replacement refused)\r\n"),
        }
    }
    pub fn event(&mut self, bits: u64) {
        if self.online && !self.install_handed_off {
            match handoff_install_authority(self.receiver) {
                Ok(true) => self.install_handed_off = true,
                Ok(false) => {}
                Err(()) => log(
                    "servicemgr: APB1 install handoff pending; receiver or authority not ready\r\n",
                ),
            }
        }
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
            Ok((child, install_handed_off)) => {
                self.receiver = child;
                self.install_handed_off = install_handed_off;
                self.online = true;
            }
            Err(()) => log("servicemgr: packaged OFFLINE (replacement refused)\r\n"),
        }
    }
}
