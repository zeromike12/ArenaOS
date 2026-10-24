#!/usr/bin/env python3
"""Host-only negative controls for the portable Phase 15 runner/verifier."""
from __future__ import annotations

import json
import os
import sys
import tempfile
import unittest
from pathlib import Path

KIT_TOOLS = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(KIT_TOOLS))

import mtest  # noqa: E402
import run_external  # noqa: E402
import verify_receipt  # noqa: E402


class ExternalKitNegativeControls(unittest.TestCase):
    def test_missing_guest_proof_is_rejected(self) -> None:
        with self.assertRaises(verify_receipt.ReceiptError):
            verify_receipt.check_guest_log("")

    def test_old_esp_identity_is_rejected(self) -> None:
        old_esp = "33dd1576ab597a7d0b3e9a9100bc618e9b251a7a645e77e79d8852a404c40a6a"
        receipt = {
            "schema": "arenaos.phase15.external-qualification.v1",
            "backend_requested": "kvm",
            "status": "BLOCKED",
            "strict_acceptance": {
                "retry_count": 0,
                "attempt_count": 2,
                "strict_image_build_source_commit": verify_receipt.STRICT_SOURCE_COMMIT,
            },
            "images": {"arena-esp.img": {"sha256": old_esp}},
        }
        with tempfile.TemporaryDirectory(prefix="phase15-wrong-identity-") as temp:
            path = Path(temp) / "receipt.json"
            path.write_text(json.dumps(receipt))
            with self.assertRaisesRegex(
                    verify_receipt.ReceiptError, "incorrect image identity"):
                verify_receipt.verify(path)

    def test_qemu_zero_with_semantic_guest_failure_stays_failed(self) -> None:
        # This negative-only serial fixture models the TCG failure shape. It
        # contains no claimed vector-19 delivery and cannot produce PASS.
        serial = """\
[arena INFO boot] ArenaOS boot stage v0.1.0
m3: RESULT PASS (13/13)
m4: RESULT PASS (9/9)
m15:test:xf_delivery_probe: FAIL (minimal unmasked SQRTSS did not fault)
m15: RESULT FAIL (5/6)
"""
        semantic_exit = mtest.semantic_exit_rc(0, serial, "negative-control")
        self.assertEqual(semantic_exit, 97)
        stages = run_external.classify_trial(
            serial, qemu_exit=0, semantic_exit=semantic_exit,
            harness_passed=False)
        self.assertEqual(stages["qemu_process_exit"], "PASS")
        self.assertEqual(stages["semantic_guest_exit"], "FAIL")
        self.assertEqual(stages["vector_19_delivery"], "FAIL")
        self.assertEqual(stages["trial"], "FAIL")

    def test_firmware_start_without_arenaos_is_separate_failure_boundary(self) -> None:
        stages = run_external.classify_trial(
            'BdsDxe: starting Boot0002 "UEFI QEMU HARDDISK"',
            qemu_exit=0, semantic_exit=97, harness_passed=False)
        self.assertEqual(stages["firmware_boot"], "PASS")
        self.assertEqual(stages["arenaos_boot"], "NOT_OBSERVED")
        self.assertEqual(stages["failure_boundary"],
                         "firmware_did_not_transfer_to_arenaos")
        self.assertEqual(stages["trial"], "FAIL")

    @unittest.skipUnless(os.name != "nt", "fake executable control is POSIX-only")
    def test_failed_accelerator_initialization_is_blocked(self) -> None:
        # This mocks the QEMU process boundary solely to exercise failure
        # capture; it does not emulate a guest or establish accelerator use.
        with tempfile.TemporaryDirectory(prefix="phase15-fake-qemu-") as temp:
            executable = Path(temp) / "fake-qemu"
            executable.write_text("""#!/usr/bin/env python3
import sys
args = sys.argv[1:]
if args == ["--version"]:
    print("QEMU emulator version 11.1.2")
    raise SystemExit(0)
if args == ["-accel", "help"]:
    print("Accelerators: kvm tcg")
    raise SystemExit(0)
print("Could not initialize KVM: mocked negative control", file=sys.stderr)
raise SystemExit(1)
""")
            executable.chmod(0o755)
            with self.assertRaises(run_external.AcceleratorProbeError) as raised:
                run_external.probe_accelerator([str(executable)], "kvm")
            self.assertFalse(raised.exception.evidence["initialized"])
            self.assertEqual(raised.exception.evidence["qemu_exit_status"], 1)
            self.assertIn("mocked negative control",
                          raised.exception.evidence["probe_output"])
            self.assertIn("-accel", raised.exception.evidence["command"])


if __name__ == "__main__":
    unittest.main(verbosity=2)
