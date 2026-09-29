#!/usr/bin/env python3
"""ADR-0042: genuine production #UD, outstanding IPC failure, bounded recovery.

Opt-in privileged serial command only; an ordinary boot stays resident.
No claim of forced-stop, lifecycle negative-space, or active dependency
probing: those remain required for 8.0 completion.
"""
import re
import sys
from pathlib import Path
sys.path.insert(0, str(Path(__file__).resolve().parent))
import arena_env  # noqa: E402
import mtest  # noqa: E402

LABEL = "m8-fault"
PASS = "m8: stackfault PASS (real #UD, in-flight call failed, same endpoint fresh wire, resources flat)"
READY = "servicemgr: production netstackd READY pid"


def check(ok: bool, what: str) -> bool:
    print(f"[{LABEL}] {'PASS' if ok else 'FAIL'}: {what}")
    return ok


def main() -> int:
    esp = mtest.build(LABEL)
    feed = [(READY.encode(), 1, b"stackfault\r"), (b"arena>", 2, b"shutdown\r")]
    rc, serial, dt = mtest.run_qemu(LABEL, esp, feed=feed)
    (arena_env.build_dir() / f"serial-{LABEL}.log").write_text(serial)
    print(f"[{LABEL}] production crash fixture: rc={rc}, {dt:.1f}s")
    ok = True
    pids = re.findall(rf"{READY} (\d+)", serial)
    audits = re.findall(r"manager-owned netstackd pid (\d+): five inherited child caps audited", serial)
    ok &= check(rc == 0 and "PANIC" not in serial and "m7: RESULT PASS (2/2)" in serial,
                "historical suite green; no whole-kernel panic on ring-3 fault")
    ok &= check(len(pids) == len(set(pids)) == 2 and audits == pids,
                "two distinct manager-owned production children audited")
    markers = ["m8: stackfault issued real-wire request and held bearer before fault",
               f"[arena user fault] pid={pids[0]} vector=0x06 (#UD Invalid Opcode)" if pids else "NO FIRST PID",
               f"destroy pid {pids[0]}: 1 in-flight call(s) answered STATUS_SERVICE_GONE" if pids else "NO FIRST PID",
               "servicemgr: production child reaped through Process cap; bounded backoff",
               "m8: stackfault in-flight call answered SERVICE_GONE on actual #UD",
               "servicemgr: restarted production netstackd on original endpoint",
               PASS]
    positions = [serial.find(m) for m in markers]
    ok &= check(all(p >= 0 for p in positions) and positions == sorted(positions),
                "CPU #UD mid-call, kernel client sweep, manager Process-cap reap and restart in order")
    after = serial.split("m8: stackfault in-flight call answered SERVICE_GONE on actual #UD", 1)[-1]
    ok &= check(after.count("netstackd: resolved 10.0.2.2") == 1 and PASS in after,
                "same endpoint rejected stale bearer; new child made new ARP wire request; accounting flat")
    ok &= check("m8: stackfault FAIL" not in serial and "servicemgr: OFFLINE" not in serial
                and "m8: RESULT PASS" not in serial,
                "no hidden failure, no invented Phase 8.0 completion")
    absent = [(b"arena>", 1, b"stackfault\r"), (b"arena>", 2, b"shutdown\r")]
    rc, missing, dt = mtest.run_qemu(f"{LABEL}-absent", esp, feed=absent,
                                     net=False, rng=False, kbd=False, vcon=False,
                                     tcp_peer=False)
    (arena_env.build_dir() / f"serial-{LABEL}-absent.log").write_text(missing)
    print(f"[{LABEL}] no-device fixture: rc={rc}, {dt:.1f}s")
    ok &= check(rc == 0 and "m8: stackfault SKIP (no production client cap)" in missing
                and "servicemgr: OFFLINE" in missing and "[arena user fault]" not in missing,
                "absent dependencies grant no fault-injection authority")
    print(f"[{LABEL}] UNEXPECTED-CRASH SUBSTRATE: {'PASS' if ok else 'FAIL'} (individual proof; full suite closes 8.0)")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
