#!/usr/bin/env python3
"""Controls proving failed runs can be packaged diagnostically without qualification."""

import hashlib
import json
import subprocess
import sys
import tarfile
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / "tools"))
import v1_diagnostic_evidence as diagnostic  # noqa: E402


class DiagnosticArchiveTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix="v1-diagnostic-test-")
        self.base = Path(self.tmp.name)
        self.evidence = self.base / "failed-run"
        self.packaging = self.base / "packaging-attempt"
        self.evidence.mkdir()
        self.packaging.mkdir()
        self.commit = subprocess.check_output(
            ["git", "rev-parse", "HEAD"], cwd=ROOT, text=True
        ).strip()
        (self.evidence / "gate/receipts").mkdir(parents=True)
        (self.evidence / "gate/receipts/build.json").write_text(json.dumps({
            "stage": "build", "exit": 0, "source_commit": self.commit,
            "code_fingerprint": "test-fingerprint",
            "efi_sha256": "e" * 64, "esp_sha256": "f" * 64,
        }))
        (self.evidence / "gate/commands.txt").parent.mkdir(parents=True, exist_ok=True)
        (self.evidence / "gate/commands.txt").write_text("python3 focused-test.py\n")
        (self.evidence / "env").mkdir()
        (self.evidence / "env/qemu-version.txt").write_text("QEMU emulator version 11.1.2\n")
        (self.evidence / "guest-gcc/shots").mkdir(parents=True)
        (self.evidence / "guest-gcc/evidence.json").write_text(json.dumps({
            "verdict": {"pass": False},
            "required_frames": list(diagnostic.REQUIRED_GUEST_FRAMES),
            "checkpoints": [
                {"case": 5, "name": "mouse response", "result": "FAIL",
                 "evidence_ref": "shots/cp05-mouse-response.png",
                 "detail": "acknowledgment without a captured visible response"},
                {"case": 6, "name": "isolation", "result": "NOT RUN",
                 "evidence_ref": "", "detail": "blocked by checkpoint 5"},
            ],
            "frames": {"shots/cp05-mouse-response.png": {"published_png_sha256": "MISSING"}},
        }))
        (self.evidence / "guest-gcc/shots/__ack-probe__.ppm").write_bytes(b"P6\n1 1\n255\x00\x01\x02")
        (self.evidence / "omitted-inputs.json").write_text(json.dumps({"omitted": []}))
        logs = self.packaging / "gate/logs"
        logs.mkdir(parents=True)
        (logs / "final-aggregate.log").write_text(
            "requirement  result     coverage\n"
            "1            FAIL       visual response\n"
            "4b           OPEN       foreign capability\n"
            "V1 GATE (full): FAIL\n"
        )
        (logs / "package.log").write_text("[evidence] checkpoint frame missing\n")
        (logs / "verify.log").write_text("[evidence] archive absent\n")
        receipts = self.packaging / "gate/receipts"
        receipts.mkdir()
        (receipts / "package.json").write_text(json.dumps({"stage": "package", "exit": 1}))
        (receipts / "verify.json").write_text(json.dumps({"stage": "verify", "exit": 2}))
        self.archive = self.base / "failed-run-diagnostic.tar.gz"

    def tearDown(self):
        self.tmp.cleanup()

    def test_failed_run_packages_and_missing_capture_remains_fail(self):
        result = diagnostic.create_archive(self.evidence, self.packaging, self.archive)
        self.assertTrue(self.archive.is_file())
        self.assertEqual("FAIL", result["manifest"]["observed_full_gate_result"])
        self.assertEqual("NONE", result["manifest"]["qualification_claim"])
        self.assertFalse(result["manifest"]["candidate_qualified"])
        self.assertEqual(1, result["manifest"]["raw_ppm_count"])
        self.assertEqual(18, len(result["manifest"]["missing_required_frames"]))
        rc, problems = diagnostic.verify_archive(self.archive)
        self.assertEqual(0, rc, problems)
        with tarfile.open(self.archive, "r:gz") as tar:
            names = tar.getnames()
            ppm_member = next(name for name in names if name.endswith("__ack-probe__.ppm"))
            self.assertEqual(b"P6\n1 1\n255\x00\x01\x02", tar.extractfile(ppm_member).read())
            report_member = next(name for name in names if name.endswith("DIAGNOSTIC-REPORT.json"))
            report = json.loads(tar.extractfile(report_member).read())
        self.assertEqual("FAIL", report["observed_full_gate_result"])
        self.assertFalse(report["candidate_qualified"])
        self.assertEqual(self.commit, report["source_commit"])
        self.assertEqual("e" * 64, report["artifact_identities"]["efi_sha256"])
        self.assertIn("QEMU emulator version 11.1.2", report["runtime_identity"]["env/qemu-version.txt"])
        self.assertIn("focused-test.py", report["full_gate_command_transcript"])
        self.assertIn("focused-test.py", report["command_transcripts"]["r6/gate/commands.txt"])
        self.assertIn("guest-gcc/shots/cp05-mouse-response.png",
                      [row["path"] for row in report["missing_checkpoint_files"]])
        self.assertEqual("OPEN", next(row["status"] for row in report["requirement_matrix"]
                                       if row["id"] == "4b"))

    def test_archive_verifier_rejects_unlisted_member(self):
        diagnostic.create_archive(self.evidence, self.packaging, self.archive)
        tampered = self.base / "unlisted.tar.gz"
        with tarfile.open(self.archive, "r:gz") as source, tarfile.open(tampered, "w:gz") as target:
            for member in source.getmembers():
                target.addfile(member, source.extractfile(member) if member.isfile() else None)
            info = tarfile.TarInfo("failed-run-diagnostic/unlisted.txt")
            body = b"unlisted evidence"
            info.size = len(body)
            target.addfile(info, __import__("io").BytesIO(body))
        rc, problems = diagnostic.verify_archive(tampered)
        self.assertEqual(1, rc)
        self.assertTrue(any("unlisted file" in problem for problem in problems), problems)

    def test_archive_verifier_rejects_payload_digest_mismatch(self):
        diagnostic.create_archive(self.evidence, self.packaging, self.archive)
        tampered = self.base / "changed.tar.gz"
        with tarfile.open(self.archive, "r:gz") as source, tarfile.open(tampered, "w:gz") as target:
            for member in source.getmembers():
                if member.isfile() and member.name.endswith("__ack-probe__.ppm"):
                    original = source.extractfile(member).read()
                    changed = bytes([original[0] ^ 1]) + original[1:]
                    info = tarfile.TarInfo(member.name)
                    info.size = len(changed)
                    target.addfile(info, __import__("io").BytesIO(changed))
                else:
                    target.addfile(member, source.extractfile(member) if member.isfile() else None)
        rc, problems = diagnostic.verify_archive(tampered)
        self.assertEqual(1, rc)
        self.assertTrue(any("SHA-256 mismatch" in problem for problem in problems), problems)

    def test_failed_run_does_not_require_qualification_frame_set_to_be_complete(self):
        # The diagnostic package is deliberately permissive about missing
        # qualification captures; it records them, while v1_evidence remains
        # the separate strict qualification gate.
        result = diagnostic.create_archive(self.evidence, self.packaging, self.archive)
        self.assertEqual(18, len(result["manifest"]["missing_required_frames"]))
        self.assertNotIn("qualification_frames_pass", result["manifest"])


if __name__ == "__main__":
    unittest.main(verbosity=2)
