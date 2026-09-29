#!/usr/bin/env python3
"""ADR-0045: actual pre-spawn netd/rngd protocol probes, bounded negatives.

Two real IPCs including a 64-byte device entropy DMA finish *before*
each production spawn. Failure and a hung driver are opt-in destructive
VMs, not part of unattended production boots. No pid or badge is authority.
"""
import re
import sys
from pathlib import Path
sys.path.insert(0, str(Path(__file__).resolve().parent))
import arena_env  # noqa: E402
import mtest  # noqa: E402

LABEL = "m8-dependencies"
READY = "servicemgr: production netstackd READY pid"
PROBE = "servicemgr: active netd MAC and rngd entropy probes passed; worker reaped"
AUDIT = r"manager-owned dependency probe pid (\d+): netd/W rngd/W private-notification/W audited, no privileged extras"
OFFLINE = "servicemgr: OFFLINE — restart authority/dependency plan refused"


def check(ok: bool, what: str) -> bool:
    print(f"[{LABEL}] {'PASS' if ok else 'FAIL'}: {what}")
    return ok


def boot(esp: Path, tag: str, feed, **opts):
    rc, s, dt = mtest.run_qemu(f"{LABEL}-{tag}", esp, feed=feed, tcp_peer=False, **opts)
    (arena_env.build_dir() / f"serial-{LABEL}-{tag}.log").write_text(s)
    print(f"[{LABEL}] {tag}: rc={rc}, {dt:.1f}s")
    return rc, s, dt


def main() -> int:
    esp = mtest.build(LABEL)
    ok = True
    rc, s, dt = boot(esp, "restart", [(READY.encode(), 1, b"stackstop\r"),
                                      (b"arena>", 2, b"shutdown\r")])
    pids = re.findall(rf"{READY} (\d+)", s)
    workers = re.findall(AUDIT, s)
    worker_ops = [m.start() for m in re.finditer("depcheck: rngd device completed 64 varied bytes", s)]
    probe_done = [m.start() for m in re.finditer(PROBE, s)]
    production = [m.start() for m in re.finditer(READY, s)]
    ok &= check(rc == 0 and "PANIC" not in s and "servicemgr: OFFLINE" not in s
                and "m8: stackstop PASS" in s and "m7: RESULT PASS (2/2)" in s,
                "full fixture, historical suites and live-stop recovery remain green")
    ok &= check(len(pids) == len(set(pids)) == 2 and len(workers) == len(set(workers)) == 2
                and not set(workers) & set(pids)
                and len(worker_ops) == len(probe_done) == len(production) == 2
                and all(a < b < c for a, b, c in zip(worker_ops, probe_done, production))
                and s.count("depcheck: netd MAC answered") == 2
                and s.count("manager-owned netstackd pid") == 2,
                "two independently cap-audited worker reaps with real MAC/entropy BEFORE two audited production pids")
    ok &= check("m8: stackstop PASS" in s and "m8: RESULT PASS" not in s,
                "same endpoint, new real wire and flat resource counts without a false boot-level proof")

    for tag, command, mode, evidence in (
        ("fault", b"depdeny\r", "1", "servicemgr: dependency probe FAILED; worker reaped, no child launched"),
        ("stall", b"depstall\r", "2", "servicemgr: dependency probe DEADLINE; blocked worker stopped, no child launched"),
    ):
        rc, neg, dt = boot(esp, tag, [(READY.encode(), 1, command),
                                     (OFFLINE.encode(), 1, b"shutdown\r")])
        p = re.findall(rf"{READY} (\d+)", neg)
        probes = re.findall(r"manager-owned dependency probe pid (\d+)", neg)
        points = [neg.find(x) for x in (
            f"manager-owned dependency probe pid {probes[-1]}: diagnostic mode {mode}" if probes else "MISSING",
            f"depcheck: private {'failure' if tag == 'fault' else 'stall'} fixture armed on rngd",
            "rngd: injecting genuine #UD with probe GET in flight" if tag == "fault"
            else "rngd: intentionally withholding GET completion in destructive test VM",
            evidence, OFFLINE)]
        ok &= check(rc == 0 and len(p) == 1 and len(probes) == 2
                    and probes[0] != probes[1] and all(i >= 0 for i in points)
                    and points == sorted(points) and "m8: stackstop PASS" not in neg
                    and "PANIC fault" not in neg and "servicemgr: restarted production netstackd" not in neg,
                    f"{tag}: real dependency refused before a replacement; bounded manager OFFLINE")
        if tag == "fault":
            ok &= check(bool(re.search(rf"\[arena user fault\] pid=(\d+) vector=0x06", neg))
                        and "1 in-flight call(s) answered STATUS_SERVICE_GONE" in neg
                        and "rngd: faulted driver pid" in neg
                        and "rngd: RESTARTED as pid" in neg,
                        "kernel deferred supervised-fault teardown, failed IPC and restarted real driver")
        else:
            ok &= check(bool(probes) and re.search(
                rf"proc_finish mode1: owner \d+ target {probes[-1]} live_threads=1 held Process/DESTROY", neg) is not None
                and "[arena user fault]" not in neg,
                "stalled IPC could not hang manager; Process-cap stop reaped worker after deadline")
    for tag, net, rng in (("no-devices", False, False), ("rng-only", False, True)):
        rc, absent, dt = boot(esp, tag, [(b"arena>", 1, b"depdeny\r"),
                                          (b"arena>", 2, b"shutdown\r")],
                              net=net, rng=rng, kbd=False, vcon=False)
        ok &= check(rc == 0 and "m8: dependency probe SKIP (no production client cap)" in absent
                    and "servicemgr: OFFLINE" in absent and "manager-owned dependency probe" not in absent
                    and READY not in absent and "PANIC" not in absent,
                    f"{tag}: absent dependency grants neither probe image nor partial production authority")
    print(f"[{LABEL}] ACTIVE DEPENDENCY PROBES: {'PASS' if ok else 'FAIL'}")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
