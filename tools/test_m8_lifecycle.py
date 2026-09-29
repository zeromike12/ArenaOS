#!/usr/bin/env python3
"""ADR-0044: real Process-cap lifecycle refusals and positive user-child reap.

Cross-check the five cap descriptions printed by ring 3 against independently
logged boot-root and manager-child pids. Refusal alone is insufficient: the
manager/driver/child must survive and serve actual post-refusal wire traffic.
"""
import re
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import arena_env  # noqa: E402
import mtest  # noqa: E402

LABEL = "m8-lifecycle"
PASS = "m8: lifetest PASS (protected/foreign/forged/stale denied; own child reaped; wire live; resources flat)"
FOREIGN = "lifecycle read-only foreign Process reference installed pid"


def check(ok: bool, what: str) -> bool:
    print(f"[{LABEL}] {'PASS' if ok else 'FAIL'}: {what}")
    return ok


def one(pattern: str, serial: str) -> re.Match[str] | None:
    matches = list(re.finditer(pattern, serial))
    return matches[0] if len(matches) == 1 else None


def main() -> int:
    esp = mtest.build(LABEL)
    # This is later than the shell's first prompt and the independent
    # manager-child cap audit. A feeder keyed to "m8: ..." would never
    # fire: kernel log lines are prefixed "[arena INFO  m8] ...".
    feed = [(FOREIGN.encode(), 1, b"lifetest\r"),
            (b"arena>", 2, b"shutdown\r")]
    rc, serial, dt = mtest.run_qemu(LABEL, esp, feed=feed)
    (arena_env.build_dir() / f"serial-{LABEL}.log").write_text(serial)
    print(f"[{LABEL}] full fixture: rc={rc}, {dt:.1f}s")
    ok = True
    root = one(r"lifecycle protected refs shell=(\d+) manager=(\d+) netd=(\d+) rngd=(\d+); foreign placeholder READ-only", serial)
    refs = one(r"m8: lifetest refs shell=(\d+) manager=(\d+) netd=(\d+) rngd=(\d+) foreign=(\d+)", serial)
    shell = one(r"shell spawned: pid (\d+)", serial)
    manager = one(r"servicemgr spawned: pid (\d+), image19", serial)
    netd = one(r"netd spawned: pid (\d+)", serial)
    rngd = one(r"rngd spawned: pid (\d+)", serial)
    child = one(r"manager-owned netstackd pid (\d+): five inherited child caps audited", serial)
    foreign = one(r"lifecycle read-only foreign Process reference installed pid (\d+) slot 14", serial)
    ready = one(r"servicemgr: production netstackd READY pid (\d+)", serial)
    expected = (shell, manager, netd, rngd, child)
    ok &= check(all(expected) and root is not None and refs is not None
                and foreign is not None and ready is not None
                and root.groups() == tuple(m.group(1) for m in expected[:4])
                and refs.groups() == tuple(m.group(1) for m in expected)
                and child.group(1) == foreign.group(1) == ready.group(1)
                and len(set(refs.groups())) == 5,
                "ring-3 described five distinct REAL boot/audited targets, not pid guesses or another fixture")
    ordered = ("m8: lifetest refs shell=",
               "m8: lifetest held DESTROY refused for self/manager/netd/rngd",
               "m8: lifetest foreign READ-only/guessed pid/empty/wrong-kind refused",
               "m8: lifetest child reaped by held cap; dead mode-1 and stale both refused",
               PASS)
    places = [serial.find(s) for s in ordered]
    ok &= check(rc == 0 and all(i >= 0 for i in places)
                and places == sorted(places) and serial.count(PASS) == 1
                and "m8: lifetest FAIL" not in serial and "PANIC" not in serial,
                "two finish modes denied for protected/foreign/forged/stale, own child finished; clean halt")
    tail = serial[places[0]:] if places[0] >= 0 else ""
    ok &= check("netstackd: resolved 10.0.2.3" in tail
                and "netstackd: shutdown" not in tail
                and "servicemgr: OFFLINE" not in tail
                and "restarted production netstackd" not in tail
                and "[arena user fault]" not in tail
                and "proc_finish mode1: owner" not in tail
                and "m7: RESULT PASS (2/2)" in serial
                and "m8: RESULT PASS" not in serial,
                "surviving production child served new post-refusal wire; no stop, restart, fault or premature 8.0 closure")
    for tag, net, rng in (("no-devices", False, False),
                          ("rng-only", False, True)):
        rc, missing, dt = mtest.run_qemu(
            f"{LABEL}-{tag}", esp,
            feed=[(b"arena>", 1, b"lifetest\r"),
                  (b"arena>", 2, b"shutdown\r")],
            net=net, rng=rng, kbd=False, vcon=False, tcp_peer=False)
        (arena_env.build_dir() / f"serial-{LABEL}-{tag}.log").write_text(missing)
        print(f"[{LABEL}] {tag}: rc={rc}, {dt:.1f}s")
        ok &= check(rc == 0 and "m8: lifetest SKIP (no production client cap)" in missing
                    and "lifecycle protected refs" not in missing
                    and FOREIGN not in missing
                    and "servicemgr: OFFLINE" in missing
                    and "m8: lifetest PASS" not in missing
                    and "PANIC" not in missing,
                    f"{tag}: honest SKIP with no diagnostic grants or manager-owned child")
    print(f"[{LABEL}] LIFECYCLE-AUTHORITY REFUSAL SUBSTRATE: {'PASS' if ok else 'FAIL'} (individual proof; full suite closes 8.0)")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
