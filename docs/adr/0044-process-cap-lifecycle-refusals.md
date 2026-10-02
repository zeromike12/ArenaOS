# ADR-0044 — Exercise lifecycle refusal with real held caps, not pid names

*Status: accepted and qualified. The previously open active-dependency-probe area is closed by ADR-0045 and the full Phase 8.0 qualification.*

## Context

The manager now reaps dead children and force-stops a live production
child through a held Process/DESTROY cap. Shell tests already refuse
wrong-kind and stale slots, but have not passed real adversarial Process
caps to the kernel for its self, manager and kernel-supervised-driver
protections. Merely passing a guessed pid as a slot or matching log text
is not a convincing test of the rights and target guards.

**Authority is not parent identity.** A foreign child with a legitimately
transferred Process/DESTROY cap *is* an authorized target, provided it is
not protected by the kernel. Do not add a pid-name/parent check to deny
an intentional delegation. "Foreign refusal" means a client without
DESTROY, even if it knows the child's pid or holds a descriptive
read-only Process reference. A valid Process/DESTROY cap is not forgeable
from that information. Self and kernel-owned manager/driver processes
are protected independently of possession, since `SYS_PROC_FINISH`
requires a non-self, non-supervised target with a live *user-child*
spawn record. The first live refusal test exposed a real defect:
kernel-started `spawn_init` processes also have spawn records, so a
literal manager Process/DESTROY reference could stop the manager.
Before implementing the repair, we choose to record creation provenance
in each spawn record: `spawn_from` creates a user child, `spawn_init` a
kernel-owned root. `SYS_PROC_FINISH` must require a user-child record,
not just any record; this does not check the caller's parent identity,
and intentional delegation of a user child's Process/DESTROY cap remains
valid. Kernel-owned root teardown stays in the kernel supervisor, never
in user `SYS_PROC_FINISH`. This structural guard applies to the manager
and all other kernel-bootstrapped services, including ones not currently
supervised.

## Decision

The trusted boot root (not the manager, a manifest or a new mint syscall)
issues four **literal, non-COPY** Process/READ|DESTROY references to the
existing Power-holding administrator, only when both production drivers
exist: shell SELF, manager (kernel-started, with a kernel-owned
spawn record), netd and rngd (kernel-supervised). The administrator already holds Power
and the production stack control endpoint; no ordinary app gains a
Process cap, and neither driver nor manager receives additional grants.
A fifth slot starts as an inert, self-naming Process/READ placeholder
before the shell can run. After the boot root observes an actually
audited manager-owned stack child, it checks and consumes precisely
that placeholder and issues Process/READ for the child. On replacement
it checks the old reference, retires it and issues a new read-only
reference, never DESTROY. Exactly named slots 10–14 avoid the shell's
slot 7 transient spawn handle, filesystem slots 8/9 and slot 15's
unheld diagnostic. Abort boot on an unexpected reserved cap or failed
literal issuance; never overwrite a user-owned cap or synthesize a
Process/DESTROY reference to the managed child. All refs are
descriptions, not lifecycle grant requests. Issuing them never changes
the manager's authority, policy or restart behavior.

An opt-in `lifetest` on the privileged shell describes the *actual*
issued refs and checks both `SYS_PROC_FINISH` modes against each:
self/manager/netd/rngd must be refused despite a held DESTROY right;
the live foreign manager-owned child must be refused because its cap
lacks DESTROY. A guessed pid, empty slot, Power and Endpoint are also
refused, and `SYS_CAP_COPY` cannot amplify the foreign READ-only cap to
DESTROY. The test spawns a normal short-lived shell child to exercise
mode-1 refusal on a dead child, a successful mode-0 Process-cap reap and
stale-handle refusal (both modes) after that exact slot is emptied.
It verifies the manager child and two drivers remain alive through the
same production endpoint with real wire work, and kernel free-frame,
spawn-record and process-slot counts return to baseline. A dedicated
QEMU host actor matches all named refs to independently boot-logged
live pids, sees no manager restart or driver death, and tests the
missing-device SKIP. This is a real syscall/cap-table proof; serial
markers only report its results.

The test is explicitly opt-in: regular boots do not kill, fault or
restart the production stack. The presence of these protected refs on
the already Power-holding administrator does not grant stop authority
for any target not already guarded by kernel policy; their lack of COPY
prevents passing them to ordinary applications. Granting a
Process/DESTROY reference to the manager-owned child would **not** be a
negative test — it would give the shell genuine lifecycle authority and
violate manager ownership. The kernel is forbidden to do so.

Each commit carries its own QEMU bundle after the complete historical
suite, fresh artifact-bound 100/100 qualification and extracted-archive
boot. The Phase 7 supervision obligation and Phase 8.0 are closed only after
ADR-0045's active probes and full, artifact-bound qualification pass.
