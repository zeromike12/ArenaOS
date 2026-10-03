#![no_std]
#![no_main]
// Fixed built-in request: arena.txt READ. Image/name alone grants nothing;
// this process inherits only mediator WRITE|COPY, not fsd, rngd or marker.
#[path = "../../abi.rs"]
mod abi;
#[path = "../../permission.rs"]
mod app_request;
use abi::*;
use core::panic::PanicInfo;
const MEDIATOR: u64 = 0;
fn fail(s: &str) -> ! {
    log_line(|o| { o.str("permapp: FAIL "); o.str(s); });
    unsafe { syscall1(SYS_THREAD_EXIT, 1) };
    loop { core::hint::spin_loop() }
}
#[panic_handler]
fn panic(_: &PanicInfo) -> ! { fail("PANIC") }
fn call(op: u64, w1: u64, msg: &mut [u8; MSG_BYTES]) -> [u64; 3] {
    let mut reply = [0u64; 3];
    let r = unsafe { syscall6(SYS_IPC_CALL, MEDIATOR, op, w1, CAP_NONE,
        reply.as_mut_ptr() as u64, msg.as_mut_ptr() as u64) };
    if r < 0 || reply[2] != CAP_NONE { fail("mediator transport"); }
    reply
}
#[unsafe(no_mangle)]
pub extern "C" fn _start() -> ! {
    log_line(|o| o.str("permapp: REQUEST arena.txt READ (not authority)"));
    let mut actual = [0u64; 3];
    let has_endpoint = unsafe { syscall2(SYS_CAP_DESCRIBE, MEDIATOR, actual.as_mut_ptr() as u64) } == 0;
    if !has_endpoint {
        let mut reply = [0u64; 3];
        let mut msg = [0u8; MSG_BYTES];
        msg[..app_request::REQUEST_BYTES].copy_from_slice(&app_request::READ_REQUEST);
        let rc = unsafe { syscall6(SYS_IPC_CALL, MEDIATOR, PERM_OP_ACQUIRE, 0, CAP_NONE,
            reply.as_mut_ptr() as u64, msg.as_mut_ptr() as u64) };
        if rc >= 0 { fail("name-only unexpectedly acquired without endpoint"); }
        log_line(|o| o.str("permapp: name-only request refused without mediator endpoint"));
        unsafe { syscall1(SYS_THREAD_EXIT, 0) };
        loop { core::hint::spin_loop() }
    }
    if actual[0] != 2 || actual[2] != RIGHTS_WRITE | RIGHTS_COPY {
        fail("inherited cap not exact mediator WRITE|COPY");
    }
    for slot in 1..32 {
        if unsafe { syscall2(SYS_CAP_DESCRIBE, slot, actual.as_mut_ptr() as u64) } == 0 {
            fail("unexpected inherited fsd/rngd/approval/other authority");
        }
    }
    log_line(|o| o.str("permapp: audited ONLY mediator WRITE|COPY, 31 other cap slots empty"));
    for op in [PERM_OP_ALLOW, PERM_OP_DENY, PERM_OP_REVOKE] {
        let mut probe = [0u8; MSG_BYTES];
        if call(op, 0, &mut probe)[0] != PERM_DENIED {
            fail("endpoint-only client mutated policy without approval marker");
        }
    }
    log_line(|o| o.str("permapp: endpoint-only ALLOW/DENY/REVOKE refused by receiver"));
    // Deliberate malformed requests are actually delivered over the
    // possessed mediator endpoint; the receiver, not a CLI validator,
    // rejects unknown versions/operations, duplicates, extra rights.
    for (field, value) in [(0, 2), (2, 2), (3, 3), (4, 2)] {
        let mut probe = [0u8; MSG_BYTES];
        probe[..app_request::REQUEST_BYTES].copy_from_slice(&app_request::READ_REQUEST);
        probe[field] = value;
        if field == 2 { probe[5] = 1; } // explicit second READ (duplicate)
        if call(PERM_OP_ACQUIRE, 0, &mut probe)[0] != PERM_BAD_INPUT {
            fail("malformed request accepted by mediator");
        }
    }
    log_line(|o| o.str("permapp: malformed version/duplicate/rights/op refused by receiver x4"));
    let mut msg = [0u8; MSG_BYTES];
    msg[..app_request::REQUEST_BYTES].copy_from_slice(&app_request::READ_REQUEST);
    let grant = call(PERM_OP_ACQUIRE, 0, &mut msg);
    if grant[0] == PERM_DENIED {
        log_line(|o| o.str("permapp: ACQUIRE denied (no approval)"));
        unsafe { syscall1(SYS_THREAD_EXIT, 0) };
        loop { core::hint::spin_loop() }
    }
    if grant[0] != PERM_OK || grant[1] != 16 { fail("ACQUIRE result"); }
    let mut bearer = [0u8; 16];
    bearer.copy_from_slice(&msg[..16]);
    for offset in (0..512).step_by(32) {
        msg = [0; MSG_BYTES];
        msg[..16].copy_from_slice(&bearer);
        let answer = call(PERM_OP_READ, offset, &mut msg);
        if answer[0] != PERM_OK || answer[1] != 32 { fail("READ result"); }
        for i in 0..32 {
            if msg[i] != pattern_byte(offset as usize + i) { fail("READ wrong file bytes"); }
        }
    }
    log_line(|o| o.str("permapp: PASS 512 mediated arena.txt bytes (no raw fsd cap)"));
    unsafe { syscall1(SYS_THREAD_EXIT, 0) };
    loop { core::hint::spin_loop() }
}
