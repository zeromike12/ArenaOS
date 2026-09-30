//! One-shot bounded dependency probe. Only the manager holds its Process
//! cap; a hung call cannot hold the manager hostage (ADR-0045).
#![no_std]
#![no_main]
use core::panic::PanicInfo;
#[path = "../../abi.rs"]
mod abi;
use abi::*;

fn fail(reason: &str) -> ! {
    let _ = unsafe { syscall2(SYS_DEBUG_WRITE, reason.as_ptr() as u64, reason.len() as u64) };
    unsafe { syscall1(SYS_THREAD_EXIT, 52) };
    loop {
        core::hint::spin_loop()
    }
}

/// A forged attenuated Notification/READ|COPY lacks DESTROY. Repeated
/// refusal must not strand references in the driver's bounded cap space.
fn challenge_wrong_marker(op: u64) {
    if unsafe { syscall3(SYS_CAP_COPY, 3, 7, RIGHTS_READ | RIGHTS_COPY) } != 0 {
        fail("depcheck: wrong-marker attenuation refused\r\n");
    }
    if unsafe { syscall1(SYS_CAP_DESTROY, 7) } >= 0 {
        fail("depcheck: ordinary copy incorrectly gained DESTROY\r\n");
    }
    for _ in 0..20 {
        if !diagnostic_refused(1, 0, op, 7, RNG_S_BAD_OP) {
            fail("depcheck: wrong-marker repeat refused or leaked server slots\r\n");
        }
    }
}

/// ADR-0049: strictly typed second mode of the already registered
/// image20. The worker cannot manufacture this result channel: its
/// manager spawns it with exactly mediator/W and private result/W.
fn permission_ping_mode() -> ! {
    let mut ep = [0u64; 3];
    let mut result = [0u64; 3];
    if unsafe { syscall2(SYS_CAP_DESCRIBE, 0, ep.as_mut_ptr() as u64) } != 0
        || unsafe { syscall2(SYS_CAP_DESCRIBE, 1, result.as_mut_ptr() as u64) } != 0
        || ep[0] != 2 || ep[2] != RIGHTS_WRITE
        || result[0] != 3 || result[2] != RIGHTS_WRITE
    { fail("depcheck: permission mode grant shape refused\r\n"); }
    let mut unexpected = [0u64; 3];
    if unsafe { syscall2(SYS_CAP_DESCRIBE, 2, unexpected.as_mut_ptr() as u64) } == 0 {
        fail("depcheck: permission mode unexpected third grant\r\n");
    }
    let mut msg = [0u8; MSG_BYTES];
    let mut out = [0u64; 3];
    let rc = unsafe { syscall6(SYS_IPC_CALL, 0, PERM_OP_PING, 0, CAP_NONE,
        out.as_mut_ptr() as u64, msg.as_mut_ptr() as u64) };
    if rc != 0 || out != [PERM_OK, PERM_PING_VERSION, CAP_NONE]
        || msg[..4] != PERM_PING_MAGIC || msg[4..].iter().any(|b| *b != 0)
    { fail("depcheck: permission PING incomplete or refused\r\n"); }
    if unsafe { syscall2(SYS_NOTIFY, 1, MGR_BADGE_PERM_PROBE_OK) } != 0 {
        fail("depcheck: permission result signal refused\r\n");
    }
    unsafe { syscall1(SYS_THREAD_EXIT, 42) };
    loop { core::hint::spin_loop() }
}

