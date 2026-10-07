# ADR-0097: Multiple ordinary windows per application session

**Status:** Accepted for Phase 13 implementation

**Date:** 2026-10-07

## Context

Phase 12 authenticates one Desktop session through a server-minted badge and
holds one exact Process capability. Its production UI path coupled that
session to one SharedRegion, one private snapshot, and one compositor window.
The compositor model already had bounded window records, but the model used a
surface backing ID as both duplicate-surface key and owner identity. This
prevented one authenticated process from owning multiple independent windows.

Window handles and application IDs are descriptive. A process must not gain
Desktop access by guessing a handle, and an additional window must not be
implemented as an unrelated process or AppInstance.

## Decision

Keep the ABI-v2 primary session unchanged. Add two explicit wire operations:

- `Create` creates the session's primary window over its already inherited
  surface region.
- `CreateAdditional` asks the same badge-authenticated endpoint for another
  ordinary window. Desktop allocates a fresh SharedRegion and private snapshot,
  creates a unique compositor surface record owned by the authenticated
  session, and transfers a bounded surface capability in the checked IPC
  reply.
- `DestroyWindow` retires one window while keeping the process and sibling
  windows alive.

The dynamically transferred surface cap carries READ|WRITE|COPY|DESTROY.
DESTROY only lets its holder discard this exact cap. It does not release a
window handle, select a surface, grant Desktop service access, or affect a
sibling cap. The primary startup cap inventory and its rights are unchanged.
All client requests continue to carry the original session backing cap and
the server-minted badge; only the Desktop's held per-window record binds the
additional surface object to its owner.

The compositor model now stores two descriptive values per ordinary window:
the authenticated owner and the unique backing-generation key. Authenticated
operations check owner; duplicate surfaces check backing. Each additional
record holds its own content VA, private snapshot VA, title, published size,
damage generation, transient surface state, and close animation state. The
existing State table still limits the desktop to 32 total ordinary windows;
the new record table is bounded at 32 and no resource ABI constant increased.

Backing keys use slots 0–31 for primary session surfaces and 32–63 for
additional surfaces. The compositor scene still contains at most 32 active
windows; its damage comparison scans all 64 descriptive backing keys so an
additional window is repainted when it first publishes and on later partial
damage. A first guest run exposed that scanning only slots 0–31 left an extra
window blank if Desktop rendered it once before its first `Damage`; the
bounded comparison and a host pixel-equivalence regression now cover the full
key space.

Closing the primary window keeps its session backing and snapshot available
for a later `Create`; the application can remain headless. Closing an
additional window drops the broker mappings after the owner acknowledges
`DestroyWindow`. Process teardown first reaps the exact held Process, then
retires every remaining window and mapping. A forced Desktop close of an
additional window terminates its owning process because the broker cannot
revoke a capability already held inside that process.

## Consequences

- One process can own zero, one, or multiple ordinary windows without
  fabricating an unrelated process identity.
- Each window has independently allocated content/snapshot memory and
  compositor, publication, resize, transient, and close state.
- Every new surface consumes one SharedRegion capability and two mappings
  while live. The existing 32-total-window policy bounds these costs.
- Process-wide application identity and multi-process AppInstance/helper
  ownership remain separate future work; this ADR does not equate a window
  handle with an AppInstance or process capability.
- The new wire operations are native ArenaOS protocol additions and introduce
  no POSIX or Linux behavior.

## Validation

Desktop host tests pass 84/84, including one owner with three independently
backed windows, distinct dimensions, owner refusal, per-window minimize state,
individual retirement, and high-slot first-publication/partial-damage pixel
equivalence. The desktop target binary and signed APB1 guest fixture build
successfully.

`tools/test_phase13_registry_guest.py` passed on QEMU 10.0.11 / OVMF 2025.02.
It installed a signed package, launched it through the receiver-verified
registry, and observed the real installed process publish three distinct
colored surfaces under three compositor-owned ordinary windows. It launched
two application instances, each owning three windows. Closing one window
released exactly two SharedRegions, 940 pages, and three maps while the same
process and sibling windows stayed live. The fixture then closed each remaining
window independently; process, SharedRegion, page, map, and cap identity counts
returned to the boot baseline. The guest also retained the prior explicit Open
With, read-only document, scalable heap, and VM proofs.
