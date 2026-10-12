#!/usr/bin/env python3
"""Regression tests for the SDL3 Guest Qualification Harness using mocked interfaces.

Validates:
1. Target A audit qualification (integer-only, 0 FP/SIMD, loader limits).
2. Target B audit qualification (2,036 SSE detected, loader limits, BLOCKED status).
3. Hash integrity enforcement: Fails strictly on SHA-256 mismatch.
4. Guest mode fail-closed behavior when QEMU is missing (returns exit code 2).
5. Guest mode architectural refusal for upstream Target B (returns exit code 2).
6. Full 12-step guest qualification under mocked QEMU/QMP/serial environment (returns exit code 0).
7. Negative regression tests:
   - Missing expected frame progression fails.
   - Missing keyboard-response evidence fails.
   - Missing mouse-response evidence fails.
   - Missing process-exit evidence fails.
   - Invalid screenshots fail.
   - Nonzero / failed QEMU execution cannot be reported as success.
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


FULL_VALID_MOCK_LOG = (
    "[desktop] real desktop frame presented\n"
    "[desktop] desktop surface shows /Users/user/Desktop\n"
    "[desktop] APB1 installed; signed version=1 (no launch authority implied)\n"
    "[sdl3-app] SDL3 video subsystem initialized successfully!\n"
    "[sdl3-app] SDL_Window created: 0x240000 (id=1)\n"
    "[sdl3-app] Surface acquired: 320x240, format=1, pitch=1280, pixels=0x250000\n"
    "[sdl3-app] Entering interactive frame loop (running for 120 frames)...\n"
    "[sdl3-app] Frame milestone reached: frame=0 (initial frame)\n"
    "[sdl3-app] Frame milestone reached: frame=25 (running smoothly)\n"
    "[sdl3-app] Event: Mouse button pressed -> state transition color=1\n"
    "[sdl3-app] Event: Mouse button released -> state transition color=1\n"
    "[sdl3-app] Event: Spacebar pressed -> state transition color=2\n"
    "[sdl3-app] Frame milestone reached: frame=60 (midpoint reached)\n"
    "[sdl3-app] Frame milestone reached: frame=90 (progression continued)\n"
    "[sdl3-app] Frame milestone reached: frame=119 (final frame completed)\n"
    "[sdl3-app] Shutting down SDL3 window and video subsystem...\n"
    "[sdl3-app] Clean termination with exit code 0\n"
    "[desktop] child Process-cap exit status=0\n"
    "[arena INFO kernel] shutdown requested by pid 157 through its Power cap - goodnight\n"
    "halting via UEFI ResetSystem(shutdown)\n"
)


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
    def _run_mocked_guest(
        self,
        serial_log_text: str,
        qemu_exit_code: int = 0,
        fail_pixels: bool = False,
        timeout_override: float = 0.05,
    ) -> int:
        """Helper to run the 12-step guest qualification harness under controlled mocks."""
        with tempfile.TemporaryDirectory() as td:
            work_dir = Path(td)
            scratch_img = work_dir / "scratch.img"
            scratch_img.write_bytes(b"\x00" * 1024)

            mock_afs2_tree = {
                "/System/Applications/org.arenaos.sdl3app/1/bin/sdl3_app": b"ELF_DATA"
            }

            mock_qmp_conn = MagicMock()
            def mock_qmp_command(cmd_name, **kwargs):
                if cmd_name == "screendump":
                    fname = kwargs.get("filename")
                    if fname:
                        Path(fname).write_bytes(b"P6\n800 600\n255\n" + b"\x00" * (800 * 600 * 3))
                return {}
            mock_qmp_conn.command.side_effect = mock_qmp_command

            mock_guest = MagicMock()
            mock_guest.poll.return_value = None
            mock_guest.wait.return_value = qemu_exit_code
            mock_guest.stdin = io.BytesIO()

            mock_fixture = MagicMock()
            mock_fixture.stdout.readline.return_value = b"READY\n"

            def mock_popen(cmd, *args, **kwargs):
                if isinstance(cmd, list) and "vcon.py" in str(cmd[2]):
                    return mock_fixture
                if isinstance(cmd, list) and "network_fixture.py" in str(cmd[2]):
                    return mock_fixture
                stdout_f = kwargs.get("stdout")
                if stdout_f:
                    stdout_f.write(serial_log_text.encode())
                    stdout_f.flush()
                qmp_sock = work_dir / "qmp.sock"
                qmp_sock.touch()
                return mock_guest

            mock_pixels = MagicMock()
            if fail_pixels:
                mock_pixels.side_effect = AssertionError("Mock pixel validation failure: invalid window content")

            with patch.object(harness, "WAIT_TIMEOUT_OVERRIDE", timeout_override):
                with patch.object(harness, "ensure_guest_artifacts", return_value="dummy_efi_sha"):
                    with patch.object(harness, "seed_disk"):
                        with patch.object(harness.subprocess, "run"):
                            with patch.object(harness, "verify_screenshot_pixels", mock_pixels):
                                with patch.object(harness.phase10, "qemu_cmd", return_value=["qemu-system-x86_64"]):
                                    with patch.object(harness.subprocess, "Popen", side_effect=mock_popen):
                                        with patch.object(harness.qmp, "Qmp", return_value=mock_qmp_conn):
                                            with patch.object(harness.afs2, "walk", return_value=mock_afs2_tree):
                                                with patch.object(harness, "WORK_DIR", work_dir):
                                                    return harness.run_qemu_test("baseline")

    def test_full_12_step_guest_qualification_success(self):
        """Positive test: Valid application lifecycle, milestones, and input events pass."""
        rc = self._run_mocked_guest(FULL_VALID_MOCK_LOG, qemu_exit_code=0)
        self.assertEqual(rc, 0, "Mocked 12-step guest qualification must succeed with exit 0")

    def test_missing_frame_progression_fails(self):
        """Negative test: Missing later frame progression (stopped after frame 25) must fail."""
        truncated_log = (
            "[desktop] real desktop frame presented\n"
            "[desktop] desktop surface shows /Users/user/Desktop\n"
            "[desktop] APB1 installed; signed version=1 (no launch authority implied)\n"
            "[sdl3-app] SDL3 video subsystem initialized successfully!\n"
            "[sdl3-app] SDL_Window created: 0x240000 (id=1)\n"
            "[sdl3-app] Surface acquired: 320x240, format=1, pitch=1280, pixels=0x250000\n"
            "[sdl3-app] Entering interactive frame loop (running for 120 frames)...\n"
            "[sdl3-app] Frame milestone reached: frame=0 (initial frame)\n"
            "[sdl3-app] Frame milestone reached: frame=25 (running smoothly)\n"
            "[sdl3-app] Event: Mouse button pressed -> state transition color=1\n"
            "[sdl3-app] Event: Spacebar pressed -> state transition color=2\n"
            # Missing frame=60, frame=90, frame=119
        )
        rc = self._run_mocked_guest(truncated_log, timeout_override=0.02)
        self.assertEqual(rc, 1, "Missing frame milestones 60/90/119 must cause test failure (exit code 1)")

    def test_missing_keyboard_response_fails(self):
        """Negative test: Application failing to handle spacebar event must cause test failure."""
        no_kbd_log = FULL_VALID_MOCK_LOG.replace(
            "[sdl3-app] Event: Spacebar pressed -> state transition color=2\n", ""
        )
        rc = self._run_mocked_guest(no_kbd_log, timeout_override=0.02)
        self.assertEqual(rc, 1, "Missing spacebar keyboard response evidence must cause test failure (exit code 1)")

    def test_missing_mouse_response_fails(self):
        """Negative test: Application failing to handle mouse click event must cause test failure."""
        no_mouse_log = FULL_VALID_MOCK_LOG.replace(
            "[sdl3-app] Event: Mouse button pressed -> state transition color=1\n", ""
        )
        rc = self._run_mocked_guest(no_mouse_log, timeout_override=0.02)
        self.assertEqual(rc, 1, "Missing mouse button response evidence must cause test failure (exit code 1)")

    def test_missing_process_exit_fails(self):
        """Negative test: Missing clean application exit status must cause test failure."""
        no_exit_log = FULL_VALID_MOCK_LOG.replace(
            "[sdl3-app] Clean termination with exit code 0\n", ""
        ).replace(
            "[desktop] child Process-cap exit status=0\n", ""
        )
        rc = self._run_mocked_guest(no_exit_log, timeout_override=0.02)
        self.assertEqual(rc, 1, "Missing process exit status evidence must cause test failure (exit code 1)")

    def test_invalid_screenshot_fails(self):
        """Negative test: Screenshot verification failure must cause test failure."""
        rc = self._run_mocked_guest(FULL_VALID_MOCK_LOG, fail_pixels=True)
        self.assertEqual(rc, 1, "Failed visual pixel verification must cause test failure (exit code 1)")

    def test_qemu_nonzero_exit_fails(self):
        """Negative test: Nonzero QEMU exit status must cause test failure."""
        rc = self._run_mocked_guest(FULL_VALID_MOCK_LOG, qemu_exit_code=1)
        self.assertEqual(rc, 1, "Nonzero QEMU exit code must cause test failure (exit code 1)")


if __name__ == "__main__":
    unittest.main()
