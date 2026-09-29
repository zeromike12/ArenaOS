//! ADR-0046: separately boot-granted, opt-in trusted updater. Neither
//! the ordinary reader nor the Power/raw-FS shell has this marker.
//! A trusted shell may stage a TEST intent file on the previous boot;
//! possession of that filename is not update authority. Only the
//! service-issued, transferred Notification cap authorizes SET.
#![no_std]
#![no_main]
use core::panic::PanicInfo;
#[path = "../../abi.rs"]
mod abi;
use abi::*;

fn log(s: &str) {
    let _ = unsafe { syscall2(SYS_DEBUG_WRITE, s.as_ptr() as u64, s.len() as u64) };
}
fn exit(status: u64) -> ! {
    unsafe { syscall1(SYS_THREAD_EXIT, status) };
    loop {
        core::hint::spin_loop()
    }
}
fn fail(s: &str) -> ! {
    log(s);
    exit(78)
}
#[panic_handler]
fn panic(_: &PanicInfo) -> ! {
    fail("configup: PANIC\r\n")
}

fn request(op: u64, cap: u64, msg: &mut [u8; MSG_BYTES]) -> (i64, [u64; 3]) {
    let mut reply = [0u64; 3];
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
    (rc, reply)
}

#[unsafe(no_mangle)]
pub extern "C" fn _start() -> ! {
    let mut ep = [0u64; 3];
    let mut marker = [0u64; 3];
    if unsafe { syscall2(SYS_CAP_DESCRIBE, 0, ep.as_mut_ptr() as u64) } != 0
        || unsafe { syscall2(SYS_CAP_DESCRIBE, 1, marker.as_mut_ptr() as u64) } != 0
        || ep[0] != 2
        || ep[2] != RIGHTS_WRITE | RIGHTS_COPY
        || marker[0] != 3
        || marker[2] != RIGHTS_READ | RIGHTS_COPY | RIGHTS_DESTROY
    {
        fail("configup: exact boot grant refused\r\n");
    }
    for slot in 2..16 {
        if unsafe { syscall2(SYS_CAP_DESCRIBE, slot, ep.as_mut_ptr() as u64) } == 0 {
            fail("configup: unexpected extra capability\r\n");
        }
    }
    let mut msg = [0u8; MSG_BYTES];
    let (rc, plan) = request(CFG_OP_TEST_PLAN, 1, &mut msg);
    if rc != 0 || plan[2] != CAP_NONE {
        fail("configup: plan IPC refused\r\n");
    }
    if plan[0] == CFG_UNSET && plan[1] == 0 {
        log("configup: SKIP (no trusted test intent; no SET)\r\n");
        exit(42);
    }
    if plan[0] == CFG_BAD_INPUT || plan[0] == CFG_CORRUPT {
        log("configup: REFUSED malformed test intent (no SET)\r\n");
        exit(42);
    }
    let wanted: &[u8] = match (plan[0], plan[1]) {
        (CFG_OK, 1) => b"guest-v1",
        (CFG_OK, 2) => b"guest-v2",
        _ => fail("configup: invalid plan status\r\n"),
    };
    msg = [0; MSG_BYTES];
    msg[..2].copy_from_slice(&(wanted.len() as u16).to_le_bytes());
    msg[2..2 + wanted.len()].copy_from_slice(wanted);
    // Independent negative proof from the privileged actor: ordinary
    // endpoint access and wrong-kind COPY never update the receiver.
    for cap in [CAP_NONE, 0] {
        let (rc, denied) = request(CFG_OP_SET, cap, &mut msg);
        if rc != 0 || denied[0] != CFG_DENIED || denied[2] != CAP_NONE {
            fail("configup: receiver accepted marker-free or wrong-kind SET\r\n");
        }
    }
    log("configup: receiver refused absent and wrong-kind markers\r\n");
    // IPC_CALL overwrites the shared inline buffer with each reply.
    // Restore the desired bytes AFTER the negative calls; otherwise a
    // zeroed refusal reply would accidentally request the empty value.
    // Genuine authority does not make an out-of-bounds request valid.
    // This transfer is consumed, so a subsequent valid SET must send
    // a NEW copy of the same held marker rather than relying on a slot.
    msg = [0; MSG_BYTES];
    msg[..2].copy_from_slice(&33u16.to_le_bytes());
    let (rc, bad) = request(CFG_OP_SET, 1, &mut msg);
    if rc != 0 || bad[0] != CFG_BAD_INPUT || bad[2] != CAP_NONE {
        fail("configup: genuine marker bypassed payload bound\r\n");
    }
    log("configup: marked oversized SET refused without writing\r\n");
    msg = [0; MSG_BYTES];
    msg[..2].copy_from_slice(&(wanted.len() as u16).to_le_bytes());
    msg[2..2 + wanted.len()].copy_from_slice(wanted);
    let (rc, done) = request(CFG_OP_SET, 1, &mut msg);
    if rc != 0 || done[2] != CAP_NONE {
        fail("configup: SET IPC failed\r\n");
    }
    match done[0] {
        CFG_COMMITTED => log("configup: SET COMMITTED after exact disk rescan\r\n"),
        CFG_UNCHANGED => {
            log("configup: SET UNCHANGED (same committed bytes; no new generation)\r\n")
        }
        CFG_NO_SPACE => {
            if done[1] == 8 {
                log("configup: SET NO_SPACE (eight immutable generations)\r\n");
            } else if done[1] == 0 {
                log("configup: SET NO_SPACE (disk allocation refused)\r\n");
            } else {
                fail("configup: invalid NO_SPACE detail\r\n");
            }
            exit(42);
        }
        CFG_CORRUPT => {
            log("configup: SET CORRUPT (visible record refused)\r\n");
            exit(42);
        }
        CFG_BAD_INPUT => {
            log("configup: SET BAD_INPUT refused\r\n");
            exit(42);
        }
        CFG_IO | CFG_DEGRADED => {
            fail("configup: ambiguous FS failure; update not acknowledged\r\n")
        }
        _ => fail("configup: wrong SET reply\r\n"),
    }
    let mut observed = [0u8; MSG_BYTES];
    let (rc, read) = request(CFG_OP_READ, CAP_NONE, &mut observed);
    if rc != 0
        || read[0] != CFG_OK
        || read[1] != done[1]
        || read[2] != CAP_NONE
        || observed[..2] != (wanted.len() as u16).to_le_bytes()
        || &observed[2..2 + wanted.len()] != wanted
    {
        fail("configup: committed value not byte-exact on second independent READ\r\n");
    }
    log("configup: TRUSTED UPDATE PASS (receiver marker, disk rescan, independent exact READ)\r\n");
    exit(42)
}
