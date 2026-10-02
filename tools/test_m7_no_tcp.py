#!/usr/bin/env python3
"""Boot a shipping-style image with no TCP host actor (ADR-0035).

The test suite must not halt a real user's machine merely because the
harness-only host port is absent. Conversely test_m7.py insists on the
positive wire proof, so a missing actor cannot turn a test boot green.
"""
import sys
from pathlib import Path
sys.path.insert(0, str(Path(__file__).resolve().parent))
import mtest  # noqa: E402


def main() -> int:
    label = "tcp-no-peer"
    esp = mtest.build(label)
    rc, serial, dt = mtest.run_qemu(label, esp, tcp_peer=False)
    ok = mtest.evaluate(label, "m7", ["timer_facility", "arp_service"], rc, serial, dt)
    skipped = "arptest: TCP SKIP — no host peer at 10.0.2.2:54321" in serial
    no_false_pass = "TCP active OPEN returned a bearer" not in serial
    print(f"[{label}] {'PASS' if skipped and no_false_pass else 'FAIL'}: "
          "an absent actor is an explicit SKIP, never a fake TCP PASS")
    return 0 if ok and skipped and no_false_pass else 1


if __name__ == "__main__":
    sys.exit(main())
