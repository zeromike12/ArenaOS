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

## Boot integration order and fixed capacities

Existing BootImage slots 0 displayd, 1 sharedprobe and 2 displayprobe stay
byte-identical; new indices 3 compositor, 4 window A and 5 window B raise
only the bounded BootImage index table to 6 (ADR-0057 correction). Keep the
legacy displayprobe completion/retirement barrier *before* compositor takes
its base image, so historical QMP pixels remain visible outside windows.
Only once a validated display is present, root creates the compositor endpoint,
spawns the compositor with display-W/compositor-R/token-R, spawns the two
independent client executables with only compositor-W, creates one 19-page
region for each child (slot 1 in each), and issues to compositor slots 3..6
the two Process/R and two matching region/RWC witness caps. The root knows
these pairs from its own `shared::create(pid, pages)` operation; no IPC PID or
new region-provenance syscall is needed. No client receives `MemoryPool`,
`Mmio`, `SharedDma`, another client's region, a Process cap or input token.
A compositor that cannot verify both root-issued pairs fails closed.

Production inputd is launched earlier and keeps its current four grants,
including slot 1's legacy READ-only service endpoint. Its no-ConsoleInput
fixture uses that endpoint unchanged. After the compositor has checked MODE
and parked, root installs the inert proof token in production inputd's fifth
slot, destroys its unused legacy endpoint cap (retiring the endpoint), and
replaces slot 1 with compositor-W. Console mode forwards keys **only** when
slot 4 describes the expected held proof and slot 1 actually has WRITE;
before that boundary it still pushes bytes to the existing console and does
not block on a compositor which is not yet serving. On compositor exit, root
must reap/retire the endpoint or fail-stop; the input service must not leave
the shell blocked by a dead compositor. No new notification, no sixth
inherited grant, no dynamic-child slot, no cap-table increase.

The current Phase-8.5/early-9 `test_m82_capspace.py` snapshots measure a
post-EBS 13-record/13-process baseline after displayd. A successful
three-additional-BootImage topology should produce 16/16 in that same
fixture (and one more only during the separate dynamic-child cutover), not
the historical 13/13 or an unchecked count. Update that assertion only with
an exact guest measurement and retain the pre-graphics 13/13 evidence.
Record shared-registry runs/pages/maps, capability-slot peaks, free frames,
spawn records and resident process high-water after both clients are live.
