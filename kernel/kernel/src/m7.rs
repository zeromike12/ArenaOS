//! Milestone 7 test suite — Phase 7 opens with the facility every
//! protocol in it will depend on (step 7.0, ADR-0029).
//!
//! Runs in `kmain` after the m6 RESULT line; suite discipline as
//! m3/m4/m5/m6 (self-contained, ring-3 claims proven by real images,
//! kernel claims proven by counted state):
//!
//! 1. `timer_facility` — REAL timers, measured. The kernel spawns
//!    `timertest` (registry image 16) with one notification, and the
//!    image arms timers against it and times them with the monotonic
//!    clock `SYS_CLOCK_NOW` returns: a 50 ms deadline must not deliver
//!    early (the one property a timeout must have — a timer that can
//!    fire early makes every retransmission rule built on it wrong),
//!    must not be much late, a CANCELLED timer must stay silent, a
//!    second cancel of the same timer must be refused, and two due
//!    timers on one notification must both deliver so a service can
//!    wait for "work OR timeout" in a single blocking call.
//!
//!    The kernel then proves its own side: the facility counted the
//!    arms, fires and cancels the client claims, and a timer the
//!    client deliberately left armed is SWEPT when the process is
//!    destroyed — a dead process must not be able to keep signalling.
//!    Frame-exact teardown, as always.
//!
//! This suite needs no fixture and no host actor: it is about the
//! kernel's own clock, so it runs and must pass on the barest machine.
//!
//! Markers: `m7:test:<name>`, `m7: RESULT`.

use crate::arch::x86_64::{paging, syscall};
use crate::cap::{Cap, CapObj};
use crate::drivers::intc;
use crate::frames;
use crate::ipc;
use crate::log::{log_error as error, log_info as info, write_marker};
use crate::proc;
use crate::sched;
use crate::{cap, timer};

type Res = Result<(), &'static str>;

/// timertest's verified-success exit.
const TIMERTEST_EXIT_OK: u64 = 42;
const TIMERTEST_EXIT_BADGE: u64 = 0x0070_0001;

/// How long the suite waits for the client to finish its measurements
/// (it sleeps ~200 ms in total across five timers), in HPET ticks at
/// QEMU's 100 MHz: ~5 s, generous against a loaded host and still far
/// inside the boot timeout.
const CLIENT_DEADLINE_TICKS: u32 = 500_000_000;

pub fn run_suite() -> bool {
    let checks: [(&str, fn() -> Res); 1] = [("timer_facility", test_timer_facility)];
    let mut passed = 0u32;
    for (name, test) in checks {
        match test() {
            Ok(()) => {
                passed += 1;
                write_marker(format_args!("m7:test:{name}: PASS"));
            }
            Err(reason) => {
                error!("m7", "test {name} failed: {reason}");
                write_marker(format_args!("m7:test:{name}: FAIL ({reason})"));
            }
        }
    }
    let total = checks.len() as u32;
    write_marker(format_args!(
        "m7: RESULT {} ({passed}/{total})",
        if passed == total { "PASS" } else { "FAIL" }
    ));
    passed == total
}

