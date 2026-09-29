# ADR-0040 — Bounded manager restart and a privileged, real-wire exercise

*Status: accepted for the next **partial** 8.0 checkpoint. Full 8.0 and the Phase 7 production-supervision closure remain open pending repeated fault/negative-space/accounting proofs.*

## Context

ADR-0037 gives the ring-3 manager sole lifecycle ownership of production
`netstackd`; ADR-0039 starts it and retains a Process cap, but parks after
its first exit. A badge identifies a possible exit, not the child or the
authority to reap. Blindly respawning leaks the bounded process and spawn
record tables. A root-only test instance (M7) is not a production restart
proof. The shell already holds `Power`, so it is a privileged administrator,
not an untrusted app. No ordinary user process is granted the production
stack's call endpoint yet.

## Decision

On the **matched** child-exit badge, the manager rechecks its unique held
Process cap and calls `SYS_PROC_FINISH(slot, 0)`; the syscall must refuse a
still-running child. On success it arms a monotonic one-shot backoff timer
on a **new manager-private notification** with a distinct bit before
any new spawn. This is an authority boundary, not just a badge namespace:
`netd` holds WRITE to the original manager event notification to signal
readiness and could forge *any* badge bit there, skipping backoff. The
stack and rngd also hold WRITE on their own notifications. None may
possess WRITE on this private timer notification. The kernel's fixed
root grants it at manager slot 7 only when both dependencies exist; the
manager checks its object is distinct from all three existing channels.
The full fixture's notification budget rises explicitly 9 → 10 and
the kernel checks an eleventh allocation is refused. The manager's
Process handle consequently lands in a later slot; it still discovers
it via the caller-cap query, NEVER by assuming a slot. A bounded counter
(from the validated manifest, at most three) refuses an unbounded failure
loop. Before each
attempt it resolves the fixed plan again against actual caller-held caps;
then only `SYS_SPAWN` attenuates and installs the four child grants. The
new child must pass its bounded startup-ready gate before READY can be
reported. A failed finish, timer, plan, spawn or ready check is explicit
OFFLINE, never silently adopted by the kernel driver supervisor. The
unchanged endpoint object survives a child's death and reaping.
A `STATUS_BUSY` from finish can mean either a still-live child or a
failed teardown under the current ABI. Treat it as an ignorable forged
exit hint ONLY if a fresh diagnostic `SYS_PROC_LIST` snapshot confirms
this exact held child still has live threads; if dead, absent, or the
snapshot fails, report OFFLINE rather than swallowing a failed reap.
The diagnostic pid snapshot never authorizes a lifecycle operation.

For this partial checkpoint, a **privileged shell** (already holding
`Power`) may receive just the stack's client `Endpoint/WRITE` cap, and
only when BOTH driver dependencies are attached. It receives NO Process,
MMIO, driver, or manager-event authority. Its explicit `stacktest`
command issues real ARP wire work, binds and holds a service-issued UDP
bearer, requests the existing stack protocol's orderly `SHUTDOWN`,
observes a response from the same endpoint after the manager restarts,
checks that the old bearer is refused (even though the endpoint survives),
and proves a new ARP request traverses the real wire. The test invokes
this command through ordinary serial console input; normal boots do
**not** auto-kill the production service. Host QEMU test checks distinct
pids, one Process-cap reap, timer backoff, two separate ready handshakes,
stale-bearer refusal and real post-restart wire evidence. Earlier M1–M7
and missing-device boots remain intact. A pure no_std state policy gets
host refusal/budget tests. The historical suite and a fresh artifact-
bound 100/100 gate qualify the resulting image, which is bundled in the
**same** commit as its code under a new checkpoint directory.

A privileged shell with stack `WRITE` can send *any* stack operation,
including the existing test `SHUTDOWN`: this does NOT make the stack
endpoint suitable for granting to arbitrary applications. Before any
unprivileged production client is granted that endpoint, split or gate
the control operation with structural authority (or remove it) under a
separate ADR and proof. A random UDP handle is bearer authority, not an
identity or a kernel capability; old handles are invalid because the new
stack loses its in-memory state, not because its client pid changed.

This single orderly-exit scenario does **not** prove forced live-stop,
foreign/self/driver refusal, repeated restart accounting, budget
exhaustion under multiple failures, or an unexpected in-flight-crash
recovery. It must not emit `m8: RESULT PASS` or close the Phase 7
supervision obligation. Driver liveness is rechecked indirectly by the
new child's own actual setup and ready signal; separately probing each
external driver BEFORE respawn remains required for complete 8.0.

## Rejected shortcuts

- Letting the kernel supervisor restart `netstackd` as a fallback: two
  lifecycle owners, contrary to ADR-0037.
- Inferring a child from a pid, forged badge, manifest key or log line:
  none conveys a Process cap or a grant.
- Reusing the manager event, rngd-ready or stack backoff notification
  for restart timing: their other WRITE holders can forge a timer badge.
  A separate object is worth one more explicitly audited table slot.
- A built-in automatic production restart test every boot: it consumes
  the finite restart budget and destroys real stack state on unattended
  machines. The privileged console command is explicit and opt-in.
- Claiming one controlled exit proves crash, forced-stop, repeated flat
  accounting and all lifecycle negative-space: those are separate gates.
