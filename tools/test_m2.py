#!/usr/bin/env python3
"""Milestone 2 automated boot test (docs/TESTING.md, ADR-0005).

Pipeline and verdict logic live in tools/mtest.py. M2's contract grows with
each roadmap step; the same boot must also still carry every M1 marker
(test_m1.py checks that against the same image).

Current coverage:
  M2.1 — interrupts & exceptions:
  * tss_installed     — TR holds our TSS selector; IST1 points at the
                        dedicated fault stack (read back from live state).
  * exc_de_recovered  — a real divide-by-zero is delivered through our IDT,
                        recorded, and recovered from (execution continues).
  * exc_pf_recovered  — a real write to unmapped memory delivers #PF with
                        CR2 = the faulting address and error code
                        not-present+write, and is recovered from.
  M2.2 — timers & monotonic clock:
  * pit_oneshot       — PIT ch0 one-shot completes; read-back status OUT
                        latches high after ~the programmed 10 ms.
  * tsc_frequency     — fresh PIT-vs-TSC calibration window agrees with the
                        boot-time calibration (<5%).
  * clock_monotonic   — clock never goes backwards; busy-wait honors its
                        request; µs scale cross-checked against the raw PIT
                        oscillator (<10%).
  * tick_rate         — 10 timer interrupts flow through PIT→IOAPIC→LAPIC→
                        IDT→absorb-stub at the programmed 100 Hz (bounds
                        generous for TCG).
  M2.3 — physical memory:
  * frame_allocator   — bitmap allocator manages exactly the clipped
                        conventional regions; unique in-region frames;
                        pattern round-trip through real RAM; exact free
                        accounting; double-free rejection; 16-frame
                        contiguous run; 200-round stress; ns/op benchmark
                        (ADR-0007 evidence).

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
    "pit_oneshot",
    "tsc_frequency",
    "clock_monotonic",
    "tick_rate",
    "frame_allocator",
]

if __name__ == "__main__":
    sys.exit(mtest.run_milestone("m2", EXPECTED_TESTS))
