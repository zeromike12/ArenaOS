# ADR-0060 — Bounded boot-client liveness and input-producer witness

Status: implemented and qualified for the bounded two-boot-client topology; original-client-death RED/GREEN and service-death fail-stop proved, exact-EFI 100/100 and extracted archive boot passed.
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

## Implemented integration and provisional evidence (not milestone closure)

The BootImage table now has six indices. The displayprobe's historical
completion/retirement barrier runs first. Root grants the compositor a
call-side display endpoint, serve-side client endpoint, and proof comparator;
it separately issues two held Process/READ and corresponding root-allocated
SharedRegion witnesses. Each independent client receives only the compositor
call endpoint and its own 19-page region. There is no MemoryPool grant to
either window, no client token and no cap-table expansion. The production
inputd swaps its unused serve-side endpoint for compositor-W, receives the
fifth-slot token and forwards decoded printable virtio-keyboard presses while
continuing to feed the original console; its diagnostic instance is unchanged.
A generic held-Process/READ-only `SYS_PROC_LIVE=41` returns one for live
threads, zero for an exited but still recorded original process, and refuses
wrong-kind/right or retired references. The compositor checks both witnesses
on every request, retires dead-owner surfaces and queued keys before serving
that request, validates landed full-region generation against root's held
witness for every owner operation, and consumes landed caps on all paths.
It only accepts KEY when the transferred proof matches its held comparator.

`tools/test_m9_compositor_input.py` boots the actual EFI, captures an 800x600
QMP PPM with both real client windows, overlap z-order, bitmap title and
base pixels, injects `q` through QMP/virtio-input, then checks a second
actual screenshot for the client-painted key pixel while unchanged pixels
stay exact. A separate client sends forty forged KEY requests carrying its
region cap and sees forty refusals. The first guest run failed because root
tried to delete its own historical READ-only input endpoint grant through
`cap::destroy` (which correctly requires DESTROY); root now retires that
kernel-granted cap through the private internal `cap::consume` path before
issuing the new endpoint. A subsequent guest run succeeded but the test
failed because it checked the asynchronous forged-request marker before the
client had finished forty IPC round trips. The test now waits for all markers
before stopping the machine, and the complete guest/pixel proof passes.
`tools/test_m82_capspace.py` measured post-EBS `(frames,records,processes)=
(1672,16,16)` in the full guest: the exact 13/13 prior baseline plus three
static graphics residents, not an expansion of the dynamic-child allowance.
A provisional `tools/stability_loop.sh 2` passed 2/2 on one EFI with **two
real QMP captures, injected input and exact host DNS receipts per boot**.
The bootstrap root additionally reports occupied cap slots, shared runs,
page total, map pins, live process count and free frames after both client
surfaces map; the later Power-only high-water and teardown measurements below close the peak-accounting obligation.

ADR-0061 now supplies real forced original-client exit, root/compositor
ref/pin retirement and QMP base uncover, a mutation that leaves a stale
pixel and fails the bounded deadline RED, and a real service-death/IPC
fail-stop RED with restored pixel GREEN. Power-only guest measurements
record 16/16 residents, 507 pages/6 maps at the client high-water and
15/15 residents, 488 pages/4 maps after one death. There is **no service
restart claim**. Compositor cleanup is still triggered by its next receive;
if no requester drives it, root's bounded deadline fails closed rather than
pretend cleanup occurred. By itself this targeted evidence did not qualify Phase 9; the full-suite,
100-boot and archive gates below now do.

**Final gates closed:** 79/79 historical/graphics suites, final-image
100/100 with per-boot graphics input and an independently extracted
pixel-verified archive. Model capacity/queue overflow and real SharedRegion
capacity/refusal gates ran in the complete suite; an arbitrary
malicious client's copied cap still cannot be revoked by deleting its original.
