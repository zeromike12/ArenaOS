#!/usr/bin/env python3
"""Milestone 1 automated boot test (docs/TESTING.md, ADR-0005).

Pipeline and verdict logic live in tools/mtest.py (shared by every
milestone harness). M1's contract: the kernel boots, passes all eight
M1 self-tests, and shuts the machine down cleanly via UEFI ResetSystem.

Exit code: 0 = PASS, 1 = FAIL (with the serial tail printed for diagnosis).
"""

import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import mtest  # noqa: E402

EXPECTED_TESTS = [
    "serial_loopback",
    "cpu_long_mode",
    "cpuid_sane",
    "gdt_installed",
    "idt_installed",
    "memory_map",
    "wide_math",
    "timer_irq_absorbed",
]

if __name__ == "__main__":
    sys.exit(mtest.run_milestone("m1", EXPECTED_TESTS))
