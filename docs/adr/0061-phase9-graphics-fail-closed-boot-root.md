# ADR-0061 — Fail-stop graphics boot-root service death, no implied restart

Status: qualified with real guest death/RED controls, 79/79 historical suites, exact-EFI 100/100 graphical boots and independently pixel-verified extracted archive. Date: 2026-10-02.

## Context

ADR-0060's compositor holds a copy of each original client's SharedRegion
and two Process/READ witnesses. A copied cap is still authority after the
original exits; numerically named owners or a dropped sender cap cannot revoke
it. A compositor whose last thread exits can also strand inputd in an IPC
CALL and leave its last pixels visible. The current ordinary boot has a
long-lived root idle thread, and `proc::destroy(server)` already fails all
queued/delivered callers and marks its serve endpoint orphaned. No existing
manager protocol has a policy for reviving compositor state, surfaces, key
queues and the scanout together. Advertising restart would be false.

## Decision to implement and qualify

The boot root records the exact original display and compositor processes it
created. In the idle loop it checks thread liveness, not a caller-provided PID,
name or wire field. If either service unexpectedly loses its last live thread,
root destroys the exited process (so historical IPC teardown answers every
call state and the endpoint becomes orphaned), then **halts the whole machine
with an ERROR**. It does not reassign the display BAR/scanout, preserve stale
focus, replace the serving endpoint or claim restart. The machine's orderly
shutdown path remains reserved for an authorized Power cap. A test injects a
controlled service exit in the real guest after both surfaces are present;
RED must show the fatal halt and no healthy graphical key result. The exact
source-restored EFI then takes the same input and produces the genuine QMP
changed pixel. A zero QEMU exit without those semantics is RED, not PASS.

A separate original-client-death probe must make an actual child exit while
another bounded client remains live. Root first destroys/forgets exactly its
original dead BootImage child, including PTEs, caps and in-flight calls; this
invalidates the compositor's held Process/READ witness without granting any
new authority. On its next receive the compositor treats zero-live *or a
stale original Process witness* as dead, drops its retained mapping/cap,
removes the owner's surfaces/focus/queued keys, and redraws the uncovered
pixels. A copied SharedRegion alone cannot resurrect the client. Root then
uses a read-only generic SharedRegion registry snapshot for the **known,
root-created full ID**, checking exactly `(refs,pins)=(1,0)`: only its
original comparator reference remains, with no client or compositor mapping.
Only at that stable boundary may root consume the comparator and stale
Process reference, proving full SharedRegion and process-record retirement.
Wrong tuple or delayed compositor receive is not treated as a successful
teardown; a bounded timeout must fail-stop rather than retain an unaccounted
owner forever. The still-live second client continues to serve. A forced
client exit *without an orderly DESTROY* plus a QMP screenshot must
demonstrate that former window pixels are gone and the original underlying
base is restored. The separate service-death control covers an in-flight
caller slot; it is not recast as a queued-client-exit proof.
Failing closed at the machine boundary for a dead service is distinct from a
successful service restart.

No extra Notification, dynamic child, inherited grant, cap slot, framebuffer
access or kernel display policy is authorized by this ADR. The kernel change
is only root-owned liveness supervision using existing process/IPC teardown;
the user-space compositor retains all clipping, window, font and focus policy.

## Real-guest lifecycle evidence (qualified with final Phase-9 image)

`tools/test_m9_service_death_red.py`: a test-only mutation exits the real
compositor after a receiver-authenticated QMP keyboard KEY while production
inputd is in an IPC call. The root's actual process destroy answers **one
in-flight call** with `STATUS_SERVICE_GONE` and halts ERROR; no healthy client
pixel is painted. The exact source/EFI restored in `finally` then boots the
two-window QMP-before/after injected-key pixel test GREEN. The service is
**not** restarted or revived under a name.

`tools/test_m9_client_death.py`: QMP sends `x` to the focused second client,
which exits without sending DESTROY. The kernel retires that original child's
Process record/caps/PTEs; compositor uses the now-stale held Process witness
to retire its own copy, focus, queue, region mapping and surface and redraws.
At stable `(refs,pins)=(1,0)` root drops its comparator and stale Process cap;
actual guest shared usage falls from 3/8 runs, 507/2048 pages and 6/32 maps
to 2/8, 488/2048 and 4/32. Captured QMP `(200,190)` changes from the second
window's RGB `(61,207,122)` back to underlying display RGB `(227,53,66)`;
overlap title uncovers the first window, which continues to run. A Power-only
shell snapshot in `tools/test_m9_resources.py` measured 16/16 -> 15/15
spawn-record/process retirement and free frames 114445 -> 114482. Compositor
cap occupancy may read 7 or 8 depending on one *other live client* IPC
landing; the per-region ref/pin and map conservation boundary is exact.

`tools/test_m9_client_death_red.py` deliberately forces the compositor to
ignore the Process witness. The actual machine leaves a stale QMP pixel and
a copied mapping, and root halts ERROR after its bounded retirement deadline.
Restored byte-exact source/EFI boots GREEN with the actual uncovered QMP
pixel and exact accounting. This is a verified failure control, not a fake
retirement or a claim about spontaneous idle cleanup. Pointer routing and
live service restart remain unimplemented and are not claimed.

GPU-only integration now has a separate real-guest QMP-before/after focused
keyboard control in `tools/test_m9_gpu_compositor_input.py` (see ADR-0058).
Both the real GOP and GPU-only paths preserve identical owned-window pixel
semantics. This does not change the no-restart decision. A 78-test historical
suite run was deliberately interrupted after M8.1 to add this missing
integration check; it was not a completed gate. The subsequent fresh 79/79
suite, exact-EFI 100/100 pixel boots and independently extracted-archive
pixel boot all passed. See `docs/TESTING.md` and the hashed archive receipt.
