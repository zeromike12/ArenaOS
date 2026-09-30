//! Interactive Phase 8.2 mediated durable policy commands (ADR-0048).
//! Kept separate from the historical filesystem and manager proof commands.
use super::*;
#[path = "../../permission.rs"]
mod app_request;
const APP_WAKE: u64 = 1 << 11;
fn call(op: u64, w1: u64, marker: u64, msg: &mut [u8; MSG_BYTES]) -> Result<[u64; 3], i64> {
    let mut out = [0u64; 3];
    let r = unsafe { syscall6(SYS_IPC_CALL, SLOT_MEDIATOR, op, w1, marker,
        out.as_mut_ptr() as u64, msg.as_mut_ptr() as u64) };
    if r != 0 { return Err(r); }
    if out[2] != CAP_NONE { return Err(-1); }
    Ok(out)
}
fn admin(o: &mut Out, op: u64, marker: u64) {
    let mut msg = [0u8; MSG_BYTES];
    match call(op, 0, marker, &mut msg) {
        Ok([PERM_COMMITTED, seq, CAP_NONE]) => {
            o.str("permission: authorized durable decision generation ");
            o.u64(seq); o.str("; grant not issued by approval\r\n");
            if op == PERM_OP_ALLOW {
                // This hint is forgeable on the shared event channel;
                // manager may spawn app, but only receiver can approve.
                let r = unsafe { syscall2(SYS_NOTIFY, SLOT_MGR_WAKE, APP_WAKE) };
                if r != 0 { o.str("permission: app wake refused (durable approval intact)\r\n"); }
            }
        }
        Ok([PERM_DENIED, _, CAP_NONE]) => o.str("permission: admin marker refused by receiver\r\n"),
        Ok([PERM_NO_SPACE, _, CAP_NONE]) => o.str("permission: admin NO_SPACE (no durable acknowledgement)\r\n"),
        Ok([PERM_DEGRADED, _, CAP_NONE]) => o.str("permission: admin DEGRADED (no durable acknowledgement)\r\n"),
        Ok([PERM_CORRUPT, _, CAP_NONE]) => o.str("permission: admin CORRUPT (no durable acknowledgement)\r\n"),
        Ok([PERM_IO, _, CAP_NONE]) => o.str("permission: admin IO (no durable acknowledgement)\r\n"),
        Ok(ans) => { o.str("permission: admin status "); o.u64(ans[0]); o.crlf(); }
        Err(r) => { o.str("permission: transport refused "); o.i64(r); o.crlf(); }
    }
}
fn probe_fixture(o: &mut Out, op: u64, mode: u64) {
    let mut msg = [0u8; MSG_BYTES];
    if !matches!(call(op, 0, SLOT_APPROVAL, &mut msg),
        Ok([PERM_OK, 0, CAP_NONE])) {
        o.str("permission: diagnostic probe arm refused by receiver\r\n");
        return;
    }
    let req = unsafe { syscall2(SYS_NOTIFY, SLOT_MGR_ADMIN, mode) };
    let wake = unsafe { syscall2(SYS_NOTIFY, SLOT_MGR_WAKE, MGR_BADGE_ADMIN_WAKE) };
    if req == 0 && wake == 0 {
        o.str("permission: private probe request sent; manager must prove result + exit + deadline\r\n");
    } else { o.str("permission: private probe request notification refused\r\n"); }
}
pub fn dispatch(o: &mut Out, cmd: &[u8]) {
    if eq(cmd, b"allow") { admin(o, PERM_OP_ALLOW, SLOT_APPROVAL); }
    else if eq(cmd, b"deny") { admin(o, PERM_OP_DENY, SLOT_APPROVAL); }
    else if eq(cmd, b"revoke") { admin(o, PERM_OP_REVOKE, SLOT_APPROVAL); }
    else if eq(cmd, b"allow-noauth") { admin(o, PERM_OP_ALLOW, CAP_NONE); }
    else if eq(cmd, b"allow-wrong") { admin(o, PERM_OP_ALLOW, SLOT_STACK_DIAG); }
    else if eq(cmd, b"allow-attenuated") {
        // Same marker OBJECT with insufficient transfer rights. Receiver
        // requires the exact issued form; a guessed/attenuated surrogate
        // is never an administrative approval.
        if unsafe { syscall3(SYS_CAP_COPY, SLOT_APPROVAL, 27,
            RIGHTS_READ | RIGHTS_COPY) } == 0 {
            admin(o, PERM_OP_ALLOW, 27);
            let _ = unsafe { syscall1(SYS_CAP_DESTROY, 27) };
        } else { o.str("permission: attenuated-marker fixture refused\r\n"); }
    }
    else if eq(cmd, b"show") {
        let mut msg = [0u8; MSG_BYTES];
        match call(PERM_OP_SHOW, 0, CAP_NONE, &mut msg) {
            Ok([PERM_OK, value, CAP_NONE]) if value <= 1 => {
                o.str("permission: request arena.txt READ; durable decision ");
                o.str(if value == 1 { "ALLOW" } else { "DENY" });
                o.str("; issued grant only via ACQUIRE, not request/approval\r\n");
            }
            Ok([PERM_DEGRADED, _, CAP_NONE]) => o.str("permission: policy DEGRADED; no grant\r\n"),
            Ok([PERM_IO, _, CAP_NONE]) => o.str("permission: FS backend OFFLINE; decision unavailable, no grant\r\n"),
            Ok(ans) => { o.str("permission: SHOW status "); o.u64(ans[0]); o.crlf(); }
            Err(r) => { o.str("permission: broker OFFLINE/SHOW transport refused "); o.i64(r); o.crlf(); }
        }
    } else if eq(cmd, b"fs-stop") {
        let mut msg = [0u8; MSG_BYTES];
        let mut words = [0u64; 3];
        let rc = unsafe { syscall6(SYS_IPC_CALL, SLOT_FSD, FS_OP_SHUTDOWN, 0, 19,
            words.as_mut_ptr() as u64, msg.as_mut_ptr() as u64) };
        if rc == 0 && words[0] == FS_OK && words[2] == CAP_NONE {
            o.str("permission: trusted real fsd shutdown answered; backend now absent\r\n");
        } else { o.str("permission: FS shutdown refused by receiver or transport\r\n"); }
    } else if eq(cmd, b"restart") {
        let mut msg = [0u8; MSG_BYTES];
        match call(PERM_OP_TEST_EXIT, 0, SLOT_APPROVAL, &mut msg) {
            Ok([PERM_OK, 0, CAP_NONE]) => o.str("permission: marker-authorized broker restart requested; held endpoint retained\r\n"),
            Ok(ans) => { o.str("permission: restart refused status "); o.u64(ans[0]); o.crlf(); }
            Err(r) => { o.str("permission: restart transport refused "); o.i64(r); o.crlf(); }
        }
    }
    else if eq(cmd, b"probe-bad") {
        probe_fixture(o, PERM_OP_TEST_BAD_PING, MGR_BADGE_ADMIN_PERM_PROBE);
    } else if eq(cmd, b"probe-stall") {
        probe_fixture(o, PERM_OP_TEST_STALL_PING, MGR_BADGE_ADMIN_PERM_PROBE);
    } else if eq(cmd, b"probe-stall-caller-first") {
        probe_fixture(o, PERM_OP_TEST_STALL_PING, MGR_BADGE_ADMIN_PERM_CALLER_FIRST);
    }
    else if eq(cmd, b"request") {
        o.str("permission: REQUEST arena.txt READ (description, not authority)\r\n");
    } else if eq(cmd, b"acquire") {
        let mut msg = [0u8; MSG_BYTES];
        msg[..app_request::REQUEST_BYTES].copy_from_slice(&app_request::READ_REQUEST);
        match call(PERM_OP_ACQUIRE, 0, CAP_NONE, &mut msg) {
            Ok([PERM_OK, 16, CAP_NONE]) => {
                unsafe { core::ptr::copy_nonoverlapping(msg.as_ptr(),
                    core::ptr::addr_of_mut!(PERM_TOKEN).cast::<u8>(), 16) };
                o.str("permission: ACQUIRE received 128-bit bearer (bytes not logged)\r\n");
            }
            Ok([PERM_DENIED, _, CAP_NONE]) => o.str("permission: ACQUIRE denied (no ALLOW)\r\n"),
            Ok([PERM_NO_SPACE, _, CAP_NONE]) => o.str("permission: ACQUIRE NO_SPACE (four live bearers; no eviction)\r\n"),
            Ok([PERM_IO, _, CAP_NONE]) => o.str("permission: ACQUIRE backend IO/offline; no bearer issued\r\n"),
            Ok(ans) => { o.str("permission: ACQUIRE status "); o.u64(ans[0]); o.crlf(); }
            Err(r) => { o.str("permission: transport refused "); o.i64(r); o.crlf(); }
        }
    } else if eq(cmd, b"forge") {
        let mut msg = [0u8; MSG_BYTES];
        msg[..16].fill(0xA7); // intentionally NOT a random service-issued bearer
        match call(PERM_OP_READ, 0, CAP_NONE, &mut msg) {
            Ok([PERM_BAD_TOKEN, 0, CAP_NONE]) => o.str("permission: forged 128-bit bearer refused by receiver\r\n"),
            Ok(ans) => { o.str("permission: FORGERY incorrectly answered "); o.u64(ans[0]); o.crlf(); }
            Err(r) => { o.str("permission: forged bearer transport refused "); o.i64(r); o.crlf(); }
        }
    } else if eq(cmd, b"snapshot") {
        let mut v = [0u64; 3];
        let r = unsafe { syscall2(SYS_RESOURCE_SNAPSHOT, SLOT_POWER, v.as_mut_ptr() as u64) };
        if r == 0 {
            o.str("permission: resource snapshot free/records/processes ");
            o.u64(v[0]); o.str("/"); o.u64(v[1]); o.str("/"); o.u64(v[2]); o.crlf();
        } else { o.str("permission: resource snapshot refused\r\n"); }
    } else if eq(cmd, b"name-only") {
        // Same built-in image and request but ZERO client grants. This
        // image/name is not acquisition authority, even under ALLOW.
        let pid = unsafe { syscall5(SYS_SPAWN, SLOT_PERM_IMAGE, 0, 0, SLOT_NOTIF, SPAWN_BADGE) };
        if pid <= 0 { o.str("permission: name-only spawn refused\r\n"); }
        else {
            let badge = unsafe { syscall1(SYS_WAIT, SLOT_NOTIF) };
            let reaped = unsafe { syscall2(SYS_PROC_FINISH, SLOT_LIFE_CHILD, 0) };
            if badge == SPAWN_BADGE as i64 && reaped == 0 {
                o.str("permission: name-only child reaped without mediator authority\r\n");
            } else { o.str("permission: name-only child exit/reap refused\r\n"); }
        }
    } else if eq(cmd, b"delegate") {
        // Intentional endpoint transfer. The shell does not invoke fsd
        // for this child; app image inherits only mediator WRITE|COPY.
        let grants = [(SLOT_MEDIATOR, RIGHTS_WRITE | RIGHTS_COPY)];
        let pid = unsafe { syscall5(SYS_SPAWN, SLOT_PERM_IMAGE, grants.as_ptr() as u64,
            1, SLOT_NOTIF, SPAWN_BADGE) };
        if pid <= 0 { o.str("permission: delegate spawn refused\r\n"); }
        else {
            let badge = unsafe { syscall1(SYS_WAIT, SLOT_NOTIF) };
            let result = unsafe { syscall2(SYS_PROC_FINISH, SLOT_LIFE_CHILD, 0) };
            if badge == SPAWN_BADGE as i64 && result == 0 {
                o.str("permission: delegated endpoint-only child reaped\r\n");
            } else { o.str("permission: delegate exit/reap refused\r\n"); }
        }
    } else if eq(cmd, b"retain") {
        unsafe { core::ptr::copy_nonoverlapping(core::ptr::addr_of!(PERM_TOKEN).cast::<u8>(),
            core::ptr::addr_of_mut!(PERM_RETAINED).cast::<u8>(), 16) };
        o.str("permission: independent bearer byte copy retained\r\n");
    } else if eq(cmd, b"read") || eq(cmd, b"read-retained") {
        let mut msg = [0u8; MSG_BYTES];
        let token = if eq(cmd, b"read-retained") {
            core::ptr::addr_of!(PERM_RETAINED).cast::<u8>()
        } else { core::ptr::addr_of!(PERM_TOKEN).cast::<u8>() };
        unsafe { core::ptr::copy_nonoverlapping(token, msg.as_mut_ptr(), 16) };
        match call(PERM_OP_READ, 0, CAP_NONE, &mut msg) {
            Ok([PERM_OK, 32, CAP_NONE]) => {
                if (0..32).all(|i| msg[i] == pattern_byte(i)) {
                    o.str("permission: READ verified 32 bytes from arena.txt via mediator\r\n");
                } else { o.str("permission: READ returned wrong bytes\r\n"); }
            }
            Ok([PERM_BAD_TOKEN, _, CAP_NONE]) => o.str("permission: READ refused old/missing bearer\r\n"),
            Ok(ans) => { o.str("permission: READ status "); o.u64(ans[0]); o.crlf(); }
            Err(r) => { o.str("permission: transport refused "); o.i64(r); o.crlf(); }
        }
    } else { o.str("permission: use request|show|allow|deny|revoke|acquire|delegate|name-only|retain|read|read-retained|forge|restart|fs-stop|allow-noauth|allow-wrong\r\n"); }
}
