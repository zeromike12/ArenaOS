#![no_std]
#![no_main]
// Fixed built-in request: arena.txt READ. Image/name alone grants nothing;
// this process inherits only mediator WRITE|COPY, not fsd, rngd or marker.
#[path = "../../abi.rs"]
mod abi;
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
    let mut msg = [0u8; MSG_BYTES];
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
