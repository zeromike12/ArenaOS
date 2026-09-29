#!/usr/bin/env python3
"""ADR-0041: real, opt-in, bounded manager stress on production processes.

The first three exits return behind the same endpoint with byte-exact
kernel frame/record/process accounting. The fourth exhausts the fixed
budget and leaves OFFLINE. Ordinary boots are not deliberately killed.
This is not a crash or forged-Process-cap proof, and not 8.0 completion.
"""
import re
import sys
from pathlib import Path
sys.path.insert(0, str(Path(__file__).resolve().parent))
import arena_env  # noqa: E402
import mtest  # noqa: E402

LABEL = "m8-stress"
READY = "servicemgr: production netstackd READY pid"
OFFLINE = "servicemgr: OFFLINE — bounded restart budget exhausted"
PASS = "m8: stackstress PASS (3 real-wire restarts, exact frames/records/processes)"


def check(ok: bool, what: str) -> bool:
    print(f"[{LABEL}] {'PASS' if ok else 'FAIL'}: {what}")
    return ok


def main() -> int:
    esp = mtest.build(LABEL)
    feed = [(READY.encode(), 1, b"stackstress\r"),
            (OFFLINE.encode(), 1, b"shutdown\r")]
    rc, serial, dt = mtest.run_qemu(LABEL, esp, feed=feed)
    (arena_env.build_dir() / f"serial-{LABEL}.log").write_text(serial)
    print(f"[{LABEL}] full fixture: rc={rc}, {dt:.1f}s")
    ok = True
    ok &= check(rc == 0 and "PANIC" not in serial and "m7: RESULT PASS (2/2)" in serial,
                "M1–M7 regressions still green and clean QEMU shutdown")
    baseline = re.findall(r"m8: stackstress baseline frames=(\d+) records=(\d+) processes=(\d+)", serial)
    cycles = re.findall(r"m8: stackstress cycle (\d+) frames=(\d+) records=(\d+) processes=(\d+)", serial)
    ok &= check(len(baseline) == 1 and all(int(v) > 0 for v in baseline[0])
                and len(cycles) == 3 and [n for n, *_ in cycles] == ["1", "2", "3"]
                and all(tuple(values) == baseline[0] for _, *values in cycles) and PASS in serial,
                "actual kernel free frames/spawn records/process slots flat after EACH of three restarts")
    pids = re.findall(rf"{READY} (\d+)", serial)
    audited = re.findall(r"manager-owned netstackd pid (\d+): four installed child caps audited", serial)
    ok &= check(len(pids) == len(set(pids)) == 4 and audited == pids,
                "four distinct production child instances, every installed cap audited")
    ok &= check(serial.count("servicemgr: production child reaped through Process cap") == 3
                and serial.count("servicemgr: restarted production netstackd on original endpoint") == 3
                and serial.count("m8: stacktest PASS (same endpoint; old bearer revoked; fresh ARP request on real wire)") == 3,
                "three real-wire, bearer-revoking Process-cap reap/backoff/restart cycles")
    ok &= check("m8: stackstress requested fourth exit; budget must leave service OFFLINE" in serial
                and OFFLINE in serial and serial.find(OFFLINE) > serial.find(PASS)
                and serial.count(READY) == 4,
                "fourth exit exhausted fixed budget, no fifth child invented")
    ok &= check("m8: stackstress FAIL" not in serial and "m8: stacktest FAIL" not in serial
                and "m8: RESULT PASS" not in serial,
                "no hidden failure or dishonest 8.0 completion marker")

    absent_feed = [(b"arena>", 1, b"stackstress\r"), (b"arena>", 2, b"shutdown\r")]
    rc, absent, dt = mtest.run_qemu(f"{LABEL}-absent", esp, feed=absent_feed,
                                    net=False, rng=False, kbd=False, vcon=False,
                                    tcp_peer=False)
    (arena_env.build_dir() / f"serial-{LABEL}-absent.log").write_text(absent)
    print(f"[{LABEL}] no-network fixture: rc={rc}, {dt:.1f}s")
    ok &= check(rc == 0 and "m8: stackstress SKIP (no production client cap)" in absent
                and "servicemgr: OFFLINE" in absent and PASS not in absent,
                "missing dependencies grant no client authority; explicit SKIP")
    print(f"[{LABEL}] REPEATED-ACCOUNTING SUBSTRATE: {'PASS' if ok else 'FAIL'} (Phase 8.0 NOT complete)")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
