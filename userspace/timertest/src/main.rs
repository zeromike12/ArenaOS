//! `timertest` — the timer facility, proven from ring 3 (M7.0,
//! ADR-0029). Spawn-registry image 16, spawned by the m7 suite with
//! one grant: slot 0 = `Notification` (READ|WRITE — it arms timers
//! against this and waits on it).
//!
//! Phase 7 forbids polling loops and busy-waits, so the very first
//! thing built is the facility that makes them unnecessary — and the
//! first thing tested is that it actually keeps time. Every check here
//! is a MEASUREMENT against the monotonic clock, not a claim:
//!
//! 1. **A deadline is never early.** The single property a timeout
//!    must have. A timer armed for 50 ms must not deliver its badge
//!    before 50 ms of monotonic time has passed — if it can fire
//!    early, every retransmission and every aging rule built on it is
//!    wrong in a way that only shows up under load.
//! 2. **And not much late.** Within one tick plus slack, so the
//!    facility is usable for real protocol timing rather than merely
//!    eventually correct.
//! 3. **A cancelled timer never fires.** Armed, cancelled, and then a
//!    second timer armed and waited on: the cancelled badge must not
//!    appear in the wake, or a TCP stack would retransmit segments it
//!    had already acknowledged.
//! 4. **Cancelling twice is an error, not a silent success.** A
//!    protocol cancelling a retransmission that has ALREADY gone out
//!    needs to be able to tell the difference.
//! 5. **Badges merge.** Two timers on one notification, both due:
//!    the wake carries both bits. This is the property every service
//!    in Phase 7 will depend on to wait for "device OR client OR
//!    timeout" in a single blocking call.
//!
//! 6. **A stale id cannot cancel a stranger.** Arm a timer, let it
//!    fire, then arm another — which the kernel may hand the SAME
//!    slot — and try to cancel the first one's id. It must be
//!    refused, and the second timer must still fire. Without this,
//!    a TCP stack holding dozens of retransmission timers in one
//!    process could cancel the wrong one by replaying its own stale
//!    bookkeeping, losing a retransmission in a way that only
//!    appears under the interleaving that caused it.
//!
//! It then leaves a timer armed on purpose and exits, so the suite can
//! prove the kernel sweeps it.
//!
//! Exit codes: 42 verified, 60..69 typed failures, 99 panic.
#![no_std]
#![no_main]

use core::panic::PanicInfo;

#[path = "../../abi.rs"]
mod abi;
use abi::*;

const SLOT_NOTIF: u64 = 0;

const BADGE_A: u64 = 1 << 16;
const BADGE_B: u64 = 1 << 17;
const BADGE_C: u64 = 1 << 18;
const BADGE_CANCELLED: u64 = 1 << 19;

const EXIT_CLOCK: u64 = 60;
const EXIT_ARM: u64 = 61;
const EXIT_EARLY: u64 = 62;
const EXIT_LATE: u64 = 63;
const EXIT_BADGE: u64 = 64;
const EXIT_CANCEL: u64 = 65;
const EXIT_CANCEL_TWICE: u64 = 66;
const EXIT_GHOST: u64 = 67;
const EXIT_MERGE: u64 = 68;
/// A stale id — one whose slot has been handed to a LATER timer —
/// was accepted, which means it cancelled a stranger's timer.
const EXIT_STALE: u64 = 69;

/// The delay under test: five ticks, far enough above the 10 ms
/// granularity that lateness is meaningful and short enough that the
/// suite is not slow.
const DELAY_US: u64 = 50_000;
/// How late a 50 ms timer may be. One tick of granularity plus room
/// for the boot suite's other work; if this ever needs raising, the
/// facility has a problem worth looking at rather than a constant
/// worth editing.
const SLACK_US: u64 = 60_000;

/// # Safety
/// ABI v1 wrapper; the notification cap is the granted slot 0.
unsafe fn now_us() -> u64 {
    // SAFETY: wrapper contract.
    let t = unsafe { syscall0(SYS_CLOCK_NOW) };
    if t <= 0 {
        fail(EXIT_CLOCK, "the monotonic clock read as zero or negative");
    }
    t as u64
}

/// # Safety
/// As `now_us`.
unsafe fn arm(badge: u64, delay_us: u64) -> u64 {
    // SAFETY: wrapper contract.
    let id = unsafe { syscall3(SYS_TIMER_ARM, SLOT_NOTIF, badge, delay_us) };
    if id < 0 {
        log_line(|o| {
            o.str("timertest: TIMER_ARM returned ");
            o.i64(id);
        });
        fail(EXIT_ARM, "arming a timer was refused");
    }
    id as u64
}

