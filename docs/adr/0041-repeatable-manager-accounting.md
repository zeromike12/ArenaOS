# ADR-0041 — Repeatable production restart accounting and finite-budget proof

*Status: accepted for a partial Phase 8.0 checkpoint. This does not close Phase 8.0 or the Phase 7 production-stack supervision obligation.*

## Context

ADR-0040 proves one orderly production restart, but `MAX_SPAWN_RECS`, the
process table and free physical frames are bounded. If the manager fails
to release even one child per restart, a system that passes once will run
out of resources. A host log saying "reaped" is not a measurement of
kernel state. The three-attempt restart budget must be exercised on real
processes, not just the pure host-side state machine. Normal boots must
still start one resident stack without intentionally killing it.

## Decision

Provide a narrow, read-only `SYS_RESOURCE_SNAPSHOT(power_slot, out)` ABI:
three `u64`s containing free physical frames, occupied spawn records and
occupied process slots, sampled under the kernel's interrupt-disabled
critical section. The caller must possess an actual `Power/WRITE` cap;
invalid, wrong-kind and unowned slots fail before any user-memory write,
and invalid output pointers fail without leaking counts. This is an
administrator's diagnostic observation, not permission to reap, mint a
cap or allocate a frame. The existing kernel state tables remain the
single source of truth. Do not infer frame accounting from pids, service
logs or the planner's request list. The shell's existing Power grant
permits it to use the diagnostic; the stack client Endpoint grant must
NOT. ABI number 31 is additive (ADR-0017), never repurposed.

Add an explicit opt-in `stackstress` to the Power-holding shell. First
synchronize with the filesystem service (so late mount I/O cannot
masquerade as a leak), then take a kernel snapshot with the stack READY.
Run the existing *production* `stacktest` wire/bearer/exit/restart cycle
three times on the same kernel endpoint, checking frame, spawn-record
and process counts byte-exact against the baseline after each fully
READY replacement. Check that a forged diagnostic call using the
endpoint cap and a bad pointer are refused. Then request a fourth
orderly exit; the manager's validated three-restart budget must mark
OFFLINE without spawning a fifth child. Count actual ready pids and
kernel child-cap audits independently in a dedicated QEMU host test.
Do not automatically run this destructive exercise on normal boots or
on a machine missing a network or entropy dependency. Do not make
retrying a failure an acceptance strategy; investigate its mechanism.

Each source checkpoint is committed **with** its own downloadable,
qualified QEMU bundle. Preserve all historical tests, run the complete
suite and a fresh final-EFI-bound 100/100 fixture qualification, then
extract and boot the archive before the single source+bundle commit.
The ordinary 100-boot loop may continue to exercise the one-restart
path; the new dedicated integration fixture exercises the full budget.

## Limits and rejected shortcuts

- This exercises repeated **orderly** exits, not an unexpected crash or
  a live forced stop. It does not prove forged/foreign/self/driver
  Process-cap refusal, in-flight request failure, or independent driver
  liveness probing. `m8: RESULT PASS` remains forbidden.
- Counting shell `ps` output alone cannot prove frame or spawn-record
  stability; exposing the raw kernel tables or a global pid-kill API
  would widen authority. The diagnostic returns only three scalar
  counts and requires the already privileged Power capability.
- A test that silently raises the manager's restart budget to continue
  after three failures would test a different policy, not the bounded
  manifest. The fourth exit must leave it observably OFFLINE.
