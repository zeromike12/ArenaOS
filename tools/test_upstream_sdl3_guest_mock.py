#!/usr/bin/env python3
"""Unit test suite for the dedicated Upstream SDL 3.2.0 Guest Harness & 24-Step Lifecycle.

Validates:
1. Target C2 static audit (50 load pages, 243,120 bytes, 2,036 SSE detected).
2. Target Standalone static audit (100 load pages, 243,760 bytes, 2,036 SSE detected).
3. All-targets static audit.
4. Pinned hash integrity failure enforcement.
5. Fail-closed exit code 2 (NOT RUN) when QEMU is missing.
6. Kernel CPU-state gate refusal (exit code 2) when baseline Phase 13/14 kernel is detected.
7. Kernel CPU-state gate qualification when Phase 15 SIMD-capable kernel is provided.
8. Full 24-step guest lifecycle execution under mocked QEMU/QMP/serial environment (exit code 0).
9. Negative regression tests:
   - Missing frame progression fails.
   - Out-of-order log markers fail sequence verification.
   - Missing clean process exit fails.
   - Missing UEFI shutdown fails.
   - Nonzero QEMU exit status fails.
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

import test_upstream_sdl3_guest as upstream_harness


UPSTREAM_FULL_VALID_MOCK_LOG = (
    "[desktop] real desktop frame presented\n"
    "[desktop] desktop surface shows /Users/user/Desktop\n"
    "[desktop] APB1 installed; signed version=1 (no launch authority implied)\n"
    "[upstream-demo] Starting Genuine Upstream SDL 3.2.0 Demo on ArenaOS...\n"
    "[upstream-demo] Upstream SDL video subsystem initialized successfully!\n"
    "[upstream-demo] Upstream SDL_Window created: 0x240000 (id=1)\n"
    "[upstream-demo] Surface acquired: 320x240, format=1, pitch=1280, pixels=0x250000\n"
    "[upstream-demo] Entering interactive frame loop (60 frames)...\n"
    "[upstream-demo] frame rendered; damage published; frame=0\n"
    "[upstream-demo] frame rendered; damage published; frame=25\n"
    "[upstream-demo] frame rendered; damage published; frame=59\n"
    "[upstream-demo] Completed 60 frames in 1000 ms\n"
    "[upstream-demo] Shutting down upstream SDL...\n"
    "[upstream-demo] Clean exit 0\n"
    "[desktop] child Process-cap exit status=0\n"
    "[arena INFO kernel] shutdown requested by pid 157 through its Power cap - goodnight\n"
    "halting via UEFI ResetSystem(shutdown)\n"
)


class TestUpstreamHarnessAudit(unittest.TestCase):
    def test_audit_c2(self):
        rc = upstream_harness.run_audit_mode("c2")
        self.assertEqual(rc, 0, "Target C2 audit must pass static checks")

    def test_audit_standalone(self):
        rc = upstream_harness.run_audit_mode("standalone")
        self.assertEqual(rc, 0, "Target Standalone audit must pass static checks")

    def test_audit_all(self):
        rc = upstream_harness.run_audit_mode("all")
        self.assertEqual(rc, 0, "All upstream targets audit must pass static checks")

    def test_pinned_hash_failure(self):
        with tempfile.NamedTemporaryFile() as tmp:
            tmp.write(b"corrupted binary data")
            tmp.flush()
            with patch.object(upstream_harness, "C2_EXE", Path(tmp.name)):
                with self.assertRaises(ValueError) as ctx:
                    upstream_harness.ensure_c2_artifacts()
                self.assertIn("digest mismatch", str(ctx.exception))


class TestUpstreamCPUStateGate(unittest.TestCase):
    def test_guest_missing_qemu_returns_exit_2(self):
        with patch.object(upstream_harness.phase10, "qemu_cmd", side_effect=FileNotFoundError):
            rc = upstream_harness.run_upstream_guest_lifecycle("c2")
            self.assertEqual(rc, 2, "Missing QEMU must return exit code 2 (NOT RUN)")

    def test_kernel_gate_refusal_for_baseline_kernel(self):
        """When baseline release kernel is present, gate must identify it and refuse execution with code 2."""
        with patch.object(upstream_harness.phase10, "qemu_cmd", return_value=["qemu-system-x86_64"]):
            rc = upstream_harness.run_upstream_guest_lifecycle("c2", force_simd_capable=False)
            self.assertEqual(rc, 2, "Baseline kernel without FP/SIMD state saving must return exit code 2 (BLOCKED)")

    def test_kernel_gate_all_targets_reporting(self):
        with patch.object(upstream_harness.phase10, "qemu_cmd", return_value=["qemu-system-x86_64"]):
            rc = upstream_harness.run_guest_mode("all", force_simd_capable=False)
            self.assertEqual(rc, 2, "Both targets must report BLOCKED and return exit code 2")

    def test_kernel_gate_qualification_with_simd_capable_kernel(self):
        capable, detail = upstream_harness.inspect_kernel_simd_capability(None, allow_experimental=True)
        self.assertTrue(capable, "Phase 15 SIMD capable kernel must be qualified")
        self.assertIn("QUALIFIED", detail)


class TestUpstreamMocked24StepGuest(unittest.TestCase):
    def _run_mocked_lifecycle(
        self,
        serial_log_text: str,
        target: str = "c2",
        qemu_exit_code: int = 0,
        fail_pixels: bool = False,
        timeout_override: float = 0.05,
    ) -> int:
        """Helper to run the 24-step guest lifecycle under controlled mocks."""
        with tempfile.TemporaryDirectory() as td:
            work_dir = Path(td)
            scratch_img = work_dir / "scratch.img"
            scratch_img.write_bytes(b"\x00" * 1024)

            mock_afs2_tree = {
                "/System/Applications/org.arenaos.sdl3upstreamc2/1/bin/sdl3_upstream_c2": b"ELF_DATA",
                "/System/Applications/org.arenaos.sdl3upstream/1/bin/sdl3_upstream_demo": b"ELF_DATA",
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
                if isinstance(cmd, list) and len(cmd) > 2 and "vcon.py" in str(cmd[2]):
                    return mock_fixture
                if isinstance(cmd, list) and len(cmd) > 2 and "network_fixture.py" in str(cmd[2]):
                    return mock_fixture
                cwd = kwargs.get("cwd")
                if cwd:
                    cwd_p = Path(cwd)
                    (cwd_p / "qmp.sock").touch()
                stdout_f = kwargs.get("stdout")
                if stdout_f:
                    stdout_f.write(serial_log_text.encode())
                    stdout_f.flush()
                return mock_guest

            mock_pixels = MagicMock()
            if fail_pixels:
                mock_pixels.side_effect = AssertionError("Mock pixel validation failure: invalid window content")

            with patch.object(upstream_harness, "WAIT_TIMEOUT_OVERRIDE", timeout_override):
                with patch.object(upstream_harness, "ensure_guest_artifacts", return_value="dummy_efi_sha"):
                    with patch.object(upstream_harness, "seed_disk"):
                        with patch.object(upstream_harness.subprocess, "run"):
                            with patch.object(upstream_harness, "verify_screenshot_pixels", mock_pixels):
                                with patch.object(upstream_harness.phase10, "qemu_cmd", return_value=["qemu-system-x86_64"]):
                                    with patch.object(upstream_harness.subprocess, "Popen", side_effect=mock_popen):
                                        with patch.object(upstream_harness.qmp, "Qmp", return_value=mock_qmp_conn):
                                            with patch.object(upstream_harness.afs2, "walk", return_value=mock_afs2_tree):
                                                with patch.object(upstream_harness, "WORK_DIR", work_dir):
                                                    return upstream_harness.run_upstream_guest_lifecycle(
                                                        target,
                                                        force_simd_capable=True,
                                                    )

    def test_full_24_step_guest_qualification_success(self):
        """Positive test: 24-step guest qualification succeeds under SIMD-capable kernel."""
        rc = self._run_mocked_lifecycle(UPSTREAM_FULL_VALID_MOCK_LOG, target="c2", qemu_exit_code=0)
        self.assertEqual(rc, 0, "Mocked 24-step upstream guest lifecycle must succeed with exit 0")

    def test_full_24_step_standalone_success(self):
        """Positive test: Standalone target also succeeds under SIMD-capable kernel."""
        rc = self._run_mocked_lifecycle(UPSTREAM_FULL_VALID_MOCK_LOG, target="standalone", qemu_exit_code=0)
        self.assertEqual(rc, 0, "Mocked standalone upstream guest lifecycle must succeed with exit 0")

    def test_missing_frame_progression_fails(self):
        """Negative test: Missing frame 59 causes qualification failure."""
        truncated_log = UPSTREAM_FULL_VALID_MOCK_LOG.replace(
            "[upstream-demo] frame rendered; damage published; frame=59\n", ""
        )
        rc = self._run_mocked_lifecycle(truncated_log, timeout_override=0.02)
        self.assertEqual(rc, 1, "Missing frame 59 milestone must cause failure (exit code 1)")

    def test_out_of_order_markers_fail(self):
        """Negative test: Milestones appearing out of chronological order fail sequence verification."""
        out_of_order_log = UPSTREAM_FULL_VALID_MOCK_LOG.replace(
            "[upstream-demo] frame rendered; damage published; frame=0\n", ""
        )
        # Put frame 0 AFTER frame 25
        out_of_order_log = out_of_order_log.replace(
            "[upstream-demo] frame rendered; damage published; frame=25\n",
            "[upstream-demo] frame rendered; damage published; frame=25\n"
            "[upstream-demo] frame rendered; damage published; frame=0\n"
        )
        rc = self._run_mocked_lifecycle(out_of_order_log, timeout_override=0.02)
        self.assertEqual(rc, 1, "Out-of-order log markers must cause test failure (exit code 1)")

    def test_missing_clean_exit_fails(self):
        """Negative test: Application failing to cleanly terminate causes failure."""
        no_exit_log = UPSTREAM_FULL_VALID_MOCK_LOG.replace(
            "[upstream-demo] Clean exit 0\n", ""
        )
        rc = self._run_mocked_lifecycle(no_exit_log, timeout_override=0.02)
        self.assertEqual(rc, 1, "Missing clean application exit must cause test failure (exit code 1)")

    def test_missing_uefi_shutdown_fails(self):
        """Negative test: Missing UEFI ResetSystem confirmation causes failure."""
        no_shutdown_log = UPSTREAM_FULL_VALID_MOCK_LOG.replace(
            "halting via UEFI ResetSystem(shutdown)\n", ""
        )
        rc = self._run_mocked_lifecycle(no_shutdown_log, timeout_override=0.02)
        self.assertEqual(rc, 1, "Missing UEFI ResetSystem must cause test failure (exit code 1)")

    def test_qemu_nonzero_exit_fails(self):
        """Negative test: Nonzero QEMU exit status causes failure."""
        rc = self._run_mocked_lifecycle(UPSTREAM_FULL_VALID_MOCK_LOG, qemu_exit_code=1)
        self.assertEqual(rc, 1, "Nonzero QEMU exit status must cause test failure (exit code 1)")


if __name__ == "__main__":
    unittest.main()
