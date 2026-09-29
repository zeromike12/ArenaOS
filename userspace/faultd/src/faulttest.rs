//! `faulttest` — the client that outlives its service (M6.5, ADR-0028).
//! Spawn-registry image 15, spawned only by the m6 suite's
//! `service_death` test with one grant: slot 0 = `Endpoint` (WRITE,
//! the call side of faultd's endpoint).
//!
//! What it proves, in order:
//!
//! 1. **The service works.** A `PING` is answered; the endpoint and
//!    the reply path are real before anything is broken.
//! 2. **A dead server is an ANSWER, not a hang.** It calls `HANG`,
//!    which faultd takes and never replies to. The suite destroys
//!    faultd while this call is in its hands. A client has no timeout
//!    and no way to observe its server's liveness, so without the
//!    kernel's help this thread would wait forever; instead the call
//!    must return `STATUS_SERVICE_GONE`.
//! 3. **The client keeps its wits.** It reports what it learned and
//!    exits 42 — a service dying is a condition to be HANDLED, and a
//!    client that merely dies alongside its server would prove
//!    nothing worth having.
//!
//! What it deliberately does NOT claim: that the hung request did not
//! happen. `STATUS_SERVICE_GONE` means "no answer is coming", not
//! "nothing was done" — the kernel cannot know which, and a client
//! whose operation is not idempotent must treat it as unknown.
//!
//! Exit codes: 42 verified, 60..64 typed failures, 99 panic.
#![no_std]
#![no_main]

use core::panic::PanicInfo;

#[path = "../../abi.rs"]
mod abi;
use abi::*;

const SLOT_EP: u64 = 0;
/// The "quiet mode" token (M6.5b): a notification granted in slot 1
/// means this instance is the one checking a RESTARTED service — ping
/// and leave, do not ask it to hang. Mode from a capability, not an
/// argument: the same probe pattern inputd and consoled use, and the
/// spawn protocol still has no way to pass a parameter.
const SLOT_QUIET: u64 = 1;
/// Badge sent in quiet mode to say the restarted service ANSWERED.
const QUIET_BADGE: u64 = 1 << 17;
/// Badge used only to discover whether slot 1 holds anything at all.
/// A zero badge is refused by the kernel whatever the slot contains,
/// so probing with zero cannot tell "no capability" from "bad
/// argument" — the first version did exactly that and never entered
/// quiet mode. Badges are bits (ADR-0028), so the suite simply
/// ignores this one.
const PROBE_BADGE: u64 = 1 << 18;

const EXIT_PING_REFUSED: u64 = 60;
const EXIT_PING_STATUS: u64 = 61;
/// The HANG call came back with something other than the typed death:
/// either a reply (impossible — nobody replied) or a different error.
const EXIT_NOT_GONE: u64 = 62;
/// The HANG call returned the typed death, but the client had already
/// been told the service was fine — ordering the proof depends on.
const EXIT_ORDER: u64 = 63;

/// One call. Returns `Ok((status_word, w1))` or `Err(syscall status)`
/// — and the distinction is the whole point of this program: a
/// negative SYSCALL status is the kernel speaking about the service,
/// while a reply word is the service speaking for itself.
///
/// # Safety
/// The endpoint cap is the granted slot 0; single-threaded.
unsafe fn call(op: u64, msg: &mut [u8; MSG_BYTES]) -> Result<(u64, u64), i64> {
    let mut reply = [0u64; 3];
    // SAFETY: wrapper contract; `reply` is on this thread's own stack.
    let r = unsafe {
        syscall6(
            SYS_IPC_CALL,
            SLOT_EP,
            0,
            op,
            if op == FAULT_OP_HANG { SLOT_QUIET } else { CAP_NONE },
            reply.as_mut_ptr() as u64,
            msg.as_mut_ptr() as u64,
        )
    };
    if r < 0 {
        Err(r)
    } else {
        Ok((reply[0], reply[1]))
    }
}

