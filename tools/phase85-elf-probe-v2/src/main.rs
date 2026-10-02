//! Standalone 8.5 *measurement* fixture: a real static ET_EXEC with useful
//! capability-mediated behavior, not a signed installer or an 8.4 Image cap.
//! A later accepted 8.5 implementation must prove it runs through real
//! registration + SYS_SPAWN; compilation alone cannot prove execution.
#![no_std]
#![no_main]
use core::panic::PanicInfo;
#[path = "../../../userspace/abi.rs"]
mod abi;

#[panic_handler]
fn panic(_: &PanicInfo) -> ! {
    done(99)
}

fn done(code: u64) -> ! {
    unsafe {
        abi::syscall1(abi::SYS_THREAD_EXIT, code);
    }
    loop {
        core::hint::spin_loop();
    }
}

/// Slot zero is deliberately the *only* inherited grant. Query the real
/// package receiver for app.test using exactly the existing v1 ABI; check
/// the reply was an eligible signed version 7 with no returned authority,
/// and that its digest is not empty before announcing success. Failure is a
/// typed exit, never a printed PASS. No raw disk, marker or Power grant.
#[unsafe(no_mangle)]
pub extern "C" fn _start() -> ! {
    let mut held = [0u64; 3];
    let status = unsafe { abi::syscall2(abi::SYS_CAP_DESCRIBE, 0, held.as_mut_ptr() as u64) };
    if status != 0 || held[0] != 2 || held[2] != abi::RIGHTS_WRITE {
        done(43);
    }
    let mut message = [0u8; abi::MSG_BYTES];
    message[..8].copy_from_slice(b"app.test");
    let mut reply = [0u64; 3];
    let rc = unsafe {
        abi::syscall6(
            abi::SYS_IPC_CALL,
            0,
            abi::PKG_OP_QUERY,
            0,
            abi::CAP_NONE,
            reply.as_mut_ptr() as u64,
            message.as_mut_ptr() as u64,
        )
    };
    if rc != 0
        || reply[0] != abi::PKG_ELIGIBLE
        || reply[1] != 8
        || reply[2] != abi::CAP_NONE
        || message[..32].iter().all(|&b| b == 0)
    {
        done(44);
    }
    let msg = b"phase85-v2: version-eight image queried signed stage via inherited endpoint\n";
    let written =
        unsafe { abi::syscall2(abi::SYS_DEBUG_WRITE, msg.as_ptr() as u64, msg.len() as u64) };
    if written != msg.len() as i64 {
        done(45);
    }
    done(42)
}
