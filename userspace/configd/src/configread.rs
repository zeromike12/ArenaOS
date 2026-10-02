//! One-shot ordinary configuration reader. A boot root gives it ONLY
//! Endpoint/WRITE|COPY, no fsd endpoint, Power, or update marker. The
//! extra COPY is to send a WRONG-KIND cap to the receiving service;
//! it does not confer config update authority (ADR-0046/0047).
#![no_std]
#![no_main]
use core::panic::PanicInfo;
#[path = "../../abi.rs"]
mod abi;
use abi::*;

fn log(s: &str) {
    let _ = unsafe { syscall2(SYS_DEBUG_WRITE, s.as_ptr() as u64, s.len() as u64) };
}
fn fail(s: &str) -> ! {
    log(s);
    unsafe { syscall1(SYS_THREAD_EXIT, 78) };
    loop {
        core::hint::spin_loop()
    }
}
#[panic_handler]
fn panic(_: &PanicInfo) -> ! {
    fail("configread: PANIC\r\n")
}

fn request(op: u64, cap: u64) -> (i64, [u64; 3], [u8; MSG_BYTES]) {
    let mut reply = [0u64; 3];
    let mut msg = [0u8; MSG_BYTES];
    let rc = unsafe {
        syscall6(
            SYS_IPC_CALL,
            0,
            op,
            0,
            cap,
            reply.as_mut_ptr() as u64,
            msg.as_mut_ptr() as u64,
        )
    };
    (rc, reply, msg)
}

#[unsafe(no_mangle)]
pub extern "C" fn _start() -> ! {
    let mut held = [0u64; 3];
    if unsafe { syscall2(SYS_CAP_DESCRIBE, 0, held.as_mut_ptr() as u64) } != 0
        || held[0] != 2
        || held[2] != (RIGHTS_WRITE | RIGHTS_COPY)
    {
        fail("configread: ordinary endpoint grant invalid\r\n");
    }
    for i in 0..20 {
        let (rc, reply, _) = request(CFG_OP_SET, if i % 2 == 0 { CAP_NONE } else { 0 });
        if rc != 0 || reply[0] != CFG_DENIED || reply[2] != CAP_NONE {
            fail("configread: SET accepted without marker or with wrong-kind cap\r\n");
        }
    }
    log("configread: SET absent/wrong-kind refused x20 by receiver\r\n");
    let (rc, reply, msg) = request(CFG_OP_READ, CAP_NONE);
    if rc != 0 || reply[2] != CAP_NONE {
        fail("configread: READ IPC failed\r\n");
    }
    match reply[0] {
        CFG_UNSET if reply[1] == 0 => log("configread: READ UNSET\r\n"),
        CFG_OK if (1..=8).contains(&reply[1]) => {
            let len = u16::from_le_bytes([msg[0], msg[1]]) as usize;
            if len > 32 {
                fail("configread: READ invalid length\r\n");
            }
            if reply[1] == 1 && len == 7 && &msg[2..9] == b"seed-v1" {
                log("configread: READ VALUE seed-v1 seq1 (exact host fixture bytes)\r\n");
            } else {
                log("configread: READ VALUE\r\n");
            }
        }
        CFG_CORRUPT => log("configread: READ CORRUPT (fail-closed)\r\n"),
        _ => fail("configread: READ returned invalid status\r\n"),
    }
    // Normal read still works after twenty wrong-cap deliveries. No
    // wrong reference is stranded in configd's bounded cap table.
    let (rc2, second, other) = request(CFG_OP_READ, CAP_NONE);
    if rc2 != 0 || reply != second || msg != other {
        fail("configread: refused calls altered visible configuration\r\n");
    }
    log("configread: ORDINARY READ BOUNDARY PASS (no fsd or marker grant)\r\n");
    unsafe { syscall1(SYS_THREAD_EXIT, 42) };
    loop {
        core::hint::spin_loop()
    }
}
