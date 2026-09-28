//! `contest` — the console service's client (M6.4, ADR-0027).
//! Spawn-registry image 13, spawned ONLY by the m6 suite's
//! `console_service` test with two kernel-literal grants:
//!
//! - slot 0: `Endpoint` (WRITE — the call side of consoled's service),
//! - slot 1: `Notification` (WRITE — the exit badge the suite waits on,
//!   delivered by the kernel when this thread exits).
//!
//! What it proves, both halves on the real wire:
//!
//! 1. **guest → host.** It sends a fixture line through
//!    `CONSOLE_OP_WRITE`. consoled puts it on the port's transmit
//!    queue and the DEVICE takes it; the harness, connected to the
//!    other end of the chardev socket, reads those exact bytes. The
//!    assertion lives on the host, where the evidence is.
//! 2. **host → guest.** It then reads with `CONSOLE_OP_READ` until it
//!    has the answer the harness sent back, and verifies it
//!    byte-for-byte. Bytes that arrive between calls are buffered by
//!    the driver, so the check is on CONTENT, not timing.
//!
//! A port whose far end nobody is attached to is answered honestly:
//! `CONSOLE_S_NO_DATA` (the spawner called the wait off) exits 68, the
//! m6 suite turns that into a SKIP, and a machine with a console
//! device but no listener still boots green.
//!
//! Exit codes: 42 verified, 68 nobody was listening (an honest skip),
//! 60..67 typed failures, 99 panic.
#![no_std]
#![no_main]

use core::panic::PanicInfo;

#[path = "../../abi.rs"]
mod abi;
use abi::*;

const SLOT_EP: u64 = 0;

const EXIT_CALL: u64 = 60;
const EXIT_WRITE: u64 = 61;
const EXIT_STATUS: u64 = 63;
const EXIT_COUNT: u64 = 64;
const EXIT_MISMATCH: u64 = 65;
const EXIT_SHORT: u64 = 66;
const EXIT_POISON: u64 = 67;
/// Not a failure: nothing was ever sent from the host, because nobody
/// was attached to the port's far end.
const EXIT_NO_DATA: u64 = 68;

/// What this client sends OUT the port. The harness matches this exact
/// run of bytes on its socket — a marker no other output can forge.
const FIXTURE_OUT: &[u8] = b"contest: hello from ArenaOS\n";

/// What the harness sends BACK, and this client verifies. Ends in a
/// newline so the reader knows where the answer stops without
/// depending on how the host chose to chunk it.
const FIXTURE_IN: &[u8] = b"host-says-hello\n";

/// One request/reply exchange with consoled.
///
/// # Safety
/// The endpoint cap is in slot 0 (the spawn grant); `payload` is this
/// image's own memory; single-threaded.
unsafe fn request(op: u64, w0: u64, msg: &mut [u8; MSG_BYTES]) -> (u64, u64) {
    let mut reply = [0u64; 3];
    // SAFETY: wrapper contract. IPC v1.1: `msg` is IN/OUT — the
    // kernel snapshots it as the request's inline message and
    // OVERWRITES it with the reply's, so a caller that needs both
    // must read the reply before staging the next request.
    let r = unsafe {
        syscall6(
            SYS_IPC_CALL,
            SLOT_EP,
            w0,
            op,
            CAP_NONE,
            reply.as_mut_ptr() as u64,
            msg.as_mut_ptr() as u64,
        )
    };
    if r < 0 {
        log_line(|o| {
            o.str("contest: IPC_CALL returned ");
            o.i64(r);
        });
        fail(EXIT_CALL, "the call was refused");
    }
    (reply[0], reply[1])
}