#[unsafe(no_mangle)]
pub extern "C" fn _start() -> ! {
    let mut second = [0u64; 3];
    // Original driver mode always has rngd Endpoint in slot 1. A
    // Notification in slot 1 is exclusively the mediator probe mode.
    if unsafe { syscall2(SYS_CAP_DESCRIBE, 1, second.as_mut_ptr() as u64) } == 0
        && second[0] == 3 { permission_ping_mode(); }
    // Slots 0/1 are netd/rngd Endpoint/WRITE, slot 2 is the manager's
    // PRIVATE notification/WRITE. No shutdown or device rights are held.
    if !diagnostic_refused(0, 0, NET_OP_SHUTDOWN, CAP_NONE, NET_S_BAD_OP)
        || !diagnostic_refused(1, 0, RNG_OP_SHUTDOWN, CAP_NONE, RNG_S_BAD_OP)
    { fail("depcheck: production driver poison opcode accepted without marker\r\n"); }
    let text = b"depcheck: production driver poison opcodes refused without marker\r\n";
    let _ = unsafe { syscall2(SYS_DEBUG_WRITE, text.as_ptr() as u64, text.len() as u64) };
    let mut out = [0u64; 3];
    let rc = unsafe {
        syscall6(
            SYS_IPC_CALL,
            0,
            0,
            NET_OP_MAC,
            CAP_NONE,
            out.as_mut_ptr() as u64,
            0,
        )
    };
    if rc != 0 || out[0] != NET_S_OK || out[1] == 0 || out[1] >> 48 != 0 {
        fail("depcheck: netd MAC probe refused\r\n");
    }
    let text = b"depcheck: netd MAC answered\r\n";
    let _ = unsafe { syscall2(SYS_DEBUG_WRITE, text.as_ptr() as u64, text.len() as u64) };
    // COPY on netd's ordinary endpoint is an explicit trusted-manager
    // diagnostic mode, not an ambient pid or a private badge guess.
    let mut authority = [0u64; 3];
    if unsafe { syscall2(SYS_CAP_DESCRIBE, 0, authority.as_mut_ptr() as u64) } != 0
        || authority[0] != 2
        || authority[2] & RIGHTS_WRITE == 0
        || authority[2] & !(RIGHTS_WRITE | RIGHTS_COPY) != 0
    {
        fail("depcheck: unexpected probe grant\r\n");
    }
    if authority[2] & RIGHTS_COPY != 0 {
        // Same real rngd endpoint, deliberately omit the boot-granted
        // marker, then transfer the *different* stack marker. Neither
        // numeric opcode nor a syntactically valid Notification is authority.
        if !diagnostic_refused(1, 0, RNG_OP_FAULT_NEXT_GET, CAP_NONE, RNG_S_BAD_OP)
            || !diagnostic_refused(1, 0, RNG_OP_FAULT_NEXT_GET, 3, RNG_S_BAD_OP)
        { fail("depcheck: rngd accepted ungranted fault authority\r\n"); }
        challenge_wrong_marker(RNG_OP_FAULT_NEXT_GET);
        let text = b"depcheck: rngd refused absent and wrong-object fault marker\r\n";
        let _ = unsafe { syscall2(SYS_DEBUG_WRITE, text.as_ptr() as u64, text.len() as u64) };
        out = [0; 3];
        let r = unsafe {
            syscall6(
                SYS_IPC_CALL,
                1,
                0,
                RNG_OP_FAULT_NEXT_GET,
                4,
                out.as_mut_ptr() as u64,
                0,
            )
        };
        if r != 0 || out[0] != RNG_S_OK {
            fail("depcheck: rngd fault-arm request refused\r\n");
        }
        let text = b"depcheck: private failure fixture armed on rngd\r\n";
        let _ = unsafe { syscall2(SYS_DEBUG_WRITE, text.as_ptr() as u64, text.len() as u64) };
    }
    let mut rng_authority = [0u64; 3];
    if unsafe { syscall2(SYS_CAP_DESCRIBE, 1, rng_authority.as_mut_ptr() as u64) } != 0
        || rng_authority[0] != 2
        || rng_authority[2] & RIGHTS_WRITE == 0
        || rng_authority[2] & !(RIGHTS_WRITE | RIGHTS_COPY) != 0
        || rng_authority[2] & RIGHTS_COPY != 0 && authority[2] & RIGHTS_COPY != 0
    {
        fail("depcheck: unexpected entropy probe grant\r\n");
    }
    if rng_authority[2] & RIGHTS_COPY != 0 {
        if !diagnostic_refused(1, 0, RNG_OP_STALL_NEXT_GET, CAP_NONE, RNG_S_BAD_OP)
            || !diagnostic_refused(1, 0, RNG_OP_STALL_NEXT_GET, 3, RNG_S_BAD_OP)
        { fail("depcheck: rngd accepted ungranted stall authority\r\n"); }
        challenge_wrong_marker(RNG_OP_STALL_NEXT_GET);
        let text = b"depcheck: rngd refused absent and wrong-object stall marker\r\n";
        let _ = unsafe { syscall2(SYS_DEBUG_WRITE, text.as_ptr() as u64, text.len() as u64) };
        out = [0; 3];
        let r = unsafe {
            syscall6(
                SYS_IPC_CALL,
                1,
                0,
                RNG_OP_STALL_NEXT_GET,
                4,
                out.as_mut_ptr() as u64,
                0,
            )
        };
        if r != 0 || out[0] != RNG_S_OK {
            fail("depcheck: rngd stall-arm request refused\r\n");
        }
        let text = b"depcheck: private stall fixture armed on rngd\r\n";
        let _ = unsafe { syscall2(SYS_DEBUG_WRITE, text.as_ptr() as u64, text.len() as u64) };
    }
    // The LENT copy is made before self-map consumes the owned cap.
    if unsafe { syscall1(SYS_ALLOC_FRAME, 5) } <= 0
        || unsafe { syscall3(SYS_CAP_COPY, 5, 6, RIGHTS_ALL) } != 0
    {
        fail("depcheck: entropy buffer allocation refused\r\n");
    }
    let va = unsafe { syscall2(SYS_MAP_MEMORY, 5, 1) };
    if va <= 0 {
        fail("depcheck: entropy frame mapping refused\r\n");
    }
    out = [0; 3];
    let rc = unsafe {
        syscall6(
            SYS_IPC_CALL,
            1,
            64,
            RNG_OP_GET,
            6,
            out.as_mut_ptr() as u64,
            0,
        )
    };
    if rc != 0 || out[0] != RNG_S_OK || out[1] != 64 {
        fail("depcheck: rngd GET probe refused\r\n");
    }
    let first = unsafe { core::ptr::read_volatile(va as *const u8) };
    let mut varied = false;
    for i in 1..64 {
        if unsafe { core::ptr::read_volatile((va as u64 + i) as *const u8) } != first {
            varied = true;
            break;
        }
    }
    if !varied {
        fail("depcheck: entropy device returned constant data\r\n");
    }
    let text = b"depcheck: rngd device completed 64 varied bytes\r\n";
    let _ = unsafe { syscall2(SYS_DEBUG_WRITE, text.as_ptr() as u64, text.len() as u64) };
    if unsafe { syscall2(SYS_NOTIFY, 2, MGR_BADGE_PROBE_OK) } != 0 {
        fail("depcheck: private success signal refused\r\n");
    }
    unsafe { syscall1(SYS_THREAD_EXIT, 42) };
    loop {
        core::hint::spin_loop()
    }
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    fail("depcheck: PANIC\r\n")
}
