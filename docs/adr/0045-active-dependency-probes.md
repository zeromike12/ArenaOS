# ADR-0045 — Bounded active dependency probes before every managed stack spawn

*Status: accepted. The original four Phase 8.0 exit areas were qualified;
ADR-0047 subsequently closes the service-side diagnostic authorization
gap discovered during review. Use the later, freshly qualified checkpoint.*

## Context

Boot readiness badges attest that the drivers *were* ready. The manager
currently substitutes `External { ready: true }` on each restart without
checking either driver's current service boundary. The new stack checks
both during its own startup, but that is after the manager has already
spawned it. A direct `SYS_IPC_CALL` in the manager would block forever if
a still-live driver stopped replying: a manager timer cannot run while
that same thread is blocked. A static cap description or pid is not an
active probe.

## Decision

Use a short-lived, embedded **user** probe image (registry image 20),
spawned and owned by the manager only on a complete boot fixture. The
root grants its Image/READ cap to the manager in a fixed new slot; no
privileged device or Power caps go to the worker. The manager copies
only its existing netd Endpoint/WRITE and rngd Endpoint/WRITE to the
worker, and an attenuated WRITE-only copy of its private restart
notification. It owns a unique Process/DESTROY handle for the worker.
The worker performs a real `NET_OP_MAC` request, checks a nonzero MAC,
allocates/maps its own page and performs `RNG_OP_GET` with a LENT copy
of that page, requiring a full device completion and nonconstant data.
Only *after both calls* it signals a private success badge and exits.
The exit notification uses a distinct bit on that same manager-private
object. The manager uses its own deadline on that object, requiring
both success and exit before the deadline wins (including coalescence),
then reaps through the held Process cap and only then passes both
`External { ready: true }` facts to the manifest resolver. Failure,
missing badges, dead worker, or expiry means OFFLINE, not a spawn. On
expiry the manager force-stops the blocked worker through its held cap;
the kernel cancels in-flight IPC when destroying it. No synchronous
probe call runs in the manager. The restart timer object becomes
COPY-capable *in the manager only* so it can pass WRITE to the worker;
netd, rngd, stack and shell cannot signal the private success or
backoff bits. The kernel independent manager-child audit identifies
and verifies the worker's exact attenuated cap shape separately from
its four-grant production stack audit. All process handles are retired
before production launch and after every restart probe; the manager
never assumes the worker's cap slot based on a pid.

The default shipping boot actively probes once before the first spawn.
Each restart actively probes again after bounded backoff, before
planning or launching a new child. A full-fixture host actor must match
probe rounds and reaps to production pids, verify actual driver ops,
post-restart wire work and flat accounting. An absent-device boot
cannot receive the probe image or partially launch a child. A negative
fixture must prove no new production child is launched if the
current dependency fails or does not answer before the bound; a synthetic
`ready=true`, log message or healthy replacement boot is not evidence.
The negative fixture uses the existing private admin notification,
not ambient pid authority: an opt-in shell `depdeny` request asks the
manager to stop its live child and arm a one-shot failure on the *next*
probe. The manager delegates netd Endpoint/WRITE|COPY instead of WRITE
to this trusted worker only; this extra COPY right is a bounded mode
flag the worker observes on its actual cap and cannot forge without the
manager's delegation. The worker requests rngd `FAULT_NEXT_GET` (a
replying driver-control operation, not a fake successful probe) and then
makes the real RNG_GET call. Rngd triggers a real user #UD while serving
that request; kernel IPC death handling replies SERVICE_GONE to the
worker, the worker fails without signaling private success, and the
manager reaps it and refuses another production child even if the
kernel supervisor later restarts rngd. The first real negative boot
exposed an existing supervised-fault gap: a driver #UD exits its only
thread but leaves its process address space and in-flight caller alive.
Before implementing the fix, decide that the fault exit marks the
supervised driver dead (only on its last thread); the kernel idle-thread
supervisor reaps that still-present process with `proc::destroy` before
forgetting its spawn record or replaying grants. This occurs outside
the faulting CR3/IDT context, releases blocked callers with
`STATUS_SERVICE_GONE`, and permits bounded driver restart. An already-
destroyed driver is not destroyed twice. Teardown failure must stop
restart rather than leak a root and manufacture a healthy driver.
The kernel audit accepts only the three exactly defined worker cap
shapes (ordinary, fault, stall); diagnostic modes exist only behind
the shell's pre-existing private admin authority. A second private
`depstall` request selects rngd Endpoint/WRITE|COPY
instead, instructing the same trusted worker to ask rngd to stall its
next GET *without* a device completion. This opt-in fixture proves the
manager's own timer expires while its worker is blocked and its held
Process cap stops/reaps that worker without launching a replacement
stack; the driver is intentionally wedged only in the destructive test
VM. Ordinary unattended boots never inject a fault or stall.
An active probe is a point-in-time check, not an uptime guarantee;
subsequent failures still use the stack's established re-attach path.

## Rejected alternatives

- Block the single-threaded manager in IPC and pretend a timer bounds
  it: blocked callers cannot observe their own timer.
- Make the kernel mint a new driver/health capability or infer service
  health from a pid, boot badge, manifest, or cap description.
- Give the shell or stack a Process cap to the driver, or let a shared
  badge assert readiness; neither grants driver liveness authority.
- Add a test-only mint syscall or a second restart owner.
