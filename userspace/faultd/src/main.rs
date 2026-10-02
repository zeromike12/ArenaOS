//! ArenaOS fault-injection service — `faultd` (M6.5, ADR-0028).
//! Spawn-registry image 14, spawned only by the m6 suite's
//! `service_death` test with two kernel-literal grants:
//!
//! - slot 0: `Endpoint` (READ — the serve side),
//! - slot 1: `Notification` (WRITE — how it tells the suite the exact
//!   moment to kill it; write-only so it cannot consume its own
//!   signal),
//! - slot 2: `Notification` (READ — the void it parks on, which
//!   nobody will ever signal).
//!
//! This is the smallest service in ArenaOS, and the only one written
//! to be MURDERED. Every other driver is tested for what it does when
//! things go right; this one exists so that "the server died while
//! holding my request" is a case with a tested answer instead of a
//! situation nobody has ever run.
//!
//! It has no device, no frames, and no cleverness:
//!
//! - `PING` replies immediately — proof the endpoint works before the
//!   fault, and (once a supervisor restarts this image) proof that it
//!   works again after.
//! - `HANG` takes the request and never answers it: it signals the
//!   suite and then parks forever on a notification nobody will ever
//!   signal. The request sits in the endpoint queue in `Delivered`,
//!   which is precisely the state a driver dying mid-I/O leaves behind.
//! - `SHUTDOWN` replies and exits, so the test can also end this
//!   service the ordinary way.
//!
//! The signal-then-park order matters and is not a race: whether the
//! suite destroys this process while it is still runnable or already
//! parked, the client's slot is `Delivered` either way, and that is
//! the only thing the proof depends on.
//!
//! Exit codes: 42 clean shutdown, 85 receive refused, 88 reply
//! refused, 99 panic. There is deliberately no exit code for "killed"
//! — being killed is the point, and a corpse does not report.
#![no_std]
#![no_main]

use core::panic::PanicInfo;

#[path = "../../abi.rs"]
mod abi;
use abi::*;

const SLOT_EP: u64 = 0;
const SLOT_DIAG: u64 = 3; // short-lived test service only
/// The hang signal to the suite — granted WRITE-ONLY, and that is the
/// point: the first version signalled and then parked on the SAME
/// notification, so it consumed its own badge before the suite could
/// see it and the kernel halted on a waiter woken with nothing
/// pending. Rights now make that mistake unrepresentable.
const SLOT_SIGNAL: u64 = 1;
/// The void this service parks on: a notification nobody will ever
/// signal. Granted READ-only.
const SLOT_VOID: u64 = 2;
/// Private test-only opcode: deliberately return an unrequested, inert
/// READ-only notification cap to a linked generic IPC client (ADR-0052).
const OP_UNEXPECTED_CAP: u64 = 3;

const EXIT_RECV: u64 = 85;
const EXIT_REPLY: u64 = 88;

/// # Safety
/// Entered by the spawn protocol exactly as every other image: ring 3,
/// RSP at the derived stack top, grants in slots 0..1.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _start() -> ! {
    // SAFETY: this is the whole program — ABI v1 wrappers over this
    // image's own statics and stack. Single-threaded, no aliases.
    unsafe {
        log("faultd: starting — the service that exists to be killed (M6.5, ADR-0028)");
        let mut w = [0u64; 3];
        let mut inbox = [0u8; MSG_BYTES];
        let mut pings = 0u64;
        loop {
            let r = syscall3(
                SYS_IPC_RECV,
                SLOT_EP,
                w.as_mut_ptr() as u64,
                inbox.as_mut_ptr() as u64,
            );
            if r < 0 {
                log_line(|o| {
                    o.str("faultd: IPC_RECV returned ");
                    o.i64(r);
                });
                fail(EXIT_RECV, "the serve-side receive failed");
            }
            let (op, landed) = (w[1], w[2]);
            if landed != CAP_NONE && op != FAULT_OP_HANG && op != FAULT_OP_SHUTDOWN {
                let _ = syscall1(SYS_CAP_DESTROY, landed);
            }
            match op {
                FAULT_OP_PING => {
                    pings += 1;
                    reply(FAULT_S_OK, pings);
                }
                OP_UNEXPECTED_CAP => {
                    // Only the isolated M6 fault fixture grants COPY on
                    // SLOT_VOID. The caller requested no reply cap; the
                    // generic library must discard this landed reference.
                    let r = syscall5(SYS_IPC_REPLY, SLOT_EP, FAULT_S_OK, 0,
                                     SLOT_VOID, 0);
                    if r < 0 { fail(EXIT_REPLY, "test reply cap refused"); }
                }
                FAULT_OP_HANG => {
                    if !take_diagnostic(landed, SLOT_DIAG) {
                        reply(FAULT_S_BAD_OP, 0);
                        continue;
                    }
                    // The request is now DELIVERED and will never be
                    // answered. Tell the suite that the moment has
                    // arrived, then park forever.
                    log_line(|o| {
                        o.str("faultd: holding a request and hanging on purpose (after ");
                        o.u64(pings);
                        o.str(" ping(s)) — kill me now");
                    });
                    let n = syscall2(SYS_NOTIFY, SLOT_SIGNAL, FAULT_BADGE_HANGING);
                    if n < 0 {
                        log_line(|o| {
                            o.str("faultd: notify returned ");
                            o.i64(n);
                        });
                    }
                    loop {
                        // Nobody ever signals this. The only way out is
                        // proc::destroy, which is the experiment.
                        syscall1(SYS_WAIT, SLOT_VOID);
                    }
                }
                FAULT_OP_SHUTDOWN => {
                    if !take_diagnostic(landed, SLOT_DIAG) {
                        reply(FAULT_S_BAD_OP, 0);
                        continue;
                    }
                    log_line(|o| {
                        o.str("faultd: clean shutdown after ");
                        o.u64(pings);
                        o.str(" ping(s)");
                    });
                    reply(FAULT_S_OK, pings);
                    syscall1(SYS_THREAD_EXIT, EXIT_OK);
                }
                _ => reply(FAULT_S_BAD_OP, 0),
            }
        }
    }
}

/// Reply with a status and one word.
///
/// # Safety
/// Legal exactly once per received call; the caller is inside the
/// serve loop that received one.
unsafe fn reply(status: u64, w1: u64) {
    // SAFETY: wrapper contract.
    let r = unsafe { syscall5(SYS_IPC_REPLY, SLOT_EP, status, w1, CAP_NONE, 0) };
    if r < 0 {
        fail(EXIT_REPLY, "the reply was refused");
    }
}

fn log(s: &str) {
    log_line(|o| o.str(s));
}

fn fail(code: u64, what: &str) -> ! {
    log_line(|o| {
        o.str("faultd: FAIL: ");
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
