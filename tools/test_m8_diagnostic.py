#!/usr/bin/env python3
"""ADR-0047: destructive opcodes require service-verified transferred caps."""
from pathlib import Path
import sys
sys.path.insert(0, str(Path(__file__).resolve().parent))
import arena_env  # noqa: E402
import mtest  # noqa: E402

READY = b"servicemgr: production netstackd READY pid"
LABEL = "m8-diagnostic"


def check(good: bool, why: str) -> bool:
    print(f"[{LABEL}] {'PASS' if good else 'FAIL'}: {why}")
    return good


def main() -> int:
    esp = mtest.build(LABEL)
    ok = True
    for name, command, markers in (
        ("orderly", b"stacktest\r", [
            "m8: stacktest service refused missing/wrong shutdown marker",
            "m8: stacktest PASS (same endpoint; old bearer revoked; fresh ARP request on real wire)"]),
        ("fault", b"stackfault\r", [
            "m8: stackfault service refused missing/wrong diagnostic marker, still live",
            "netstackd: injecting real #UD before in-flight reply",
            "m8: stackfault PASS (real #UD, in-flight call failed, same endpoint fresh wire, resources flat)"]),
        ("rng-fault", b"depdeny\r", [
            "depcheck: rngd refused absent and wrong-object fault marker",
            "depcheck: private failure fixture armed on rngd",
            "rngd: injecting genuine #UD with probe GET in flight",
            "servicemgr: OFFLINE — restart authority/dependency plan refused"]),
        ("rng-stall", b"depstall\r", [
            "depcheck: rngd refused absent and wrong-object stall marker",
            "depcheck: private stall fixture armed on rngd",
            "rngd: intentionally withholding GET completion in destructive test VM",
            "servicemgr: dependency probe DEADLINE; blocked worker stopped, no child launched"]),
    ):
        rc, serial, dt = mtest.run_qemu(f"{LABEL}-{name}", esp,
            feed=[(READY, 1, command),
                  ("servicemgr: OFFLINE — restart authority/dependency plan refused".encode()
                   if name.startswith("rng-") else b"arena>",
                   1 if name.startswith("rng-") else 2, b"shutdown\r")], tcp_peer=False)
        (arena_env.build_dir() / f"serial-{LABEL}-{name}.log").write_text(serial)
        print(f"[{LABEL}] {name}: rc={rc}, {dt:.1f}s")
        positions = [serial.find(x) for x in markers]
        ok &= check(rc == 0 and "PANIC" not in serial
                    and all(p >= 0 for p in positions)
                    and positions == sorted(positions)
                    and "m8: RESULT PASS" not in serial,
                    f"{name}: receiving service refused missing/wrong marker; real proof still works")
        for legacy in ("blktest: missing diagnostic authority refused by storaged",
                       "fstest: fsd and storaged rejected missing/wrong-object poison markers",
                       "netdtest: missing diagnostic authority refused by service",
                       "rngdtest: missing diagnostic authority refused by service",
                       "arptest: stack refused shutdown without diagnostic marker",
                       "depcheck: production driver poison opcodes refused without marker"):
            ok &= check(legacy in serial, f"{name}: historical test service proved {legacy}")
        if name.startswith("rng-"):
            ok &= check(serial.count(READY.decode()) == 1,
                        f"{name}: failed probe did not create a production replacement")
    print(f"[{LABEL}] SERVICE-SIDE DIAGNOSTIC AUTHORITY: {'PASS' if ok else 'FAIL'}")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