fn test_timer_facility() -> Res {
    let baseline = frames::free_frames();
    let before = timer::stats();
    if before.armed_now != 0 {
        return Err("a timer was already armed before the test (nothing else should be)");
    }

    let nid = ipc::create_notification().map_err(|_| "notification table full")?;
    let nid_exit = ipc::create_notification().map_err(|_| "notification table full")?;

    // One grant: the notification it arms timers against and waits on.
    // READ|WRITE because it is both the notify side (arming) and the
    // wait side — and the WRITE right is exactly what `SYS_TIMER_ARM`
    // checks, so a process can only aim a timer at something it was
    // already trusted to signal. No new capability kind was needed to
    // make timers safe (ADR-0029).
    let grants = [Cap {
        obj: CapObj::Notification { nid },
        rights: cap::RIGHTS_READ | cap::RIGHTS_WRITE,
    }];
    let pid = crate::spawn::spawn_init(16, &grants, Some((nid_exit, TIMERTEST_EXIT_BADGE)))
        .map_err(|_| "timertest (image 16) spawn failed")?;
    info!(
        "m7",
        "timer_facility: timertest pid {pid} (notification {nid}) — measuring real deadlines against the monotonic clock"
    );

    // The client sleeps on real timers, so this drain is deliberately
    // patient. It is NOT a poll of the client: the boot thread yields,
    // the client is blocked in SYS_WAIT, and the tick is what wakes it.
    if let Err(reason) = drain_pid(pid) {
        error!(
            "m7",
            "timer_facility: client threads={}, timers armed={}",
            sched::proc_live_threads(pid),
            timer::stats().armed_now
        );
        return Err(reason);
    }

    let badge = ipc::wait(nid_exit).map_err(|_| "the client's exit badge never arrived")?;
    if badge != TIMERTEST_EXIT_BADGE {
        return Err("the client's exit badge is not the granted word");
    }
    let recs = crate::spawn::records_snapshot();
    let Some(tid) = recs
        .iter()
        .flatten()
        .find(|&&(p, _)| p == pid)
        .map(|&(_, t)| t)
    else {
        return Err("the client has no spawn record");
    };
    match syscall::exit_status_of(tid) {
        Some(TIMERTEST_EXIT_OK) => {}
        Some(60) => return Err("client: SYS_CLOCK_NOW read as zero or negative (60)"),
        Some(61) => return Err("client: arming a timer or waiting on it was refused (61)"),
        Some(62) => return Err("client: a deadline fired EARLY (62)"),
        Some(63) => return Err("client: a deadline fired far later than the granularity (63)"),
        Some(64) => return Err("client: a timer delivered the wrong badge (64)"),
        Some(65) => return Err("client: cancelling an armed timer was refused (65)"),
        Some(66) => {
            return Err("client: cancelling an already-cancelled timer reported success (66)");
        }
        Some(67) => return Err("client: a CANCELLED timer delivered its badge anyway (67)"),
        Some(68) => return Err("client: two due timers did not both deliver (68)"),
        Some(69) => {
            return Err(
                "client: a STALE timer id was accepted — it could have cancelled a stranger's timer (69)",
            );
        }
        Some(99) => return Err("client: the panic handler ran (99)"),
        _ => return Err("the client exited with a code from nowhere in the contract"),
    }

    // The kernel's own accounting of what the client just did.
    let after = timer::stats();
    if after.armed_total < before.armed_total + 8 {
        error!(
            "m7",
            "timer_facility: armed_total {} → {}, expected at least 8 more",
            before.armed_total,
            after.armed_total
        );
        return Err("the kernel counted fewer arms than the client made");
    }
    if after.fired < before.fired + 6 {
        return Err("the kernel counted fewer firings than the client observed");
    }
    if after.cancelled != before.cancelled + 1 {
        return Err("the kernel did not count exactly one cancellation");
    }
    // The client left one armed ON PURPOSE. It must still be armed —
    // and then swept, not left signalling a dead process's notification.
    if after.armed_now != 1 {
        error!(
            "m7",
            "timer_facility: {} timers armed at exit, expected the 1 the client left",
            after.armed_now
        );
        return Err("the client's deliberately-abandoned timer is not there to sweep");
    }

    proc::destroy(pid).map_err(|_| "destroying the client failed")?;
    let swept = timer::stats();
    if swept.armed_now != 0 || swept.swept != after.swept + 1 {
        error!(
            "m7",
            "timer_facility: after destroy {} armed, swept {} → {}",
            swept.armed_now,
            after.swept,
            swept.swept
        );
        return Err("destroying the owner did not sweep its armed timer");
    }

    crate::spawn::forget(pid).map_err(|_| "client spawn record forget refused")?;
    ipc::destroy_notification(nid).map_err(|_| "notification teardown refused")?;
    ipc::destroy_notification(nid_exit).map_err(|_| "exit notification teardown refused")?;
    for _ in 0..4 {
        sched::yield_now();
    }
    let frames_after = frames::free_frames();
    if frames_after != baseline {
        error!(
            "m7",
            "timer_facility teardown accounting: baseline {baseline}, after {frames_after}"
        );
        return Err("timer-facility teardown is not frame-exact");
    }
    info!(
        "m7",
        "timer_facility: timertest (pid {pid}) armed {} timers and measured them against SYS_CLOCK_NOW — no deadline fired early, none lagged beyond one tick plus slack, a cancelled timer stayed silent, a second cancel was refused, two due timers merged into one wake; the kernel counted {} firing(s) and {} cancellation(s), swept the timer the client abandoned, and teardown is frame-exact (frames {frames_after})",
        after.armed_total - before.armed_total,
        after.fired - before.fired,
        after.cancelled - before.cancelled
    );
    Ok(())
}

/// Yield until `pid` has no live threads, bounded by the HPET wall
/// clock. The client spends most of its life BLOCKED on timers, so
/// this is a patient drain, not a poll of anything.
fn drain_pid(pid: u64) -> Res {
    let if_before = crate::arch::x86_64::interrupts_enabled();
    crate::arch::x86_64::sti();
    let hpet_va = paging::mmio_alias_va(intc::HPET_PHYS);
    // SAFETY: ring 0; mapped HPET alias; 32-bit main-counter read.
    let t0 = unsafe { intc::hpet_read(hpet_va, intc::HPET_MAIN_COUNTER) };
    let mut out = Ok(());
    while sched::proc_live_threads(pid) > 0 {
        // SAFETY: the same alias and register.
        let now = unsafe { intc::hpet_read(hpet_va, intc::HPET_MAIN_COUNTER) };
        if now.wrapping_sub(t0) >= CLIENT_DEADLINE_TICKS {
            out = Err("the timer client is still live after the wall-clock deadline");
            break;
        }
        sched::yield_now();
    }
    sched::yield_now();
    if !if_before {
        crate::arch::x86_64::cli();
    }
    out
}
