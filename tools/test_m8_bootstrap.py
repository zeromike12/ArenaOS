#!/usr/bin/env python3
"""Phase 8.0: prove manager bootstrap/initial child, not restart.

An actual ring-3 process queries its own caps; real boot drivers signal
readiness after DRIVER_OK. Device-missing boots must not mint partial
service authority or claim a manager-owned service started. No m8 RESULT
is emitted until real restart and negative-space proofs are complete.
"""
import re
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import arena_env  # noqa: E402
import mtest  # noqa: E402

LABEL = "m8-bootstrap"
VALIDATED = "servicemgr: policy validated from live caps and ready drivers"
OFFLINE = "servicemgr: OFFLINE — missing or invalid boot grants; no child spawned"


def check(ok: bool, msg: str) -> bool:
    print(f"[{LABEL}] {'PASS' if ok else 'FAIL'}: {msg}")
    return ok


def boot(esp: Path, tag: str, *, net: bool, rng: bool) -> tuple[int, str]:
    arena_env.make_scratch_disk()
    # The shell can reach its prompt before the new child completes
    # setup. Wait for BOTH independent observations before shutdown;
    # otherwise a fast feeder tests scheduling luck, not readiness.
    feed = ([(b"servicemgr: production netstackd READY pid", 1, b""),
             *mtest.DEFAULT_FEED] if net and rng else mtest.DEFAULT_FEED)
    rc, serial, dt = mtest.run_qemu(f"{LABEL}-{tag}", esp, feed=feed,
                                  net=net, rng=rng, kbd=net and rng,
                                  vcon=net and rng, tcp_peer=False)
    (arena_env.build_dir() / f"serial-{LABEL}-{tag}.log").write_text(serial)
    print(f"[{LABEL}] {tag} boot: rc={rc}, {dt:.1f}s")
    return rc, serial


def main() -> int:
    esp = mtest.build(LABEL)
    ok = True
    rc, serial = boot(esp, "full", net=True, rng=True)
    ok &= check(rc == 0 and "PANIC" not in serial and "m7: RESULT PASS (2/2)" in serial,
                "full fixture booted with all historical M7 tests green")
    manager = re.search(r"servicemgr spawned: pid (\d+), image19; stack endpoint Some\((\d+)\); audited (\d+) literal caps; no device/Power/Process grants", serial)
    ok &= check(manager is not None and manager.group(3) == "17" if manager else False,
                "kernel installed and audited exactly seventeen bounded manager grants (12 historical + 5 permission sources)")
    netd = re.search(r"netd spawned: pid \d+ .*?1=Endpoint(\d+)/R.*?3=Notif(\d+)/W", serial)
    rngd = re.search(r"rngd spawned: pid \d+ .*?1=Endpoint(\d+)/R.*?3=Notif(\d+)/W", serial)
    observed = re.search(r"servicemgr: live caps image17 netd=(\d+) stack=(\d+) rngd=(\d+)", serial)
    matched = bool(manager and netd and rngd and observed
                   and observed.groups() == (netd.group(1), manager.group(2), rngd.group(1))
                   and netd.group(2) != rngd.group(2))
    ok &= check(matched, "ring-3 observed real netd/stack/rng endpoint IDs, matching kernel grants; drivers hold DIFFERENT readiness notifications")
    ok &= check("servicemgr: full fixture notification budget 15/15; sixteenth refused" in serial,
                "fifteenth notification is allocated and a sixteenth is refused at boot")
    ok &= check(VALIDATED in serial and OFFLINE not in serial,
                "two device-originated readiness badges preceded live-cap manifest resolution")
    ok &= check("servicemgr: PANIC" not in serial and "m8: RESULT PASS" not in serial,
                "no panic and no dishonest 8.0 completion marker")
    after = serial.split("servicemgr spawned: pid", 1)[-1]
    ready = re.search(r"servicemgr: production netstackd READY pid (\d+)", after)
    audited = re.search(r"manager-owned netstackd pid (\d+): five inherited child caps audited \(netd/W stack/R backoff/RW rngd/W diag/R\), IPC landings excluded", after)
    ok &= check(bool(ready and audited and ready.group(1) == audited.group(1)
                     and after.count("servicemgr: production netstackd READY pid") == 1),
                "one manager-owned PRODUCTION child booted, reported ready and passed independent kernel cap audit")
    ok &= check("restart proof still OPEN" not in after,
                "the initial child stayed resident during this test boot")

    for tag, net, rng in (("none", False, False), ("rng-only", False, True)):
        rc, serial = boot(esp, tag, net=net, rng=rng)
        ok &= check(rc == 0 and "PANIC" not in serial and "m7: RESULT SKIP" in serial,
                    f"{tag}: old suites retain honest SKIP and machine boots")
        ok &= check(re.search(r"servicemgr spawned: pid \d+, image19; stack endpoint None; audited 2 literal caps; no device/Power/Process grants", serial) is not None,
                    f"{tag}: actual cap table has exactly two root grants, no partial service authority")
        ok &= check(OFFLINE in serial and VALIDATED not in serial,
                    f"{tag}: manager reports OFFLINE instead of asserting fake readiness")

    print(f"[{LABEL}] INITIAL-START SUBSTRATE: {'PASS' if ok else 'FAIL'} (individual proof; full suite closes 8.0)")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
