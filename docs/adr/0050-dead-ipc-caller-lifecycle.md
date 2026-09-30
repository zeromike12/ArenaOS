# ADR-0050 — Sweep dead IPC callers before Process-cap teardown

*Status: Accepted (2026-09-30). Bounded internal correction to ADR-0028/0049 process teardown; no new public ABI, primitive, authority or 8.2 exit criterion. Design acceptance is not a test or phase-completion claim.*

## Failure observed, not a hypothetical race

In the real full-device guest, a marker-authorized diagnostic PING deliberately blocked inside `permissiond`. The manager's existing private timer fired; it correctly stopped the blocked PING worker via its held Process/DESTROY cap, then stopped the broker via its held Process/DESTROY cap. QEMU exited 0 only because the kernel **halted**:

```
proc_finish mode1: owner 91 target 100 live_threads=1 held Process/DESTROY
servicemgr: permission PING failed/deadline; no READY
proc_finish mode1: owner 91 target 95 live_threads=1 held Process/DESTROY
[arena ERROR ipc] fail_calls_for_server: wake(caller 196) failed: wake: thread is not blocked
[arena ERROR halt] halting machine: ipc: a failed caller could not be woken
```

See `build/serial-m82-probe-stall.log` (development log, not a qualified artifact). The rejected PING fixture (wrong typed reply, no success badge) already terminated without a halt. The stalled case reveals an actual kernel invariant breach: `ipc::release_blocked_of` removes a dying process's **server** and Notification waiter thread references, but not its IPC **caller** references in endpoint `Waiting`/`Delivered` queue slots. The worker dies; the queue still names its dead thread; later `ipc::fail_calls_for_server` treats that stale id as wakeable and halts. `SYS_PROC_FINISH` must not leave a reference to a removed caller.

## Options and proposed correction

1. **Recommended:** before killing/reaping the worker's threads during `proc::destroy`, sweep every live endpoint queue slot whose `caller` belongs to that process. Clear the whole slot regardless of `Waiting`, `Delivered`, `Failed`, or `Replied` state (no reply cap may escape to another caller). Also clear any parked server/notification references as today. Do this at the same IF=0/kernel reference-release boundary already required by ADR-0028; use the real scheduler caller→pid relation, not a userspace-provided id. A server whose Delivered caller disappears receives a typed reply refusal and continues serving, not a kernel halt. Keep the bound `QUEUE_DEPTH * MAX_ENDPOINTS`, no dynamic allocations or new IPC syscall. Count/verify released caller slots in a source-reviewed and live-process test. Ensure service-death sweep never attempts to wake a dead caller, even in mixed caller/server teardown order.
2. As an immediate manager ordering invariant, stop the blocked **broker before its blocked PING worker** on timeout. The broker's failure sweep wakes the still-live worker with typed `STATUS_SERVICE_GONE`; then the manager can reap/stop it using its Process cap. This preserves the best typed failure semantics, but **cannot substitute for option 1**: other authorized lifecycle actors can kill callers before servers, and a stale kernel queue reference would remain a machine-wide hazard.
3. Ignore `sched::wake` errors during broker teardown. Rejected: conceals stale kernel references and does not reclaim the occupied call slot or queued reply cap.

The decision is options **1 and 2 together**. Audit no accidental reply to a later caller when a slot is recycled. Guest proofs must exercise worker-first teardown independently (no halt, queue reclaimed, live server still works), broker-first timeout teardown (typed failure, two Process handles reaped, no false READY), and all prior managed-fault/restart/IPC suites. Do not count QEMU `rc=0` as a successful guest boot without also rejecting `[arena ERROR halt]` and checking the expected completion marker. No full-suite/100-of-100 qualification or release should follow until this is fixed and tested.

## Implemented invariant and focused falsification

