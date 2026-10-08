# ADR-0067 — Owned desktop frame publication

Status: implemented; actual publication and production direct-read RED/restored GREEN proven; full qualification pending.

## Problem

A client renders into its own writable SharedRegion before sending Damage.
Reading that mutable staging backing on unrelated pointer/focus frames could
expose a partially painted or unsubmitted drawing. Complete displayd staging
alone does not establish the client's publication boundary.

## Decision

The desktop retains one private complete pixel snapshot per bounded session.
Only an authenticated Damage request for that session's exact region and owned
window copies width*height pixels into the snapshot. Subsequent redraws,
movement, focus transitions and close animations use that private snapshot.
A new session starts unpublished; its body is a blank elevated surface until
its first complete Damage. Reuse cannot reveal a prior owner's cached pixels.

The copy uses volatile reads from shared memory and normal writes to private
broker memory. The ordinary launch topology gives clients one thread and no
Process/Image authority to create additional threads; the sender is blocked
in synchronous CALL during publication on the existing single-core scheduler.
No client receives a cap/map into the snapshot. Clients cannot publish another
window by numerical handle or descriptive metadata. The neutral wire remains
unchanged; Damage means publication of the complete owned staging raster.

The static six-snapshot reservation is 6*448*288*4 = 3,096,576 bytes (756 pages)
in the broker's private address space. It is allocated/reclaimed through the
existing BootImage loader, not a kernel window mechanism or new SharedRegion
class. The six-session and 127-page client backing limits remain unchanged.
This preserves the eight-object/2048-page SharedRegion ceiling on both GOP and
GPU paths. There is no additional per-launch resident snapshot allocation.

## Evidence

The signed dynamic fixture stages different actual pixels on key `f` without
Damage. Pointer frames must continue showing the previous owned raster. Key
`p` publishes those prior bytes without drawing any new pixels and must make
them visible. The production direct-staging-read mutant must fail this oracle.
Existing exact drag, scope, stale-owner, process retirement, capacity and full
working-set tests remain required after this change.

The broker also uses checked reply cancellation: an abandoned caller consumes
its tombstone and returns CALLER_GONE without causing desktop service death.
Other IPC failures remain fail-stop. Native occupancy is sampled before landed
references are dropped and after spawn; the observed cap high-water is retained
separately from the steady-state resource snapshots.

## Limits

This is opaque complete-raster publication, not client alpha blending, damage
rectangles, GPU fences or a multi-core/multi-writer protocol. A future grant
that permits additional client writer threads must define synchronization before
claiming coherent publication. The bounded private cache cost is accepted for
this desktop and must be included in actual memory measurements.