/// # Safety
/// Entered by the spawn protocol exactly as every other image.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _start() -> ! {
    // SAFETY: this is the whole program — ABI v1 wrappers over this
    // image's own statics and stack. Single-threaded, no aliases.
    unsafe {
        let mut msg = [0u8; MSG_BYTES];
        // The probe: a zero badge is always refused, so this asks the
        // kernel "do I hold slot 1 at all?" without signalling anything.
        let quiet = syscall2(SYS_NOTIFY, SLOT_QUIET, PROBE_BADGE) >= 0;
        if quiet {
            log("faulttest: quiet mode — checking a service that was RESTARTED under its clients");
            match call(FAULT_OP_PING, &mut msg) {
                Ok((status, n)) => {
                    if status != FAULT_S_OK {
                        fail(EXIT_PING_STATUS, "the restarted service refused a PING");
                    }
                    log_line(|o| {
                        o.str("faulttest: PASS — the RESTARTED service answered on the SAME endpoint (ping #");
                        o.u64(n);
                        o.str("); the capability I was granted before the crash still works");
                    });
                    let _ = syscall2(SYS_NOTIFY, SLOT_QUIET, QUIET_BADGE);
                    syscall1(SYS_THREAD_EXIT, EXIT_OK);
                }
                Err(e) => {
                    log_line(|o| {
                        o.str("faulttest: the restarted service refused the call: ");
                        o.i64(e);
                    });
                    fail(EXIT_PING_REFUSED, "the restarted service did not answer");
                }
            }
        }
        log("faulttest: client starting — a service is about to die under me");

        // 1. The service is alive and the endpoint works.
        let alive = match call(FAULT_OP_PING, &mut msg) {
            Ok((status, n)) => {
                if status != FAULT_S_OK {
                    fail(EXIT_PING_STATUS, "the service refused a PING");
                }
                n
            }
            Err(e) => {
                log_line(|o| {
                    o.str("faulttest: PING returned ");
                    o.i64(e);
                });
                fail(EXIT_PING_REFUSED, "the PING call was refused");
            }
        };
        log_line(|o| {
            o.str("faulttest: PING answered (ping #");
            o.u64(alive);
            o.str(") — the service is live; now asking it to hang");
        });

        // 2. The call the service will never answer. The suite kills
        //    it while this is in flight; the kernel must answer for it.
        if !diagnostic_refused(SLOT_EP, 0, FAULT_OP_HANG, CAP_NONE, FAULT_S_BAD_OP) {
            fail(EXIT_NOT_GONE, "ordinary endpoint authorized hang");
        }
        log("faulttest: service refused hang without diagnostic marker");
        match call(FAULT_OP_HANG, &mut msg) {
            Ok((status, _)) => {
                log_line(|o| {
                    o.str("faulttest: the HANG call RETURNED a reply (status ");
                    o.i64(status as i64);
                    o.str(") — nobody could have sent one");
                });
                fail(
                    EXIT_NOT_GONE,
                    "a reply arrived for a request nobody answered",
                );
            }
            Err(e) if e == STATUS_SERVICE_GONE => {
                log_line(|o| {
                    o.str("faulttest: PASS — the HANG call returned STATUS_SERVICE_GONE (");
                    o.i64(e);
                    o.str("): the server died holding my request and the kernel said so, ");
                    o.str("instead of leaving me blocked forever");
                });
            }
            Err(e) => {
                log_line(|o| {
                    o.str("faulttest: the HANG call returned ");
                    o.i64(e);
                    o.str(", expected STATUS_SERVICE_GONE (-5)");
                });
                fail(EXIT_NOT_GONE, "the dead service produced the wrong status");
            }
        }

        if alive == 0 {
            fail(EXIT_ORDER, "the ping count makes no sense");
        }
        log("faulttest: a client that outlived its service, and knows it");
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

fn fail(code: u64, what: &str) -> ! {
    log_line(|o| {
        o.str("faulttest: FAIL: ");
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
