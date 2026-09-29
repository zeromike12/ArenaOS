# ADR-0038 — Service-manager bootstrap grants and driver-readiness channel

*Status: Accepted (Phase 8.0 bootstrap/inventory checkpoint, not a completed manager).*

## Context

ADR-0037 reserves device-driver grants and lifecycle for the kernel, and
ordinary service policy for one ring-3 manager. A supplied `Held`
inventory is not kernel authority: the manager must query its *actual*
cap space, prove external dependencies reached a real ready state, and
refuse a partial graph before it may spawn `netstackd`. The existing
notification table has eight entries, and `MAX_INHERIT=4` exactly fits
the stack's four grants. M7 starts and tears down its own test stack;
it does not start a production stack.

A kernel log string saying `ready` is not a service-readiness protocol.
A synchronous IPC probe from the manager could park indefinitely if a
driver never serves it; a sleep is neither a readiness check nor a
bound. Neither can become an implicit permission-minting path.

## Decision

The fixed kernel trust root embeds image **19** as the manager. It
always installs slot 0 = Image17/READ and slot 1 = manager netd-ready
Notification/READ|WRITE. When **both** network and RNG dependencies
exist, it additionally grants in literal slot order: 2 = netd
Endpoint/WRITE|COPY, 3 = stack Endpoint/READ|COPY, 4 = stack-backoff
Notification/READ|WRITE|COPY, 5 = rngd Endpoint/WRITE|COPY,
6 = dedicated rngd-ready Notification/READ|WRITE. These are
held caps, not manifest requests. With either dependency missing,
bootstrap grants **only** slots 0 and 1 and the manager reports
OFFLINE. At bootstrap it receives no MMIO, Power or Process cap. The
kernel audits the installed cap table against the literal list and
refuses every unexpected occupied slot. The ring-3 program uses
`SYS_CAP_DESCRIBE` to read and resolve its live inventory; the manager
does not infer capabilities from names or device identities.

The kernel creates two **different** readiness notifications before
starting production netd/rngd, then lends each driver WRITE to *only
its own* notification at its previously unused slot 3. The manager
holds READ on both. A driver signals after its device and queue reach
DRIVER_OK and pass startup checks, before serving IPC. Distinct badge
BITS on the same notification would NOT isolate the drivers: either
WRITE holder could forge the other driver's badge. Cap possession,
not a label in the badge word, is the proof of signal origin. The old M6/M7 test drivers have no slot 3: their optional
notification is refused and ignored, preserving their old contract.
Production drivers are registered with the kernel supervisor; a
restart replays its own readiness grant. The manager arms a real timer
and waits on each channel in turn (two bounded 2-second stages, never
an unbounded blocking IPC call). It requires both independent signals
before their respective deadlines. Timeout, missing device, unexpected
badge, wrong kind/rights, or a partial manifest leave the service OFFLINE rather
than launching it. A timeout coalesced with a last-minute ready badge
still wins.

This design consumes three additional production notifications: netd
readiness/manager events, rngd readiness, and stack backoff. The full
fixture needs **nine** slots (storaged, netd, rngd, inputd, consoled,
shell and the three above), so `MAX_NOTIFS` rises explicitly 8 → 9.
The kernel asserts a tenth allocation is refused in the full-fixture
boot. Later managed-child exit badges can share the manager-events
notification; the stack's backoff object remains separate.

## Options rejected / limits

- Distinct badge bits from two drivers on ONE shared notification:
  rejected because either driver with WRITE can spoof both bits.
- Poll a log line or sleep until drivers "probably" started: no actual
  authority, no bound, and no negative-space behavior.
- Give the manager MMIO or a generic device cap request so it could
  probe hardware itself: violates ADR-0037's kernel/device boundary.
- Start `netstackd` if only one driver exists: the missing grant makes
  UDP bearer issuance unreliable and a partial service look healthy.
- Declare full readiness from a badge alone: a driver could die just
  after signalling. The badge proves a specific startup stage, **not
  current liveness**. Spawn-time behavior and child-specific readiness
  need later integration proofs. In particular this bootstrap slice
  does not spawn a child, restart the stack, or close Phase 7's open
  `netstackd` supervision obligation.

## Qualification / implications

Host tests check the readiness gate, duplicate/unknown badges,
missing authority, and coalesced deadline rejection. A QEMU test must
cross-check the IDs the ring-3 manager actually observed with the
kernel's literal grants and ensure two *different* readiness objects,
prove the ready path on the full network
fixture, and prove OFFLINE with no partial grants when devices are
absent. The full historical suite and a fresh artifact-bound 100/100
boot receipt gate this partial checkpoint. No `m8: RESULT PASS` line
may appear until actual manager spawn, Process-cap-gated reaping,
production stack restart/recovery and negative-space accounting pass.
