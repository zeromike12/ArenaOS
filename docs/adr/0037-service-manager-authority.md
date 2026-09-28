# ADR-0037 — Service-manager authority, bootstrap and manifest v1

*Status: proposed, design gate for Phase 8.0; no implementation or 8.0 qualification yet.*

## Context: the existing mechanisms and the missing one

`SYS_SPAWN` (ADR-0019) already requires an `Image` cap with READ and
inherits only parent-held caps with COPY, refusing rights amplification.
The kernel's `supervise.rs` (ADR-0028) replays kernel-minted grants for
devices, notably MMIO; its idle-loop poll exists in production. A
ring-3 manager cannot mint MMIO, and a manifest string must not become
an alternate mint syscall. In the current tree the kernel still starts
production drivers itself; M7's `netstackd` is spawned by its *test*
and destroyed afterward, not registered as a resident supervised
service. The Phase 7 promise to supervise it remains OPEN.

There is a further lifecycle gap: `SYS_SPAWN` gives a parent a Process
cap and last-thread exit badge, but `proc::destroy` and `spawn::forget`
are kernel-only. A userspace manager that respawns without reaping will
exhaust process/spawn-record tables. We must not call a restart loop a
manager until the death/reap path and its authority are proven.

## Decision: two disjoint owners, not two managers of one service

- **Kernel bootstrap and driver supervisor** retain lifecycle of
  device-facing services that need kernel-minted grants (including
  `netd` and `rngd`). The kernel alone enumerates devices and creates
  MMIO windows. Production registration, not just a test registration,
  must be verified for any driver that 8.0 depends on. The kernel
  supervisor does NOT register a manager-owned service.
- **One ring-3 service manager** owns the initial spawn, bounded
  restart-on-exit, dependency ordering and offline state of explicitly
  listed ordinary services, beginning with `netstackd`. It must not
  manage `netd`, `rngd`, itself, or an arbitrary pid. There is no
  implicit fallback to kernel supervision of the same service. The
  manager is the only spawner for its manifest's instance; restart
  retains the same service endpoint rather than minting a new one.
- **The trusted bootstrap root** is the kernel's fixed boot policy:
  registered, built-in manager image and a literal, auditable initial
  cap list. It starts the existing device dependencies first; on a
  fixture with no network it records netstackd OFFLINE rather than
  manufacturing device authority. No mutable filesystem config,
  unauthenticated file or caller-provided manifest is a trust root.
  The manager is not self-supervised in v1: its failure is visible and
  leaves already-running children UNMANAGED and offers no restart until
  reboot, not secretly adopted by the driver supervisor. A failed
  manager is never reported as a healthy supervision service. Recovery of the manager itself
  requires a separate lifecycle design.

## Grants: requests are never authority

A versioned, bounded **built-in** manifest has a stable service id,
image id, dependencies, readiness probe, restart budget/backoff, and a
list of **requests** `(symbolic boot-cap key, expected kind,
requested rights, child slot)`. This is a policy *request*, not a cap.
The boot root separately chooses and installs the manager's actual
capabilities: only Image/READ|COPY for permitted service images,
endpoint and notification authorities required by those services,
and an exit notification; it gives the manager **no MMIO, Power,
process-root, or generic device-mint authority**. A server READ cap
and a client WRITE cap are distinct attenuated child grants even when
they name one endpoint. The manager can delegate only the actual caps
it possesses and only a subset of their rights; it may never ask a
privileged helper to create an arbitrary requested cap. A future
policy broker or device-cap issuance facility needs its own ADR and
threat model, not an unnoticed 8.0 escape hatch.

For each request, resolve the symbolic key against the *actual* boot
inventory; verify cap kind and rights (a narrow, caller-cap-only
query if the present ABI cannot prove this), reject duplicates,
unknown keys, wrong kind, absent dependencies, exceeded bounds or
rights not held. Compile an explicit inheritance list from the
resolved grants. `SYS_SPAWN` remains the atomic kernel enforcement of
COPY and attenuation at the moment of spawn; no truncating/clamping
rights and no retry with broader caps. Record **actually granted**
`(object/key, rights, child slot)` only after spawn succeeds and,
for the milestone test, independently audit the child's installed
caps against that exact list. A missing, mismatched or refused grant
leaves the service OFFLINE with a typed error and no partially running
child. The manifest cannot authorize access by name alone, and a
service cannot rely on an ungranted optional permission silently
becoming available. Manifest parsing does not execute authority.

