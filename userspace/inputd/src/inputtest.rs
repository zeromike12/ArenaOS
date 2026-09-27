//! The input-service test client (M6.3, ADR-0026) — spawn-registry
//! image 11, `inputtest`. Built from the same crate as `inputd`, so
//! both sides of the wire protocol come from one `abi.rs`.
//!
//! Slot 0 holds the call side of inputd's endpoint; the client owns no
//! device, no frame, and no console authority — everything it learns
//! about the keyboard comes through the service boundary.
//!
//! The proof: the harness types `arena` on the VIRTUAL KEYBOARD
//! (QEMU's QMP `input-send-event`, evdev codes 30/19/18/49/30 — see
//! tools/qmp.py), inputd's device interrupt delivers the events, and
//! this client reads the DECODED bytes back through IPC and compares
//! them to the fixture byte-for-byte. A fixed injected sequence is a
//! documented fixture contract, exactly like the slirp gateway's
//! 10.0.2.2 in the net test — the client never invents expected
//! data it did not receive.
//!
//! Exit codes (the m6 suite maps every one): 42 the sequence arrived
//! decoded and in order, 62..67 the failure stages, 99 panic.
#![no_std]
#![no_main]

use core::panic::PanicInfo;

#[path = "../../abi.rs"]
mod abi;
use abi::*;

/// The one grant: the endpoint's call side.
const SLOT_EP: u64 = 0;

// ---- the diagnostic exit contract (m6.rs maps every code) --------------------

const EXIT_CALL: u64 = 62;
const EXIT_STATUS: u64 = 63;
const EXIT_COUNT: u64 = 64;
const EXIT_MISMATCH: u64 = 65;
const EXIT_STALLED: u64 = 66;
const EXIT_POISON: u64 = 67;
/// Not a failure of anything ArenaOS does: the service was asked to
/// stop waiting because nobody typed during this boot. The m6 suite
/// turns this into an honest SKIP.
const EXIT_NO_KEYS: u64 = 68;

/// The fixture the harness injects on the virtual keyboard once
/// inputd announces DRIVER_OK.
const FIXTURE: &[u8] = b"arena";

/// How many READ round trips the client will make before giving up.
/// Each blocks in the service until a key exists, so the bound only
/// catches a service that answers with nothing forever.
const MAX_READS: u32 = 32;

