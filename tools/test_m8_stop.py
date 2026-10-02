#!/usr/bin/env python3
"""ADR-0043: manager-owned live Process-cap stop of production netstackd.

The Power-holding admin issues a private request, NEVER a Process cap.
A forged shared wake is refused independently; a true mode-1 kernel
finish observes a live child and fails an in-flight call, then the
bounded manager replaces it on the same endpoint. This test alone does not close 8.0.
"""
import re
import sys
from pathlib import Path
sys.path.insert(0, str(Path(__file__).resolve().parent))
import arena_env  # noqa: E402
import mtest  # noqa: E402

LABEL = "m8-stop"
READY = "servicemgr: production netstackd READY pid"
SUCCESS = "m8: stackstop PASS (manager mode-1 stopped live production child, new wire, resources flat)"


def check(ok: bool, what: str) -> bool:
    print(f"[{LABEL}] {'PASS' if ok else 'FAIL'}: {what}")
    return ok


def main() -> int:
    esp = mtest.build(LABEL)
    # READY and the prompt do not imply the concurrent permission app has
    # exited. Its manager-owned Process-cap reap is the resource baseline.
    feed = [((READY.encode(), b"arena>", b"permission app reaped through held Process cap"),
             1, b"stackstop\r"),
            (b"arena>", 2, b"shutdown\r")]
    rc, serial, dt = mtest.run_qemu(LABEL, esp, feed=feed)
    (arena_env.build_dir() / f"serial-{LABEL}.log").write_text(serial)
    print(f"[{LABEL}] full fixture: rc={rc}, {dt:.1f}s")
    ok = True
    pids = re.findall(rf"{READY} (\d+)", serial)
    audited = re.findall(r"manager-owned netstackd pid (\d+): five inherited child caps audited", serial)
    manager = re.search(r"servicemgr spawned: pid (\d+), image19", serial)
    mode1 = re.findall(r"proc_finish mode1: owner (\d+) target (\d+) live_threads=(\d+) held Process/DESTROY", serial)
    ok &= check(rc == 0 and "PANIC" not in serial and "m7: RESULT PASS (2/2)" in serial,
                "old suites remain green; clean QEMU halt")
    ok &= check(len(pids) == len(set(pids)) == 2 and audited == pids,
                "two distinct manager-owned cap-audited production children")
    ok &= check(manager is not None and len(mode1) == 1 and len(pids) == 2
                and mode1[0][:2] == (manager.group(1), pids[0])
                and int(mode1[0][2]) > 0,
                "kernel accepted exactly one mode-1 stop of an actually LIVE child by its manager")
    markers = ["servicemgr: ignored unauthenticated shared wake hint",
               "m8: stackstop forged shared wake alone did NOT stop live child",
               "servicemgr: refused unknown private admin request",
               "m8: stackstop malformed private request did NOT stop live child",
               "m8: stackstop sent private STOP then shared wake, no Process cap delegated",
               f"proc_finish mode1: owner {manager.group(1)} target {pids[0]}" if manager and pids else "no cap",
               f"destroy pid {pids[0]}: 1 in-flight call(s) answered STATUS_SERVICE_GONE" if pids else "no child",
               "servicemgr: forcibly stopped LIVE production child through held Process cap",
               "servicemgr: production child reaped through Process cap; bounded backoff",
               "servicemgr: restarted production netstackd on original endpoint",
               SUCCESS]
    positions = [serial.find(m) for m in markers]
    ok &= check(all(p >= 0 for p in positions) and positions == sorted(positions),
                "forged hint refused; private authority, live teardown, IPC failure and bounded restart in order")
    after = serial.split("servicemgr: forcibly stopped LIVE production child", 1)[-1]
    # The shell echoes the standalone typed command. Another process may
    # log between its prompt and that echo: never require adjacent bytes.
    # Anchor after the echo to exclude earlier M7 shutdowns.
    command = re.search(r"(?m)^(?:arena> )?stackstop\r?$", serial)
    production = serial[command.end():] if command else ""
    ok &= check(command is not None and after.count("netstackd: resolved 10.0.2.2") == 1
                and "netstackd: shutdown" not in production and "[arena user fault]" not in production
                and SUCCESS in after,
                "not orderly exit or CPU crash: forced stop, stale bearer rejected, fresh real wire and flat accounting")
    ok &= check("m8: stackstop FAIL" not in serial and "servicemgr: OFFLINE" not in serial
                and "m8: RESULT PASS" not in serial,
                "no hidden failure or premature 8.0 completion")
    absent = [(b"arena>", 1, b"stackstop\r"), (b"arena>", 2, b"shutdown\r")]
    rc, missing, dt = mtest.run_qemu(f"{LABEL}-absent", esp, feed=absent,
                                     net=False, rng=False, kbd=False, vcon=False,
                                     tcp_peer=False)
    (arena_env.build_dir() / f"serial-{LABEL}-absent.log").write_text(missing)
    print(f"[{LABEL}] no-device fixture: rc={rc}, {dt:.1f}s")
    ok &= check(rc == 0 and "m8: stackstop SKIP (no production client cap)" in missing
                and "servicemgr: OFFLINE" in missing and "proc_finish mode1:" not in missing,
                "missing dependencies grant neither admin stop nor client authority")
    print(f"[{LABEL}] FORCED-LIVE-STOP SUBSTRATE: {'PASS' if ok else 'FAIL'} (individual proof; full suite closes 8.0)")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