`proc::destroy()` calls `ipc::release_blocked_of(pid)` before killing the process's threads. The sweep removes every live endpoint call slot whose caller's scheduler-owned process ID is `pid` in `Waiting`, `Delivered`, `Failed`, and `Replied`. Clearing a slot drops its queued request/reply and any staged capability; the endpoint, other callers and its server survive. Existing server-death handling still returns `STATUS_SERVICE_GONE` to a live caller. `SYS_IPC_REPLY` to an abandoned delivered slot returns typed `STATUS_BAD_ARG`; no stale thread is awakened. The manager's normal timeout stops the broker first for typed failure; its separate diagnostic deliberately kills the blocked `SYS_IPC_CALL` worker first, stops/restarts the broker, and verifies its original endpoint still serves fresh requests. These two orderings are complementary, not substitutes.

`kernel/kernel/src/ipc_adr50_test.rs` is a deterministic in-guest M4 kernel fixture with a real live-process thread ID: it seeds each of the four call states (including staged transfers), destroys that caller process, checks every slot empty with no staged cap, refuses a late reply with `STATUS_BAD_ARG`, reuses the endpoint, and checks exact frame reclamation. It emits `ADR-0050 four abandoned caller states cleared; staged caps discarded; late server reply typed STATUS_BAD_ARG; endpoint recycled; frames exact`. The real ring-3 blocked caller diagnostic is in `tools/test_m82_ipc_caller.py`: three guest modes distinguish caller-first, broker-first deadline, and wrong-PING-result refusal, each requiring historical M3/M4 PASS, orderly shutdown, failed-PING/no false READY, endpoint reuse and exact before/after resource counters. Its host semantic-verdict self-test proves QEMU rc=0 with an `[arena ERROR halt]` marker or missing M4 PASS must fail. The shared `tools/mtest.py` QEMU helpers and `tools/stability_loop.sh` also reject fatal markers even with rc=0.

The focused three-mode run and cap-layout/capspace checks passed on the integration image (development logs `build/adr50-focused-verdict.log` and `build/adr50-capspace-baseline.log`). The subsequently rebuilt integration EFI passed **35/35** fresh historical suites and an EFI-hash-bound **100/100** ordinary-boot qualification; the extracted checkpoint archive booted independently (`phase82-volatile-permission`, EFI SHA-256 `98331c626c4febed721682fdddc6ee9c3df5a7aa48067cc9690b8e64620bbc15`). These are evidence for the IPC invariant and the **partial volatile checkpoint**, not 8.2 persistence, crash, restart/ordering proofs or phase completion.

## 8.2 restart handoff addendum (2026-09-30, accepted internal IPC correction)

The persistent broker's first real restart exposed a separate semantic gap:
`fail_calls_for_server` marks the held endpoint orphaned on server death;
`ipc::recv` reopens it, but a bounded PING worker can CALL **before**
the newly spawned broker has completed disk recovery and entered RECV.
It gets immediate `STATUS_SERVICE_GONE`, falsely failing startup despite
an existing replacement process. Busy retry of that status did not fix
the structural sequencing: a running probe consumed its whole 1.5-second
budget before the newly queued broker entered RECV. Do not fix this by
lengthening the spin or ignoring an invalid reply.

After `SYS_SPAWN` has successfully created a *new live thread* with an
attenuated Endpoint/READ inheritance, reactivate ONLY that endpoint's
orphan flag. The endpoint is still the exact same object held by the
manager; a WRITE-only client or an uncommitted/rolled-back spawn cannot
reactivate it. Existing post-death calls before that successful spawn
still get `STATUS_SERVICE_GONE`; once the new serve-cap holder exists,
calls may queue until its first RECV, like ordinary initial boot. If the
new process dies during recovery, its process-teardown failure sweep
wakes those queued callers with typed `STATUS_SERVICE_GONE`; the manager's
existing bounded worker/deadline remains the readiness witness. This
requires no syscall, additional grant, new notification, or trust-model
change. Remove the speculative worker retry; test both death gap and
replacement startup with the original endpoint and actual ALLOW/DENY
rehydration. Reopening at mere COPY of a cap or while a spawn can still
roll back would be unsound and is explicitly rejected.
