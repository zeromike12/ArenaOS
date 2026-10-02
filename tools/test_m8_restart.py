#!/usr/bin/env python3
"""ADR-0040: exercise the PRODUCTION manager's one bounded restart.

No automatic fault on an ordinary boot. This host actor types the
privileged shell command AFTER the first stack-ready event, watches the
same client endpoint survive an orderly death/reap/backoff/respawn,
then shuts down at the second shell prompt. An absent device is SKIP,
not a fictitious stack restart. This is NOT an 8.0 RESULT.
"""
import re
import sys
from pathlib import Path
sys.path.insert(0, str(Path(__file__).resolve().parent))
import arena_env  # noqa: E402
import mtest  # noqa: E402

LABEL = "m8-restart"
SUCCESS = "m8: stacktest PASS (same endpoint; old bearer revoked; fresh ARP request on real wire)"


def check(ok: bool, text: str) -> bool:
    print(f"[{LABEL}] {'PASS' if ok else 'FAIL'}: {text}")
    return ok


def main() -> int:
    esp = mtest.build(LABEL)
    feed = [(b"servicemgr: production netstackd READY pid", 1, b"stacktest\r"),
            (b"arena>", 2, b"shutdown\r")]
    rc, serial, dt = mtest.run_qemu(LABEL, esp, feed=feed, tcp_peer=False)
    (arena_env.build_dir() / f"serial-{LABEL}.log").write_text(serial)
    print(f"[{LABEL}] full network+entropy boot: rc={rc}, {dt:.1f}s")
    ok = True
    ok &= check(rc == 0 and "PANIC" not in serial and "m7: RESULT PASS (2/2)" in serial,
                "old M1–M7 regressions green and clean QEMU halt")
    pids = re.findall(r"servicemgr: production netstackd READY pid (\d+)", serial)
    audited = re.findall(r"manager-owned netstackd pid (\d+): five inherited child caps audited", serial)
    ok &= check(len(pids) == 2 and pids[0] != pids[1] and audited == pids,
                "exactly one production restart, each child independently cap-audited")
    markers = ["m8: stacktest Process-cap stop refused to endpoint-only client",
               "m8: stacktest held old endpoint and issued rngd-backed bearer after real ARP wire work",
               "m8: stacktest requested production child exit",
               "servicemgr: production child reaped through Process cap; bounded backoff",
               "servicemgr: restarted production netstackd on original endpoint",
               SUCCESS]
    positions = [serial.find(m) for m in markers]
    ok &= check(all(p >= 0 for p in positions) and positions == sorted(positions),
                "real client/wire, orderly exit, Process-cap reap, backoff, restart, resumed wire in order")
    ok &= check("m8: stacktest observed SERVICE_GONE during child absence" in serial,
                "held endpoint got typed SERVICE_GONE before the replacement was ready")
    after = serial.split("m8: stacktest requested production child exit", 1)[-1]
    ok &= check(after.count("netstackd: resolved 10.0.2.2") == 1 and SUCCESS in after,
                "replacement sent a fresh ARP request and rejected the old bearer")
    ok &= check("servicemgr: OFFLINE" not in serial and "m8: RESULT PASS" not in serial,
                "no false 8.0 completion claim or offline failure")

    no_feed = [(b"arena>", 1, b"stacktest\r"), (b"arena>", 2, b"shutdown\r")]
    rc, absent, dt = mtest.run_qemu(f"{LABEL}-absent", esp, feed=no_feed,
                                    net=False, rng=False, kbd=False, vcon=False,
                                    tcp_peer=False)
    (arena_env.build_dir() / f"serial-{LABEL}-absent.log").write_text(absent)
    print(f"[{LABEL}] no-network boot: rc={rc}, {dt:.1f}s")
    ok &= check(rc == 0 and "m8: stacktest SKIP (no production client cap)" in absent
                and "servicemgr: OFFLINE" in absent and SUCCESS not in absent,
                "no device means no shell stack authority and an honest SKIP")
    print(f"[{LABEL}] PRODUCTION ORDERLY-RESTART SUBSTRATE: {'PASS' if ok else 'FAIL'} (individual proof; full suite closes 8.0)")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
