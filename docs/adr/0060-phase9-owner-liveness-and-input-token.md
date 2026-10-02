# ADR-0060 — Bounded boot-client liveness and input-producer witness

Status: proposed for Phase-9 service integration; no guest proof claimed.
Date: 2026-10-02

The current pure compositor model cannot know that an original client died
while it retains a *copy* of that client's SharedRegion cap. Revoking one cap
is not revoking all copies, and neither IPC sender PID nor user-declared owner
numbers is authority. To make the two independently linked boot windows
lifecycle-safe without touching the one-unretired dynamic-child bound, boot
root grants the compositor two **Process/READ** capabilities for the two
specific `BootImage` children. The compositor may query a new generic
`SYS_PROC_LIVE(slot)` mechanism (syscall 41): require a held Process/READ cap,
return 1 if a live thread exists, 0 if it has exited, or a typed negative
status for a wrong kind, stale cap, or missing right. No process ID argument,
kill, spawn, privilege escalation, or mutation; kernel remains mechanism-only.
Boot root allocates two ordinary bounded SharedRegions *to the respective
BootImage children* before they run and separately issues matching, attenuated
region caps to the compositor together with each child's Process/READ cap.
The recipient compares the full landed SharedRegion ID against those two
root-provisioned caps; that exact possession-based association selects the
corresponding held liveness witness. Clients get no ambient allocation pool
and cannot claim another client's root-provisioned surface with a guessed ID.
No PID in the wire or requester-identity check grants operations. The
compositor checks held Process caps before accepting requests and retires
all the dead child's mappings, surfaces, focus and queued keys before reusing
a slot; root remains owner of child destruction and full process retirement.
Absent a root-provisioned region + Process witness pair for an arbitrary
launched client, the service refuses CREATE, not a false cleanup promise.

Keyboard producer authentication needs an **inert** root-issued
`CapObj::ProofToken { id }`, exposed as a distinct kind 10 in the existing
held-cap descriptor ABI. No syscall mints, invokes or guesses a token. Root
grants the same object (READ|COPY) only to production inputd and (READ) to
compositor; only the former transfers it with typed KEY. The compositor
receiver compares the full token object and rights, destroying the landed
copy on *all* paths. A copied token remains authority by possession; deleting
an original cap does not revoke the copy. The inputd diagnostic fixture keeps
its existing four-slot topology and marker at slot 4; production uses that
fifth slot for the input witness and remains under `MAX_INHERIT=5`.

There are no additional notifications or ambient framebuffer rights.
A service-death/restart assertion needs a separate real kernel IPC/death
negative, a fail-closed stale endpoint and QMP pixel proof. Neither a host
model nor a source-only draft proves any of that; do not ship a restart claim
from this ADR alone. Raising CAP_SLOTS or the dynamic-child bound is outside
this decision. Pointer routing is optional and not inferred from keyboard.
