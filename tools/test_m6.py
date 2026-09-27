#!/usr/bin/env python3
"""Milestone 6 automated boot test (docs/TESTING.md, ADR-0024).

Pipeline and verdict logic live in tools/mtest.py (including the
cross-milestone regression guard: the same boot must still carry
PASSing m1..m5 RESULT lines, and the harness attaches BOTH fixtures —
the virtio-blk scratch disk and, since M6.1, the slirp NIC:
arena_env.net_args).

Current coverage:
  M6.2 — the shared virtio core + the entropy proof (ADR-0025):
  * rng_service   — the kernel spawns rngd (registry image 8, the
                    userspace virtio-rng driver built on the SHARED
                    virtio core) and rngtest (image 9). The client
                    takes two 4 KiB draws into SEPARATE frames, each
                    LENT through IPC so the device DMAs entropy
                    straight into the client's own page, and asserts
                    real variance: neither draw all-zero, neither a
                    single repeated byte, and the two different.
                    The kernel witnesses the mechanics: exactly two
                    relay deliveries (one MSI per draw), exact exit
                    badges, exit 42 from both, frame-exact teardown.
                    Entropy QUALITY is the host backend's business —
                    the suite proves transport and fill, honestly.

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

EXPECTED_TESTS = ["net_service", "rng_service", "input_service"]

# The keystrokes the harness types on the VIRTUAL KEYBOARD (M6.3,
# ADR-0026), paced by a serial marker like every other harness action:
# once the m6 suite's inputd instance announces DRIVER_OK — after which
# the device has buffers to fill — `arena` is typed through QMP,
# keystroke by keystroke. The sequence is a documented fixture
# contract, exactly like the slirp gateway's 10.0.2.2 in the net test;
# inputtest verifies the decoded bytes against it.
KEY_SCRIPT = [(b"inputd: virtio-input ready", 1, "arena")]


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

    # --- M6.2: the entropy service (ADR-0025) ---
    check("rngtest: PASS — two device-filled draws" in serial,
          "rngtest verified both draws and reported its PASS line")
    check("2 interrupt deliveries on relay vector 48 (one per draw"
          in serial,
          "the kernel counted exactly two hardware deliveries — one "
          "MSI per draw (no polling)")
    check("rngd spawned: pid" in serial,
          "the PRODUCTION rngd spawned at boot with the fixture attached")
    check(serial.count("virtio-rng ready") == 2,
          "rngd reached DRIVER_OK twice (the suite's instance + the "
          "production service)")
    # The draws are real device entropy: the fingerprints the client
    # logs must differ from each other (a cached or looping source
    # would repeat). The in-guest checks already failed the boot if
    # not; this asserts the EVIDENCE is in the log, not just a claim.
    m = re.search(r"draw A ([0-9a-f]{8})\u2026 vs draw B ([0-9a-f]{8})\u2026",
                  serial)
    check(m is not None and m.group(1) != m.group(2),
          "the logged draw fingerprints are present and differ "
          f"({m.group(1)} vs {m.group(2)})" if m else
          "the logged draw fingerprints are present and differ")

    # --- M6.3: the input service (ADR-0026) ---
    check("inputtest: PASS — the injected keystrokes arrived decoded "
          "and in order: \"arena\"" in serial,
          "inputtest read back the exact sequence typed on the virtual "
          "keyboard")
    check("service mode — no console authority" in serial,
          "the suite's inputd instance detected the WITHHELD ConsoleInput "
          "capability and ran as a service")
    check("console mode — keystrokes feed the shell" in serial,
          "the PRODUCTION inputd instance found the capability and ran as "
          "the console feeder")
    check("inputd spawned: pid" in serial,
          "the PRODUCTION inputd spawned at boot with the keyboard attached")
    check(serial.count("virtio-input ready") == 2,
          "inputd reached DRIVER_OK twice (the suite's instance + the "
          "production service)")
    check("virtio device_id 0x1052 \u2192 type 18 (input), modern" in serial,
          "the kernel's PCI scan classified the keyboard as modern "
          "virtio-input (no transitional alias exists for this class)")
    return ok


def boot_without_net() -> bool:
    """The compatibility-window boot (ADR-0024/0025): NO optional fixtures.

    A pre-v0.6.0 QEMU invocation must stay bootable-green with honest
    SKIPs — the network service was new in v0.6.0 and the entropy
    service in v0.7.0, and users' saved commands predate both.
    """
    label = "test-m6-nonet"
    ok = True

    def check(cond: bool, msg: str) -> None:
        nonlocal ok
        print(f"[test-m6-nonet] {'PASS' if cond else 'FAIL'}: {msg}")
        ok = ok and cond

    try:
        esp = mtest.build(label)
        rc, serial, dt = mtest.run_qemu(label, esp, net=False,
                                        rng=False, kbd=False)
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
    check(re.search(r"^m6:test:rng_service: SKIP \(no virtio-rng device",
                    serial, re.MULTILINE) is not None,
          "the rng_service test reported an HONEST SKIP too")
    check("entropy service stays offline" in serial,
          "the kernel logged the entropy service offline")
    check("rngd spawned" not in serial,
          "production rngd was NOT spawned without the device")
    check("rngd: starting" not in serial,
          "no rngd instance ran at all without the device")
    check(re.search(r"^m6:test:input_service: SKIP \(no virtio-input device",
                    serial, re.MULTILINE) is not None,
          "the input_service test reported an HONEST SKIP too")
    check("keyboard service stays offline" in serial,
          "the kernel logged the keyboard service offline")
    check("inputd spawned" not in serial,
          "production inputd was NOT spawned without the device")
    check("inputd: starting" not in serial,
          "no inputd instance ran at all without the device")
    check("arena>" in serial,
          "the shell still reached its prompt on the SERIAL console with "
          "no keyboard attached (the keyboard is additive, never required)")
    for prior, tests in (("m1", 8), ("m2", 21), ("m3", 13), ("m4", 9),
                         ("m5", 6)):
        check(f"{prior}: RESULT PASS" in serial,
              f"prior-milestone regression: {prior} RESULT PASS ({tests} "
              "tests) in the same no-net boot")
    check("halting via UEFI ResetSystem(shutdown)" in serial,
          "the kernel declared its clean halt")
    check(rc == 0, f"QEMU exited cleanly (rc={rc}, {dt:.1f}s)")
    return ok


def boot_with_keyboard_but_nobody_typing() -> bool:
    """A keyboard attached and NO typist (ADR-0026).

    This is the configuration a real user gets: they add
    `-device virtio-keyboard-pci`, boot, and go get coffee while the
    suite runs. A keyboard produces nothing unless someone types —
    unlike slirp, which answers ARP by itself, and the entropy source,
    which fills buffers by itself — so the suite cannot simply wait
    forever. It waits a bounded window, calls the driver's wait off,
    and reports an HONEST SKIP: attaching a device must never make a
    machine unusable, and a test that proved nothing must never claim
    it did.
    """
    label = "test-m6-notypist"
    ok = True

    def check(cond: bool, msg: str) -> None:
        nonlocal ok
        print(f"[{label}] {'PASS' if cond else 'FAIL'}: {msg}")
        ok = ok and cond

    try:
        esp = mtest.build(label)
        # keys=[] is the whole point: the keyboard is THERE, nobody
        # uses it. The serial feeder still types `shutdown` at the
        # prompt — which only happens if the machine survived the
        # suite.
        rc, serial, dt = mtest.run_qemu(label, esp, keys=[])
    except Exception as e:  # noqa: BLE001 — any harness fault is a FAIL
        print(f"[{label}] FAIL: the no-typist boot crashed the harness: {e}")
        return False
    arena_env.build_dir().joinpath("serial-m6-notypist.log").write_text(serial)

    check("PANIC" not in serial, "no kernel panic when nobody types")
    check(re.search(r"^m6:test:input_service: SKIP \(no keystrokes arrived",
                    serial, re.MULTILINE) is not None,
          "the input_service test reported an HONEST SKIP (not a FAIL, "
          "not a fake PASS)")
    check(re.search(r"^m6: RESULT SKIP ", serial, re.MULTILINE) is not None,
          "the m6 RESULT line says SKIP")
    check("calling the driver's wait off" in serial,
          "the suite ended the wait deliberately after its window "
          "(the driver cannot time itself out — ADR-0026)")
    check("nobody typed during this boot" in serial,
          "inputtest reported the honest outcome instead of hanging")
    check("inputd spawned: pid" in serial,
          "the production keyboard service still came up for the user")
    check("arena>" in serial,
          "the machine reached the shell prompt — attaching a keyboard "
          "nobody uses does NOT make the machine unusable")
    check("halting via UEFI ResetSystem(shutdown)" in serial,
          "the kernel declared its clean halt after a serial `shutdown`")
    check(rc == 0, f"QEMU exited cleanly (rc={rc}, {dt:.1f}s)")
    return ok


if __name__ == "__main__":
    rc = mtest.run_milestone("m6", EXPECTED_TESTS, keys=KEY_SCRIPT)
    if rc == 0:
        serial = arena_env.build_dir().joinpath("serial-m6.log").read_text()
        if not check_with_net_extras(serial):
            rc = 1
    if rc == 0:
        if not boot_without_net():
            rc = 1
    if rc == 0:
        if not boot_with_keyboard_but_nobody_typing():
            rc = 1
    print(f"[test-m6] {'=' * 46}")
    print(f"[test-m6] MILESTONE 6.1+6.2+6.3: {'PASS' if rc == 0 else 'FAIL'} "
          "(link + entropy + input proofs, and honest SKIPs with no "
          "fixtures)")
    sys.exit(rc)
