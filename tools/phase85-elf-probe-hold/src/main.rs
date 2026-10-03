//! Test-only signed v7: first launch exits; with staged v8 visible, parks in
//! cooperative package IPC with no extra grant until held Process-cap STOP.
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
#[unsafe(no_mangle)]
pub extern "C" fn _start() -> ! {
    let mut desc = [0u64; 3];
    if unsafe { abi::syscall2(abi::SYS_CAP_DESCRIBE, 0, desc.as_mut_ptr() as u64) } != 0
        || desc[0] != 2
        || desc[2] != abi::RIGHTS_WRITE
    {
        done(43);
    }
    let mut req = [0u8; abi::MSG_BYTES];
    req[..8].copy_from_slice(b"app.test");
    let mut r = [0u64; 3];
    if unsafe {
        abi::syscall6(
            abi::SYS_IPC_CALL,
            0,
            abi::PKG_OP_QUERY,
            0,
            abi::CAP_NONE,
            r.as_mut_ptr() as u64,
            req.as_mut_ptr() as u64,
        )
    } != 0
        || r[0] != abi::PKG_ELIGIBLE
        || r[2] != abi::CAP_NONE
        || req[..32] == [0; 32]
    {
        done(44);
    }
    if r[1] == 7 {
        let line = b"phase85-hold: first signed v7 launch exited normally\n";
        if unsafe {
            abi::syscall2(
                abi::SYS_DEBUG_WRITE,
                line.as_ptr() as u64,
                line.len() as u64,
            )
        } != line.len() as i64
        {
            done(45);
        }
        done(42);
    }
    if r[1] != 8 {
        done(46);
    }
    let line = b"phase85-hold: old signed v7 child ALIVE until manager Process-cap STOP\n";
    if unsafe {
        abi::syscall2(
            abi::SYS_DEBUG_WRITE,
            line.as_ptr() as u64,
            line.len() as u64,
        )
    } != line.len() as i64
    {
        done(47);
    }
    // Scheduler preemption is disarmed outside its early self-tests. Park
    // via the existing package endpoint's malformed no-mutation request.
    loop {
        let mut msg = [0u8; abi::MSG_BYTES];
        let mut ans = [0u64; 3];
        if unsafe {
            abi::syscall6(
                abi::SYS_IPC_CALL,
                0,
                u64::MAX,
                0,
                abi::CAP_NONE,
                ans.as_mut_ptr() as u64,
                msg.as_mut_ptr() as u64,
            )
        } != 0
            || ans[0] != abi::PKG_BAD_FORMAT
            || ans[2] != abi::CAP_NONE
        {
            done(48);
        }
    }
}
