#!/usr/bin/env python3
"""Milestone 3 automated boot test (docs/TESTING.md, ADR-0005).

Pipeline and verdict logic live in tools/mtest.py. M3's contract grows with
each roadmap step; the same boot must also still carry every M1/M2 marker
(test_m1.py / test_m2.py check those against the same image).

Current coverage:
  M3.1 — kernel threads + context switch (ADR-0012; all five markers are
  emitted by kmain after the m2 RESULT line):
  * thread_spawn_run        — a spawned thread runs its entry exactly once
                              on its own stack, its side effects (magic +
                              arg delivery) verify, it exits and is reaped;
                              ≥ one there-and-back context switch observed.
  * thread_rr_interleave    — three threads × four rounds log (id, round)
                              into a global log; the *exact* round-robin
                              order 1,2,3 × 4 is asserted entry by entry
                              (determinism, not "some interleaving").
  * thread_callee_saved     — one asm block writes distinctive constants
                              into rbx/rsi/rdi/r12–r15, calls yield (a
                              real switch away and back), and reads the
                              same physical registers: all seven must
                              match (rbp: mechanical push/pop symmetry,
                              documented in the test + ADR-0012).
  * thread_stack_isolation  — two threads keep distinct 64-qword stack
                              patterns intact across interleaved yields,
                              report disjoint kernel-view stack ranges,
                              and return a correct depth-200 recursion
                              (checked against an iterative model).
  * thread_churn_accounting — 8 rounds × 8 threads created/run/exited/
                              reaped with *exact* frame-allocator and
                              heap-reservation accounting back to the
                              recorded baseline, plus the MAX_THREADS
                              table-full boundary (clean refusal, then
                              full drain and exact accounting again).
  M3.2 — timer-driven preemption (ADR-0013; the vector-32 PIT tick stub
  runs the scheduler when a quantum expires):
  * preempt_rotation        — three threads that contain NO yield call
                              anywhere are rotated in *exact* round-robin
                              order (bootstrap→1→2→3 cycle, ≥12 timer
                              switches, entry-by-entry log comparison)
                              and all three make progress — the direct
                              ROADMAP 3.2 proof ("interleaving that
                              proves preemption is happening").
  * preempt_coexist         — two threads doing 15 ms busy-waits (longer
                              than the 10 ms quantum, so they are rotated
                              out mid-wait) interleaved with cooperative
                              yields: both finish all rounds, the waits
                              never trip the anti-hang guard, and timer
                              rotations are observed alongside yields —
                              cooperative and preemptive triggers compose.

Exit code: 0 = PASS, 1 = FAIL (with the serial tail printed for diagnosis).
"""

import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import mtest  # noqa: E402

EXPECTED_TESTS = [
    "thread_spawn_run",
    "thread_rr_interleave",
    "thread_callee_saved",
    "thread_stack_isolation",
    "thread_churn_accounting",
    "preempt_rotation",
    "preempt_coexist",
]

if __name__ == "__main__":
    sys.exit(mtest.run_milestone("m3", EXPECTED_TESTS))