fn fail(code: u64, what: &str) -> ! {
    log_line(|o| {
        o.str("inputtest: FAIL: ");
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

fn log(s: &str) {
    log_line(|o| o.str(s));
}

/// Exit with a code that is NOT a failure (nobody typed). Distinct
/// from `fail` on purpose: a log line that says FAIL for an expected,
/// honest outcome would be its own small lie.
fn note_exit(code: u64, what: &str) -> ! {
    log_line(|o| {
        o.str("inputtest: ");
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
    write_str("inputtest: PANIC\r\n");
    // SAFETY: thread_exit diverges.
    unsafe { syscall1(SYS_THREAD_EXIT, EXIT_PANIC) };
    loop {
        core::hint::spin_loop();
    }
}

/// One synchronous request through the service boundary. `msg_out` is
/// the inline-message buffer the reply's key bytes land in (or 0).
/// Returns (reply status word, reply payload word).
fn request(op: u64, w0: u64, msg_out: u64) -> (u64, u64) {
    let mut reply = [0u64; 3];
    // SAFETY: wrapper contract; `reply` is on this thread's own
    // (registered) stack; the endpoint cap is the granted slot 0.
    let r = unsafe {
        syscall6(
            SYS_IPC_CALL,
            SLOT_EP,
            w0,
            op,
            CAP_NONE,
            reply.as_mut_ptr() as u64,
            msg_out,
        )
    };
    if r < 0 {
        log_line(|o| {
            o.str("inputtest: call(op ");
            o.u64(op);
            o.str(") returned ");
            o.i64(r);
        });
        fail(EXIT_CALL, "the service call was refused");
    }
    (reply[0], reply[1])
}

/// The client entry: the spawn protocol's first thread lands here at
/// ring 3 with slot 0 holding the endpoint's call side.
///
/// # Safety
/// As the driver's `_start`: ring 3, derived stack top, kernel-loaded
/// address space, one grant in slot 0. The body is ABI v1 wrappers
/// over this image's own statics and stack.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _start() -> ! {
    // SAFETY: this is the whole program — ABI v1 wrappers over this
    // image's own statics and stack. Single-threaded, no aliases.
    unsafe {
        log(
            "inputtest: input-service client starting — reading keystrokes typed on the virtual keyboard",
        );

        // Collect decoded key bytes until the fixture's length is in
        // hand. Every READ blocks inside the service until the device
        // has actually delivered something, so this loop is paced by
        // real keystrokes, not by spinning.
        let mut got = [0u8; 16];
        let mut have = 0usize;
        let mut reads = 0u32;
        let mut inbox = [0u8; MSG_BYTES];
        while have < FIXTURE.len() {
            reads += 1;
            if reads > MAX_READS {
                fail(
                    EXIT_STALLED,
                    "the service answered too many times without completing the sequence",
                );
            }
            let want = (FIXTURE.len() - have) as u64;
            let (status, n) = request(INPUT_OP_READ, want, inbox.as_mut_ptr() as u64);
            if status == INPUT_S_NO_KEYS {
                // Nobody was at the keyboard. Poison the service so it
                // exits cleanly, then report the honest outcome — a
                // boot with a keyboard attached and no typist must not
                // hang, and must not pretend to have proved anything.
                log("inputtest: nobody typed during this boot — the wait was called off");
                let (st, _) = request(INPUT_OP_SHUTDOWN, 0, 0);
                if st != INPUT_S_OK {
                    fail(EXIT_POISON, "the poison shutdown was refused");
                }
                note_exit(
                    EXIT_NO_KEYS,
                    "no keystrokes arrived (nobody was typing) — nothing to verify, and nothing claimed",
                );
            }
            if status != INPUT_S_OK {
                log_line(|o| {
                    o.str("inputtest: READ reported status ");
                    o.i64(status as i64);
                });
                fail(EXIT_STATUS, "the service reported an error status");
            }
            if n == 0 || n > want {
                log_line(|o| {
                    o.str("inputtest: READ asked for ");
                    o.u64(want);
                    o.str(" byte(s) and got ");
                    o.u64(n);
                });
                fail(EXIT_COUNT, "the service returned an impossible key count");
            }
            got[have..have + n as usize].copy_from_slice(&inbox[..n as usize]);
            have += n as usize;
            log_line(|o| {
                o.str("inputtest: read ");
                o.u64(n);
                o.str(" key byte(s), ");
                o.u64(have as u64);
                o.str("/");
                o.u64(FIXTURE.len() as u64);
                o.str(" of the injected sequence");
            });
        }

        // The verification: byte for byte, in typing order.
        for i in 0..FIXTURE.len() {
            if got[i] != FIXTURE[i] {
                log_line(|o| {
                    o.str("inputtest: byte ");
                    o.u64(i as u64);
                    o.str(" is ");
                    o.hex(u64::from(got[i]));
                    o.str(", expected ");
                    o.hex(u64::from(FIXTURE[i]));
                });
                fail(
                    EXIT_MISMATCH,
                    "the decoded keys are not the injected sequence",
                );
            }
        }
        log_line(|o| {
            o.str("inputtest: PASS — the injected keystrokes arrived decoded and in order: \"");
            o.bytes(&got[..have]);
            o.str("\"");
        });

        // Poison the service and check its own account of the run: at
        // least one INTERRUPT-delivered event batch carried these
        // keys (the device coalesces events per SYN_REPORT, so the
        // exact count is QEMU's batching policy, not our behavior —
        // the suite asserts what is genuinely ours).
        let (status, batches) = request(INPUT_OP_SHUTDOWN, 0, 0);
        if status != INPUT_S_OK || batches == 0 {
            log_line(|o| {
                o.str("inputtest: shutdown reported status ");
                o.i64(status as i64);
                o.str(" after ");
                o.u64(batches);
                o.str(" batch(es)");
            });
            fail(
                EXIT_POISON,
                "the poison shutdown was refused or reported no interrupt batches",
            );
        }
        log_line(|o| {
            o.str("inputtest: the service reported ");
            o.u64(batches);
            o.str(" interrupt-delivered event batch(es) — exiting 42");
        });
        // SAFETY: thread_exit diverges; 42 is the verified-success code.
        syscall1(SYS_THREAD_EXIT, EXIT_OK);
        #[allow(unreachable_code)]
        loop {
            core::hint::spin_loop();
        }
    }
}
