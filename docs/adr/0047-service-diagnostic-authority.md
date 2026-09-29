# ADR-0047 — Service-side authority for destructive and lifecycle diagnostics

*Status: accepted and qualified. The receiving-service boundary closure passed all
27 historical suites and a fresh final-EFI-bound 100/100 QEMU qualification.
Phase 8.0 is closed again; 8.1 remains the next design milestone.*

## Gap

An Endpoint/WRITE grants the right to make a *call*, not the right to
kill its server. IPC v1 transfers a capability but does not expose the
rights on the caller's endpoint. Checking the caller's local COPY bit
in depcheck, or checking Power in the shell, does not authorize an
operation at rngd or netstackd. This applies to #UD/stall injection and
all service-side `SHUTDOWN`/poison opcodes, including historical M5/M6
fixtures. The manager remains the only owner of production stack
lifecycle; the kernel supervisor remains the only driver restart owner.

## Decision

At each service with an opt-in destructive opcode, the trusted boot
root mints a **distinct, otherwise unused Notification object** and
grants the server a READ-only reference in a fixed diagnostic slot.
Only the explicitly trusted test actor receives a READ|COPY|DESTROY reference;
COPY is necessary to send it in a call. The marker has no WRITE right
and is never used for waiting or signaling. `SYS_IPC_RECV` lands the
actual transferred cap in the *service's* cap space. The server compares
its `SYS_CAP_DESCRIBE` kind and object with its OWN boot-granted
reference, rejects a missing/wrong-kind/wrong-object cap or unexpected
rights, and destroys every landed reference with its transferred DESTROY right before replying or exiting.
Numeric nids, opcodes, pids and string names are NOT credentials: a
client cannot mint the Notification object whose reference it lacks.
There is no new kernel capability kind or syscall.

Production rngd's diagnostic marker is boot-granted to rngd and the
manager; only the manager delegates it to its fault/stall worker during
an opt-in private-admin request. Production netstackd's marker is
boot-granted to the manager, inherited by each production child via
its manifest with a READ-only server reference, and given READ|COPY|DESTROY
only to the privileged Power-holding shell for the `stackfault`
fixture. The shell's ordinary stack endpoint is insufficient. The
manager's existing Process/DESTROY and private admin channel remain
its production lifecycle authority; the marker does **not** grant a
Process handle. The existing COPY-bit mode switch in depcheck remains
only a trusted *fixture selector*, never service authorization.

Legacy M5/M6/M7 ephemeral test services likewise receive a separate
marker and their test actors carry its READ|COPY|DESTROY reference for their
existing poison call. Production storaged, fsd, netd, inputd and
consoled are granted **no** destructive marker or trusted poison
caller: they reject poison on their ordinary service endpoints. Test
roots retire their marker after children exit. No change to normal
GET/SEND/RECV/ARP/FS data operations or DMA frame landing; a marker
is not accepted as an ordinary data buffer. Bounds on Notification
objects and spawn inheritance must be adjusted and independently
audited rather than hiding grants in ambient powers. The worker's
ordinary three grants become five only for an opted-in fault/stall;
the service's own boot reference never has COPY or DESTROY. Proof
senders explicitly carry DESTROY so they can transfer and discard a
received marker; that right never retires the shared Notification
object. **Malformed** sender-supplied caps may omit DESTROY; refusing
those calls while leaving a landed reference forever would let an
ordinary client fill the receiver's 16 slots. Thus the kernel marks
IPC-landed slots at grant time; `SYS_CAP_DESTROY` may drop only that
receiver-owned reference even if its transferred rights omit DESTROY.
This does NOT confer Process/DESTROY authority (SYS_PROC_FINISH still
checks its original rights), free an owned frame (IPC makes it lent),
or add a right to the transferred cap. Derived/copied ordinary caps
are not marked. The receiving service checks exact proof rights and
consumes every landed reference on success *and* failure.

The independent production-child audit checks the five inherited
slots against literal policy. It must not treat post-spawn caller-
controlled IPC landings in other slots as bootstrap grants and halt
the kernel on an untrusted client's request. SYS_SPAWN itself limits
the inherited cap count to five; child grants are independently checked
at that boundary. The legacy M6 test services use marker slot 3;
production drivers' slot 3 already names readiness or console control,
so the shared service helper accepts slot 3 only if it is exactly a
READ-only Notification, otherwise checks their reserved slot 4/5.

## Proof obligations

At the receiving service, exercise ordinary Endpoint/WRITE with no
marker and with a distinct live marker (and a forged integer/slot or
wrong-kind cap), verify a typed refusal and continued normal service,
then demonstrate the real opted-in marker triggers the historical
#UD, stalled GET, and poison/exit proofs. Verify the marker cannot
signal manager/private timers, cannot be inferred from an endpoint
rights bit, is never given to a data-plane client, and persists through
production respawn and kernel driver grant replay. The entire historical suite, fresh exact-EFI-bound 100/100 QEMU boots
and an extracted-archive boot gate qualify this reopened 8.0 checkpoint.

Reject server-side pid checks, raw numeric object-id 'secrets', a
caller-only check, and removing regression tests by making SHUTDOWN
unavailable in the old short-lived fixtures.
