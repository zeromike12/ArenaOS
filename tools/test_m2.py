#!/usr/bin/env python3
"""Milestone 2 automated boot test (docs/TESTING.md, ADR-0005).

Pipeline and verdict logic live in tools/mtest.py. M2's contract grows with
each roadmap step; the same boot must also still carry every M1 marker
(test_m1.py checks that against the same image).

Current coverage (M2.1 — interrupts & exceptions):
  * tss_installed     — TR holds our TSS selector; IST1 points at the
                        dedicated fault stack (read back from live state).
  * exc_de_recovered  — a real divide-by-zero is delivered through our IDT,
                        recorded, and recovered from (execution continues).
  * exc_pf_recovered  — a real write to unmapped memory delivers #PF with
                        CR2 = the faulting address and error code
                        not-present+write, and is recovered from.

Exit code: 0 = PASS, 1 = FAIL (with the serial tail printed for diagnosis).
"""

import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import mtest  # noqa: E402

EXPECTED_TESTS = [
    "tss_installed",
    "exc_de_recovered",
    "exc_pf_recovered",
]

if __name__ == "__main__":
    sys.exit(mtest.run_milestone("m2", EXPECTED_TESTS))
