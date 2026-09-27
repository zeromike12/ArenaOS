#!/usr/bin/env python3
"""Milestone 6 automated boot test (docs/TESTING.md, ADR-0024).

Pipeline and verdict logic live in tools/mtest.py (including the
cross-milestone regression guard: the same boot must still carry
PASSing m1..m5 RESULT lines, and the harness attaches BOTH fixtures —
the virtio-blk scratch disk and, since M6.1, the slirp NIC:
arena_env.net_args).

Current coverage:
  M6.1 — the virtio-net link proof (ADR-0024):
  * net_service   — the kernel spawns netd (registry image 6, the
                    userspace virtio-net driver: Mmio-cap window
                    grant, endpoint serve side, one notification
                    carrying BOTH MSI-X relay badges) and nettest
                    (image 7, the client: endpoint call side). The
                    client fetches the device MAC through the
                    service (device-config read path), hand-builds a
                    42-byte ARP request for the slirp gateway
                    (who-has 10.0.2.2 tell 10.0.2.15), and SENDs it —
                    the frame is LENT through IPC and the device DMAs
                    the client's own page, chained behind netd's own
                    virtio header (zero-copy TX). slirp's reply
                    arrives on the receive queue as an MSI-X
                    interrupt (never a poll) and returns in the
                    reply's inline message; the client verifies
                    ethertype/opcode/sender-IP/target-MAC and the
                    sender-MAC==Ethernet-source consistency at their
                    exact wire offsets, then poisons the service.
                    The kernel proves the machine side: both exit
                    badges exact, both exit codes 42, exactly ONE
                    relay delivery per vector (48 RX + 49 TX), the
                    dead driver's relays swept by proc::destroy, and
                    frame-exact teardown.

  This script additionally proves BOTH sides of ADR-0024's
  compatibility window with a second boot:
  * the production netd spawns at boot when the fixture is attached
    (asserted on the with-net serial), and
  * a pre-v0.6.0-style invocation WITHOUT the virtio-net fixture
    stays bootable-green: the m6 suite reports an honest SKIP (never
    a fake PASS, never a FAIL), the kernel logs the network service
    offline, production netd is NOT spawned, and m1..m5 all stay
    green in the same boot.

Exit code: 0 = PASS, 1 = FAIL (with the serial tail printed for diagnosis).
"""

import re
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import mtest  # noqa: E402
import arena_env  # noqa: E402

EXPECTED_TESTS = ["net_service"]


def check_with_net_extras(serial: str) -> bool:
    """Extra assertions on the with-net boot beyond the m6 markers."""
    ok = True

    def check(cond: bool, msg: str) -> None:
        nonlocal ok
        print(f"[test-m6] {'PASS' if cond else 'FAIL'}: {msg}")
        ok = ok and cond

    check("nettest: PASS — the ARP round trip completed" in serial,
          "nettest verified the ARP reply and reported its PASS line")
    check("ARP reply verified: ethertype 0x0806, opcode 2, sender 10.0.2.2"
          in serial,
          "the reply's protocol fields were verified at their wire offsets")
    check("1 RX + 1 TX interrupt deliveries on relay vectors 48/49"
          in serial,
          "the kernel counted exactly one hardware delivery per relay "
          "vector (no polling)")
    check("teardown frame-exact" in serial,
          "the suite's teardown was frame-exact")
    check("netd spawned: pid" in serial,
          "the PRODUCTION netd spawned at boot with the fixture attached")
    check(serial.count("virtio-net ready") == 2,
          "netd reached DRIVER_OK twice (the suite's instance + the "
          "production service)")
    return ok


def boot_without_net() -> bool:
    """The compatibility-window boot (ADR-0024): NO virtio-net fixture.

    A pre-v0.6.0 QEMU invocation must stay bootable-green with an
    honest SKIP — the network service was new in v0.6.0, and users'
    saved commands predate it.
    """
    label = "test-m6-nonet"
    ok = True

    def check(cond: bool, msg: str) -> None:
        nonlocal ok
        print(f"[test-m6-nonet] {'PASS' if cond else 'FAIL'}: {msg}")
        ok = ok and cond

    try:
        esp = mtest.build(label)
        rc, serial, dt = mtest.run_qemu(label, esp, net=False)
    except Exception as e:  # noqa: BLE001 — any harness fault is a FAIL
        print(f"[test-m6-nonet] FAIL: the no-net boot crashed the harness: {e}")
        return False
    arena_env.build_dir().joinpath("serial-m6-nonet.log").write_text(serial)

    check("PANIC" not in serial, "no kernel panic without the net fixture")
    check(re.search(r"^m6:test:net_service: SKIP \(no virtio-net device",
                    serial, re.MULTILINE) is not None,
          "the m6 suite reported an HONEST SKIP (not PASS, not FAIL)")
    check(re.search(r"^m6: RESULT SKIP ", serial, re.MULTILINE) is not None,
          "the m6 RESULT line says SKIP (a skip is never laundered "
          "into a pass count)")
    check("network service stays offline" in serial,
          "the kernel logged the network service offline")
    check("netd spawned" not in serial,
          "production netd was NOT spawned without the device")
    check("netd: starting" not in serial,
          "no netd instance ran at all without the device")
    for prior, tests in (("m1", 8), ("m2", 21), ("m3", 13), ("m4", 9),
                         ("m5", 6)):
        check(f"{prior}: RESULT PASS" in serial,
              f"prior-milestone regression: {prior} RESULT PASS ({tests} "
              "tests) in the same no-net boot")
    check("halting via UEFI ResetSystem(shutdown)" in serial,
          "the kernel declared its clean halt")
    check(rc == 0, f"QEMU exited cleanly (rc={rc}, {dt:.1f}s)")
    return ok


if __name__ == "__main__":
    rc = mtest.run_milestone("m6", EXPECTED_TESTS)
    if rc == 0:
        serial = arena_env.build_dir().joinpath("serial-m6.log").read_text()
        if not check_with_net_extras(serial):
            rc = 1
    if rc == 0:
        if not boot_without_net():
            rc = 1
    print(f"[test-m6] {'=' * 46}")
    print(f"[test-m6] MILESTONE 6.1: {'PASS' if rc == 0 else 'FAIL'} "
          "(with-net link proof + no-net honest SKIP)")
    sys.exit(rc)