`SYS_SPAWN` currently returns a pid and drops the Process cap into the
first free parent slot. The manager must get an unambiguous handle to
that cap (an explicit destination/result slot or equivalent checked
lookup); it must not guess a cap slot from a pid or cap-table order.

8.0 needs a narrow **Process-cap + DESTROY gated child stop/reap**
mechanism. For a dead child, it releases the process, its cap space
and spawn record exactly once; it refuses live children unless a
separate explicit stop operation is requested and authorized. Both
operations refuse kernel-supervised driver pids and the manager itself;
no pid-only, global kill or `proc::destroy(any_pid)` syscall. An exit
badge identifies a candidate, never grants authority to reap it:
the held Process cap does. Test stale/forged/reused caps, attempted
foreign/driver/manager reaps, and flat frame/record accounting over
multiple restarts. Hung-service detection and forced recovery are
out of scope until a separately tested timeout/stop policy exists.

## Dependencies, readiness and restart

Reject duplicate ids, absent required nodes and cycles *before any
child is started*. Topologically start dependencies; a successful
spawn is not READY. Each required dependency must complete a bounded,
explicit service-specific IPC/notification readiness check before its
dependent starts. A dead required dependency marks dependents
DEGRADED/OFFLINE; do not replay unknown-outcome non-idempotent calls
or silently widen their grants. On a managed child's exit, reap via
its Process cap, apply a monotonic-timer backoff and a small fixed
restart budget, then respawn the **same** manifest/grant plan behind
the **same** endpoint. Exhaustion leaves it OFFLINE, explicitly
observable. A restarted `netstackd` loses in-memory UDP/TCP handles;
clients must observe SERVICE_GONE/revoked state rather than a false
continuation. Preservation of sessions is not promised. Dependencies
are checked again before each restart.

The first production managed service is `netstackd`: it depends on
kernel-started `netd` for link access and on `rngd` for bearer-backed
operations. Its four existing grants (netd call endpoint, own serve
endpoint, backoff notification, RNG call endpoint) fit today's
`MAX_INHERIT=4`; a readiness channel requiring a fifth grant must
raise and test that bound explicitly, not silently omit a grant.
Proof must include killing the production stack while a real client
holds its endpoint, a bounded reattach/restart, observable stale
handle refusal, real wire work through the new instance and an
unchanged spawn-record/frame baseline. This closes the *open* Phase 7
supervision commitment only when 8.0 actually passes those tests;
writing this ADR does not close it.

## Options rejected and later policy

- Give the manager MMIO or a generic `request_privileged_cap` API:
  rejected. That would turn a manifest typo or parser compromise into
  device authority. Kernel device policy stays in ADR-0021/0028.
- Register netstackd with the kernel supervisor *and* the manager:
  rejected. Two independent restart loops can spawn competing servers
  and replay the same grants. A separate Phase 7 kernel-only closure
  would need a tested, atomic ownership handoff before 8.0; the
  simpler v1 is to close this obligation as a mandatory 8.0 gate.
- Interpret requested grants as success and silently skip unavailable
  ones: rejected. The child may start without its security preconditions
  or the manager may broaden rights in an attempt to make it work.
- Treat pid, manifest name or caller identity as authority: rejected.
  The kernel enforces possession of Image/Process/Endpoint caps; UDP/TCP
  bearers remain service-issued, transferable authority (ADR-0033/0036).

Later permission manifests and grant UI (8.2) decide *who may ask* for
application-level permissions, interactive approval, persistence,
revocation UX and policy updates. 8.0 manifests are static service
startup declarations only; no shell command or network input can edit
them. Transactional config (8.1), signed package trust (8.4) and
installer/updater (8.5) have separate roots and acceptance tests.

## Qualification before accepting this ADR

Implement only after this design is reviewed. Tests must inspect
*actual installed child capabilities* and reject absent/overbroad
requests, duplicate owner, cycle, missing dependency, premature
readiness, unauthorized reap, unbounded restart and stale TCP/UDP
bearers. Preserve every old regression. A completed 8.0 checkpoint
requires `tools/run_tests.sh`, a freshly built image and
`tools/stability_loop.sh 100` with matching artifact receipt; this
proposal and a green M7 receipt are not that checkpoint.
