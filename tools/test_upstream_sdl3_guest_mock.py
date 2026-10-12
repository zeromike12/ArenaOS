#!/usr/bin/env python3
"""Unit test suite for the dedicated Upstream SDL 3.2.0 Guest Harness & CPU-State Gate."""

from __future__ import annotations

import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import MagicMock, patch

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / "tools"))

import test_upstream_sdl3_guest as upstream_harness


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
            rc = upstream_harness.run_guest_gate("c2")
            self.assertEqual(rc, 2, "Missing QEMU must return exit code 2 (NOT RUN)")

    def test_guest_cpu_state_gate_refusal_when_qemu_present(self):
        """When QEMU is present, the strict CPU-state gate must refuse execution and return code 2."""
        with patch.object(upstream_harness.phase10, "qemu_cmd", return_value=["qemu-system-x86_64"]):
            rc = upstream_harness.run_guest_gate("c2")
            self.assertEqual(rc, 2, "Upstream SDL3 guest execution must be strictly refused with code 2 (BLOCKED)")

    def test_guest_cpu_state_gate_all_targets(self):
        with patch.object(upstream_harness.phase10, "qemu_cmd", return_value=["qemu-system-x86_64"]):
            rc = upstream_harness.run_guest_gate("all")
            self.assertEqual(rc, 2, "Target all in guest mode must return exit code 2 (BLOCKED)")


if __name__ == "__main__":
    unittest.main()
