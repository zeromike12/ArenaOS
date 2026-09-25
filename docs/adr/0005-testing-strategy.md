# ADR-0005: Testing strategy — automated QEMU boot tests with machine-checkable serial markers

Status: Accepted
Date: 2026-09-25
Milestone context: Phase 0 — Architecture, delivered in Milestone 1

## Problem

Kernel code fails catastrophically and silently: triple faults, hangs, and
wrong-value bugs give no diagnostics by default. We need a testing
architecture that (a) proves subsystems actually work — never fake output —
(b) runs automatically on every change, and (c) stays deterministic enough
that a red test means a real regression.

## Options considered

**Manual "boot it and look at the screen":** not repeatable, not checkable,
rots immediately. Rejected.

**QEMU + GDB single-stepping as primary loop:** excellent for deep debugging,
too slow and human-bound as the regression mechanism. Kept as a debugging
tool, not a test harness.

**Host-side unit tests only:** impossible for hardware-touching code; fine for
pure logic. Necessary but insufficient.

**Chosen — layered:**
1. **Boot integration tests** (`tools/test_*.py`): build the image, boot it in
   QEMU/EDK2 headless, capture serial, and assert on a **marker grammar** the
   kernel emits; assert the VM terminates cleanly via the kernel's own
   `ResetSystem()` shutdown within a timeout. Markers:
   `m<N>:test:<name>: PASS|FAIL (<reason>)` and
   `m<N>: RESULT PASS (p/t)` / `m<N>: RESULT FAIL (p/t)`.
2. **Host-side unit tests** for architecture-independent logic (bitmap
   allocators, ring buffers, parsers) — `cargo test --target <host>` once such
   crates exist (M2+).
3. **In-kernel self-tests** at boot for hardware behaviors that can only be
   checked on the metal (UART loopback, GDT load read-back via `sgdt`,
   `cpuid`, 128-bit arithmetic through `compiler_builtins`, memory-map
   parsing against firmware).
4. **Panic diagnostics:** the panic handler prints location + message over
   serial before halting; any `PANIC` marker in serial output fails the test
   even if later output looks clean.
5. **Regression rule:** every milestone's test script stays in `tools/` and is
   re-run by `tools/run_tests.sh` on every change. "When this milestone works,
   it must remain working permanently" is enforced mechanically, not by
   memory.

## Decision

Every milestone exits only when its automated boot test passes from a clean
build, and all previous milestone tests still pass. Tests assert on
*observable machine effects* (serial bytes produced by hardware register
round-trips, firmware data parsed from real UEFI structures, clean VM
termination), never on "we printed that initialization succeeded".

## Reasoning

- Deterministic environment: QEMU/TCG + fixed EDK2 blob + fixed memory size
  gives bit-reproducible boots; no wall-clock assertions in tests.
- Clean-exit discipline (UEFI `ResetSystem`, `-no-reboot`) distinguishes
  "kernel halted correctly" from "kernel hung" (timeout) from "kernel
  crashed" (marker/panic check) — three failure modes, three different
  diagnoses, automatically.
- The marker grammar is trivially greppable, human-readable on failure, and
  extensible to every future milestone.

## Downsides accepted

- Serial-marker tests are coarser than in-guest test frameworks; some
  properties (scheduler fairness) will later need in-kernel test hooks that
  report through markers — accepted, same grammar.
- QEMU-only testing can hide real-hardware timing/firmware bugs until Phase 8
  hardening. Accepted and tracked in RISKS.md.
- Test infrastructure is Python in `tools/` — a second language in the repo.
  Accepted: it is dev tooling, explicitly outside the OS image (ADR-0004).

## Future implications

- M2 adds: exception-handling tests (deliberate #PF/#DE triggers), frame
  allocator stress via markers, paging tests (map/unmap/permissions).
- M3 adds: deterministic scheduler tests (thread interleaving proofs printed
  over serial).
- M4 adds: userspace process tests (a real ring-3 binary executes and exits
  via syscall — the marker proves the privilege boundary was crossed).
- Later: KVM acceleration when available, fuzzing of parsers (boot-info,
  filesystem), and property tests for allocators on host.
