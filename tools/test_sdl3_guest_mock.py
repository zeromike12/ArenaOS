#!/usr/bin/env python3
"""Regression tests for the SDL3 Guest Qualification Harness using mocked interfaces.

Validates:
1. Target A audit qualification (integer-only, 0 FP/SIMD, loader limits).
2. Target B audit qualification (2,036 SSE detected, loader limits, BLOCKED status).
3. Hash integrity enforcement: Fails strictly on SHA-256 mismatch.
4. Guest mode fail-closed behavior when QEMU is missing (returns exit code 2).
5. Guest mode architectural refusal for upstream Target B (returns exit code 2).
6. Full 12-step guest qualification under mocked QEMU/QMP/serial environment (returns exit code 0).
"""

from __future__ import annotations

import io
import os
import shutil
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import MagicMock, patch

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / "tools"))

import test_sdl3_guest as harness


class TestHarnessAudit(unittest.TestCase):
    def test_audit_baseline(self):
        rc = harness.run_audit_mode("baseline")
        self.assertEqual(rc, 0, "Target A baseline audit must pass")

    def test_audit_upstream(self):
        rc = harness.run_audit_mode("upstream")
        self.assertEqual(rc, 0, "Target B upstream audit must pass static check")

    def test_audit_all(self):
        rc = harness.run_audit_mode("all")
        self.assertEqual(rc, 0, "All-targets audit must pass")

    def test_pinned_hash_failure(self):
        with tempfile.NamedTemporaryFile() as tmp:
            tmp.write(b"corrupted binary data")
            tmp.flush()
            with patch.object(harness, "SDL3_EXE", Path(tmp.name)):
                with self.assertRaises(ValueError) as ctx:
                    harness.ensure_baseline_bundle()
                self.assertIn("SHA-256 mismatch", str(ctx.exception))


class TestHarnessGuestBlocked(unittest.TestCase):
    def test_guest_missing_qemu_returns_exit_2(self):
        with patch.object(harness.phase10, "qemu_cmd", side_effect=FileNotFoundError):
            rc = harness.run_qemu_test("baseline")
            self.assertEqual(rc, 2, "Missing QEMU must return exit code 2 (NOT RUN)")

    def test_guest_upstream_architectural_refusal_returns_exit_2(self):
        rc = harness.run_qemu_test("upstream")
        self.assertEqual(rc, 2, "Target B upstream in guest mode must return exit code 2 (BLOCKED)")

    def test_guest_all_targets_returns_exit_2(self):
        with patch.object(harness.phase10, "qemu_cmd", side_effect=FileNotFoundError):
            rc = harness.run_qemu_test("all")
            self.assertEqual(rc, 2, "Target all in guest mode without QEMU must return exit code 2")


class TestHarnessMocked12StepGuest(unittest.TestCase):
    @patch.object(harness, "ensure_guest_artifacts", return_value="dummy_efi_sha")
    @patch.object(harness, "seed_disk")
    @patch.object(harness.subprocess, "run")
    @patch.object(harness, "verify_screenshot_pixels")
    def test_full_12_step_guest_qualification_success(
        self, mock_pixels, mock_subproc_run, mock_seed, mock_artifacts
    ):
        # Construct mock serial log output containing all 12 step milestones
        mock_serial_log = (
            "[desktop] real desktop frame presented\n"
            "[desktop] desktop surface shows /Users/user/Desktop\n"
            "[desktop] APB1 installed; signed version=1 (no launch authority implied)\n"
            "[sdl3-app] SDL3 video subsystem initialized successfully!\n"
            "[sdl3-app] SDL_Window created: 0x240000 (id=1)\n"
            "[sdl3-app] Surface acquired: 320x240, format=1, pitch=1280, pixels=0x250000\n"
            "[sdl3-app] Entering interactive frame loop (running for 30 frames)...\n"
            "[sdl3-app] frame rendered; damage published; frame=15 (box at 120,80)\n"
            "[sdl3-app] frame rendered; damage published; frame=30 (box at 150,110)\n"
            "[sdl3-app] Shutting down SDL3 window and video subsystem...\n"
            "[sdl3-app] Clean termination with exit code 0\n"
            "[desktop] child Process-cap exit status=0\n"
            "[arena INFO kernel] shutdown requested by pid 157 through its Power cap - goodnight\n"
            "halting via UEFI ResetSystem(shutdown)\n"
        )

        with tempfile.TemporaryDirectory() as td:
            work_dir = Path(td)
            scratch_img = work_dir / "scratch.img"
            scratch_img.write_bytes(b"\x00" * 1024)

            # Create mock AFS2 tree
            mock_afs2_tree = {
                "/System/Applications/org.arenaos.sdl3app/1/bin/sdl3_app": b"ELF_DATA"
            }

            # Create mock QMP client
            mock_qmp_conn = MagicMock()
            def mock_qmp_command(cmd_name, **kwargs):
                if cmd_name == "screendump":
                    fname = kwargs.get("filename")
                    if fname:
                        Path(fname).write_bytes(b"P6\n800 600\n255\n" + b"\x00" * (800 * 600 * 3))
                return {}
            mock_qmp_conn.command.side_effect = mock_qmp_command

            # Create mock guest subprocess
            mock_guest = MagicMock()
            mock_guest.poll.return_value = None
            mock_guest.wait.return_value = 0
            mock_guest.stdin = io.BytesIO()

            # Create mock console and actor fixtures
            mock_fixture = MagicMock()
            mock_fixture.stdout.readline.return_value = b"READY\n"

            def mock_popen(cmd, *args, **kwargs):
                if isinstance(cmd, list) and "vcon.py" in str(cmd[2]):
                    return mock_fixture
                if isinstance(cmd, list) and "network_fixture.py" in str(cmd[2]):
                    return mock_fixture
                # QEMU command
                # Write serial log to stdout file descriptor
                stdout_f = kwargs.get("stdout")
                if stdout_f:
                    stdout_f.write(mock_serial_log.encode())
                    stdout_f.flush()
                # Create fake qmp socket file
                qmp_sock = work_dir / "qmp.sock"
                qmp_sock.touch()
                return mock_guest

            with patch.object(harness.phase10, "qemu_cmd", return_value=["qemu-system-x86_64"]):
                with patch.object(harness.subprocess, "Popen", side_effect=mock_popen):
                    with patch.object(harness.qmp, "Qmp", return_value=mock_qmp_conn):
                        with patch.object(harness.afs2, "walk", return_value=mock_afs2_tree):
                            with patch.object(harness, "WORK_DIR", work_dir):
                                rc = harness.run_qemu_test("baseline")

            self.assertEqual(rc, 0, "Mocked 12-step guest qualification must succeed with exit 0")
            # Verify QMP commands were issued
            mock_qmp_conn.command.assert_any_call("screendump", filename=unittest.mock.ANY, format="ppm")
            mock_qmp_conn.type_text.assert_called_with("sdl3", gap_s=0.04)
            mock_pixels.assert_called()


if __name__ == "__main__":
    unittest.main()
