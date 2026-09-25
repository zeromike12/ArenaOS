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
  M3.3 — ring-3 boundary (ADR-0014; hand-assembled user payloads execute
  at CPL 3 and cross via the syscall MSRs; QEMU recipe gains +smep,+smap
  so the enforcement paths are exercised, not just compiled):
  * user_ring3_syscall      — GDT ring-3 pair + STAR/LSTAR/SFMASK/EFER.SCE
                              read-backs; a payload whose privileged `cli`
                              faults #GP and resumes (impossible at CPL 0),
                              whose invalid syscall number is rejected with
                              -1 *observed in ring 3*, whose SYS_WRITE
                              bytes are verified in a kernel-side buffer,
                              and whose SYS_EXIT(42) reaps through the
                              scheduler; SMAP: bare kernel read of a user
                              page → recovered #PF with CR2 asserted;
                              TSS RSP0 names the user thread's kernel
                              stack top; user-page map/run/unmap/free with
                              exact frame accounting.
  * user_ring3_interrupted  — a ring-3 spin loop (flag-gated, anti-hang
                              bounded) runs under armed preemption: timer
                              ticks land in user mode through RSP0, the
                              tick hook rotates the user thread mid-spin
                              and resumes it back into ring 3, a kernel
                              write into the process's own data page
                              releases the spin, and SYS_WRITE+SYS_EXIT(7)
                              verify afterwards — interrupts, preemption,
                              and shared memory all compose with ring 3.
  M3.3b — process address spaces (ADR-0014; proc.rs: a process = owned
  PML4 with private user half + cloned kernel half):
  * process_address_spaces  — two processes map the same user VA over
                              distinct frames; a write under procA's CR3
                              is invisible under procB's and persists in
                              procA's; kernel-half .data reads identical
                              under both CR3s; an unmapped user VA under a
                              process CR3 gives a recovered #PF with CR2
                              asserted; destroy reclaims every frame.
  * process_accounting      — 8 create/map/destroy churn rounds plus a
                              MAX_PROCESSES full-table refusal reclaim
                              every frame exactly; payload A runs at
                              ring 3 INSIDE a process address space
                              (user pages only in the process PML4,
                              thread cr3 = process root, syscalls on the
                              cloned kernel half, CR3 restored to the
                              kernel view on the post-exit switch).

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
    "user_ring3_syscall",
    "user_ring3_interrupted",
    "process_address_spaces",
    "process_accounting",
]

if __name__ == "__main__":
    sys.exit(mtest.run_milestone("m3", EXPECTED_TESTS))