/// # Safety
/// Entered by the spawn protocol exactly as every other image.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _start() -> ! {
    // SAFETY: this is the whole program — ABI v1 wrappers over this
    // image's own statics and stack. Single-threaded, no aliases.
    unsafe {
        log("contest: console-service client starting — a port round trip through the service");
        // The ONE inline buffer, in and out (the IPC v1.1 contract).
        let mut msg = [0u8; MSG_BYTES];

        // 1. guest → host. The reply lands only after the DEVICE has
        //    taken the bytes, so a success here already means the
        //    transmit queue completed.
        msg[..FIXTURE_OUT.len()].copy_from_slice(FIXTURE_OUT);
        let (status, sent) = request(CONSOLE_OP_WRITE, FIXTURE_OUT.len() as u64, &mut msg);
        if status != CONSOLE_S_OK {
            log_line(|o| {
                o.str("contest: WRITE reported status ");
                o.i64(status as i64);
            });
            fail(EXIT_WRITE, "the service refused the write");
        }
        if sent != FIXTURE_OUT.len() as u64 {
            fail(EXIT_COUNT, "the service sent a different byte count");
        }
        log_line(|o| {
            o.str("contest: wrote ");
            o.u64(sent);
            o.str(" byte(s) out the port — the harness reads them off the socket");
        });

        // 2. host → guest. Collect until the fixture's length is in
        //    hand; the driver buffers whatever arrives between calls.
        let mut got = [0u8; 64];
        let mut have = 0usize;
        while have < FIXTURE_IN.len() {
            let want = core::cmp::min(
                (FIXTURE_IN.len() - have) as u64,
                core::cmp::min(CONSOLE_MSG_MAX, (got.len() - have) as u64),
            );
            let (status, n) = request(CONSOLE_OP_READ, want, &mut msg);
            if status == CONSOLE_S_NO_DATA {
                // Nobody was attached to the port. Poison the service
                // so it exits cleanly, then report the honest outcome:
                // nothing was proved, and nothing is claimed.
                log("contest: nothing arrived on the port — nobody was attached to its far end");
                let (st, _) = request(CONSOLE_OP_SHUTDOWN, 0, &mut msg);
                if st != CONSOLE_S_OK {
                    fail(EXIT_POISON, "the poison shutdown was refused");
                }
                note_exit(
                    EXIT_NO_DATA,
                    "no bytes arrived from the host (nobody was listening on the port)",
                );
            }
            if status != CONSOLE_S_OK {
                log_line(|o| {
                    o.str("contest: READ reported status ");
                    o.i64(status as i64);
                });
                fail(EXIT_STATUS, "the service reported an error status");
            }
            if n == 0 || n > want {
                fail(EXIT_COUNT, "the service returned an impossible byte count");
            }
            got[have..have + n as usize].copy_from_slice(&msg[..n as usize]);
            have += n as usize;
        }
        if got[..have] != *FIXTURE_IN {
            log_line(|o| {
                o.str("contest: received ");
                o.u64(have as u64);
                o.str(" byte(s), not the sequence the harness sent");
            });
            fail(EXIT_MISMATCH, "the received bytes are not what was sent");
        }
        log_line(|o| {
            o.str("contest: PASS — the port carried bytes BOTH ways: ");
            o.u64(sent);
            o.str(" out, ");
            o.u64(have as u64);
            o.str(" back in, verified byte-for-byte");
        });

        // 3. Poison the service and check its accounting.
        let (status, out_total) = request(CONSOLE_OP_SHUTDOWN, 0, &mut msg);
        if status != CONSOLE_S_OK {
            fail(EXIT_POISON, "the poison shutdown was refused");
        }
        if out_total < sent {
            fail(EXIT_SHORT, "the service under-counted the bytes it sent");
        }
        syscall1(SYS_THREAD_EXIT, EXIT_OK);
    }
    #[allow(unreachable_code)]
    loop {
        core::hint::spin_loop();
    }
}

fn log(s: &str) {
    log_line(|o| o.str(s));
}

/// Exit with a code that is NOT a failure (nobody was listening).
fn note_exit(code: u64, what: &str) -> ! {
    log_line(|o| {
        o.str("contest: ");
        o.str(what);
        o.str(" — exiting ");
        o.u64(code);
    });
    // SAFETY: thread_exit diverges; the code is the m6 contract.
    unsafe { syscall1(SYS_THREAD_EXIT, code) };
    #[allow(unreachable_code)]
    loop {
        core::hint::spin_loop();
    }
}

fn fail(code: u64, what: &str) -> ! {
    log_line(|o| {
        o.str("contest: FAIL: ");
        o.str(what);
        o.str(" — exiting ");
        o.u64(code);
    });
    // SAFETY: thread_exit diverges; the code is the m6 contract.
    unsafe { syscall1(SYS_THREAD_EXIT, code) };
    #[allow(unreachable_code)]
    loop {
        core::hint::spin_loop();
    }
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    fail(EXIT_PANIC, "panic")
}
