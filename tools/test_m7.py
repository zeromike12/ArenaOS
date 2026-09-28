#!/usr/bin/env python3
"""Milestone 7 automated boot test (docs/TESTING.md, ADR-0029).

Phase 7 opens with the facility every protocol in it depends on, and
with the rule that makes it necessary: NO polling loops and NO
busy-waits anywhere in the stack. A timeout has to be a thing the
kernel delivers, so the first thing built is timers and the first
thing proven is that they keep time.

Pipeline and verdict logic live in tools/mtest.py, including the
cross-milestone regression guard — the same boot must still carry
PASSing m1..m6 RESULT lines.

Coverage:
  M7.0 — the timer facility (ADR-0029):
  * timer_facility — the kernel spawns timertest (registry image 16)
                     with ONE notification, and the image arms real
                     timers against it, measuring each against the
                     monotonic clock SYS_CLOCK_NOW returns:

                       - a 50 ms deadline never delivers EARLY (the
                         one property a timeout must have — a timer
                         that can fire early makes every
                         retransmission rule built on it wrong in a
                         way that only appears under load),
                       - and lands within one tick plus slack,
                       - a CANCELLED timer stays silent,
                       - cancelling it a second time is REFUSED (a
                         protocol cancelling a retransmission that
                         already went out must be able to tell),
                       - two due timers on one notification both
                         deliver, which is what lets a service wait
                         for "work OR timeout" in one blocking call,
                       - a STALE id (whose slot has since been handed
                         to a later timer) is REFUSED, and the timer
                         it aliases still fires. Timer ids carry a
                         generation for exactly this: a TCP stack
                         holds dozens of retransmission timers in one
                         process, so an owner check is no protection,
                         and replaying stale bookkeeping would kill a
                         stranger's timer silently.

                     The kernel then proves its own side: it counted
                     the arms, firings and cancellations the client
                     claims, and the timer the client deliberately
                     left armed is SWEPT when the process is
                     destroyed — a dead process must not keep
                     signalling. Frame-exact teardown.

This suite needs no device fixture and no host actor: it is about the
kernel's own clock, so it runs and must pass on the barest machine.

Exit status 0 = every check passed.
"""
import re
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import arena_env  # noqa: E402
import mtest  # noqa: E402

EXPECTED_TESTS = ["timer_facility"]


def extra_checks(serial: str) -> bool:
    """Evidence beyond the suite's own PASS marker."""
    ok = True

    def check(cond: bool, msg: str) -> None:
        nonlocal ok
        print(f"[test-m7] {'PASS' if cond else 'FAIL'}: {msg}")
        ok = ok and cond

    check("timer facility ready: 32 slots" in serial,
          "the kernel installed the timer facility before running anything "
          "that needs it")

    # The measurement itself, quoted back from the guest. This is the
    # line that makes the milestone a fact rather than a claim.
    m = re.search(r"timertest: PASS — 50000us timer delivered after (\d+)us "
                  r"\(never early, (\d+)us of lag", serial)
    check(m is not None,
          "the client measured a real 50 ms deadline against the monotonic "
          "clock")
    if m:
        elapsed, lag = int(m.group(1)), int(m.group(2))
        check(elapsed >= 50_000,
              f"the deadline was NOT early ({elapsed}us >= 50000us)")
        check(lag <= 60_000,
              f"the lag stayed inside one tick plus slack ({lag}us)")

    check("a cancelled timer stayed silent, and a second cancel was refused"
          in serial,
          "a cancelled timer never fired, and cancelling it twice was an "
          "error rather than a silent success")
    check("two timers on one notification both delivered" in serial,
          "badges merge — a service can wait for work OR a timeout in one "
          "blocking call")
    check("a stale id was refused and the live timer it aliased" in serial,
          "a STALE timer id was refused — ids carry a generation, so a "
          "reused slot cannot be cancelled by an old handle (C's review "
          "of v0.11.0)")
    check("swept the timer the client abandoned" in serial,
          "the kernel swept a timer whose owner died (the fourth thing "
          "proc::destroy sweeps, after relays, the console mirror and "
          "blocked-thread references)")
    check("teardown is frame-exact" in serial,
          "the whole cycle is frame-exact")

    # M7.0 also wires ADR-0028's supervisor into production. Its
    # absence of noise IS the evidence here: nothing died, so nothing
    # was restarted, and the idle loop said nothing about it.
    check("supervisor: restarted" not in serial,
          "the production supervisor ran quietly — nothing died, so nothing "
          "was restarted")

    # Phase 7's standing rule, checked the only way a log can: the
    # machine reached its prompt without a suite hanging on a clock.
    check("arena>" in serial,
          "the machine still reaches the shell with the timer facility live")
    return ok


def main() -> int:
    rc = mtest.run_milestone("m7", EXPECTED_TESTS)
    if rc != 0:
        return rc
    serial = (arena_env.build_dir() / "serial-m7.log").read_text()
    ok = extra_checks(serial)
    print(f"[test-m7] M7.0 TIMER FACILITY: {'PASS' if ok else 'FAIL'}  "
          f"(serial: build/serial-m7.log)")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