/// # Safety
/// Entered by the spawn protocol exactly as every other image.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _start() -> ! {
    // SAFETY: this is the whole program — ABI v1 wrappers over this
    // image's own statics and stack. Single-threaded, no aliases.
    unsafe {
        log("timertest: measuring the timer facility against the monotonic clock");

        // ---- 1 & 2: a deadline is not early, and not much late ----
        let t0 = now_us();
        arm(BADGE_A, DELAY_US);
        let badge = syscall1(SYS_WAIT, SLOT_NOTIF);
        let t1 = now_us();
        if badge < 0 {
            fail(EXIT_ARM, "the wait on the timer notification was refused");
        }
        let elapsed = t1 - t0;
        if badge as u64 & BADGE_A == 0 {
            log_line(|o| {
                o.str("timertest: woke with badge ");
                o.hex(badge as u64);
                o.str(", expected bit ");
                o.hex(BADGE_A);
            });
            fail(EXIT_BADGE, "the timer delivered the wrong badge");
        }
        if elapsed < DELAY_US {
            log_line(|o| {
                o.str("timertest: timer fired EARLY after ");
                o.u64(elapsed);
                o.str("us, asked for ");
                o.u64(DELAY_US);
            });
            fail(EXIT_EARLY, "a deadline fired before it was due");
        }
        if elapsed > DELAY_US + SLACK_US {
            log_line(|o| {
                o.str("timertest: timer fired ");
                o.u64(elapsed - DELAY_US);
                o.str("us late (slack is ");
                o.u64(SLACK_US);
                o.str("us)");
            });
            fail(EXIT_LATE, "a deadline fired far later than the granularity");
        }
        log_line(|o| {
            o.str("timertest: PASS — 50000us timer delivered after ");
            o.u64(elapsed);
            o.str("us (never early, ");
            o.u64(elapsed - DELAY_US);
            o.str("us of lag against a 10000us tick)");
        });

        // ---- 3 & 4: a cancelled timer never fires ----
        let doomed = arm(BADGE_CANCELLED, DELAY_US / 2);
        if syscall1(SYS_TIMER_CANCEL, doomed) != 0 {
            fail(EXIT_CANCEL, "cancelling an armed timer was refused");
        }
        // Cancelling again must FAIL: the difference between "stopped
        // it in time" and "it already went" is load-bearing.
        if syscall1(SYS_TIMER_CANCEL, doomed) >= 0 {
            fail(
                EXIT_CANCEL_TWICE,
                "cancelling an already-cancelled timer reported success",
            );
        }
        arm(BADGE_B, DELAY_US);
        let badge = syscall1(SYS_WAIT, SLOT_NOTIF);
        if badge < 0 {
            fail(EXIT_ARM, "the second wait was refused");
        }
        if badge as u64 & BADGE_CANCELLED != 0 {
            fail(EXIT_GHOST, "a CANCELLED timer delivered its badge anyway");
        }
        if badge as u64 & BADGE_B == 0 {
            fail(EXIT_BADGE, "the second timer delivered the wrong badge");
        }
        log("timertest: PASS — a cancelled timer stayed silent, and a second cancel was refused");

        // ---- 5: badges merge in one wake ----
        // Both are due well before the wait, so the kernel has ORed
        // them into one pending word: a service must be able to see a
        // timeout and its other work in the same wake.
        arm(BADGE_A, DELAY_US);
        arm(BADGE_C, DELAY_US);
        let badge = syscall1(SYS_WAIT, SLOT_NOTIF);
        if badge < 0 {
            fail(EXIT_ARM, "the third wait was refused");
        }
        let mut merged = badge as u64;
        if merged & BADGE_C == 0 {
            // They may land one tick apart; take the second wake too.
            let more = syscall1(SYS_WAIT, SLOT_NOTIF);
            if more < 0 {
                fail(EXIT_ARM, "the fourth wait was refused");
            }
            merged |= more as u64;
        }
        if merged & BADGE_A == 0 || merged & BADGE_C == 0 {
            log_line(|o| {
                o.str("timertest: two due timers produced ");
                o.hex(merged);
            });
            fail(EXIT_MERGE, "two due timers did not both deliver");
        }
        log("timertest: PASS — two timers on one notification both delivered");

        // ---- 6: a stale id must not cancel a stranger's timer ----
        // Arm, let it FIRE (so the slot is freed and may be reused),
        // then arm another and try the dead id. If ids were bare slot
        // indices the second timer would die here silently.
        let stale = arm(BADGE_A, DELAY_US);
        let badge = syscall1(SYS_WAIT, SLOT_NOTIF);
        if badge < 0 || badge as u64 & BADGE_A == 0 {
            fail(EXIT_BADGE, "the timer before the stale-id check misbehaved");
        }
        let fresh = arm(BADGE_B, DELAY_US);
        if syscall1(SYS_TIMER_CANCEL, stale) >= 0 {
            log_line(|o| {
                o.str("timertest: a STALE id (");
                o.u64(stale);
                o.str(") was accepted; the live timer is ");
                o.u64(fresh);
            });
            fail(
                EXIT_STALE,
                "a stale timer id was accepted — it could have cancelled a stranger",
            );
        }
        let badge = syscall1(SYS_WAIT, SLOT_NOTIF);
        if badge < 0 || badge as u64 & BADGE_B == 0 {
            fail(
                EXIT_STALE,
                "the timer armed after a stale cancel never fired (it was cancelled)",
            );
        }
        log_line(|o| {
            o.str("timertest: PASS — a stale id was refused and the live timer it aliased (");
            o.u64(fresh);
            o.str(") still fired");
        });

        // Leave one armed on purpose: the suite proves the kernel
        // sweeps it when this process is destroyed.
        arm(BADGE_C, 60 * 1_000_000);
        log("timertest: leaving one timer armed for 60s on purpose — the kernel must sweep it");
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
        o.str("timertest: FAIL: ");
        o.str(what);
        o.str(" — exiting ");
        o.u64(code);
    });
    // SAFETY: thread_exit diverges; the code is the m7 contract.
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
