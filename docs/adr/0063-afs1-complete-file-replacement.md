# ADR-0063 — AFS1 complete named file replacement

Status: implemented prototype; crash and refusal qualification in progress.

AFS1's existing WRITE changes existing data sectors in place. Desktop editor
saves need one atomic complete-document operation. Add `FS_OP_PUT=9` to the
trusted raw fsd interface: a canonical padded name, length 0..4096 and a held
one-frame LENT Untyped/READ buffer for nonempty data. This creates or replaces
one flat file and returns no open handle. Open existing objects refuse BUSY;
files larger than the bounded replacement budget refuse RANGE. No disk layout,
commit record, package format, mount selection or version number changes.

A pure planner reads the current bitmap and reserves two disjoint four-sector
metadata runs, a fresh extent sector and up to eight distinct data sectors.
It does not reclaim sectors or modify state on refusal. Validate old extents,
object/open-table capacity, length, held buffer type, sequence exhaustion and
bounded retirement bookkeeping before any data or metadata mutation. Ordinary
space refusal cannot occur after a successful whole-operation reservation.

Write complete data to fresh sectors, a fresh extent block and the preallocated
object/bitmap tables. Publish through the existing single-sector ping-pong
commit written last. Transport/device failure fail-stops fsd and never returns
success. Crash-prefix recovery sees exact old or exact new bytes, including
shrink and empty replacement. Partial old-file overwrites are forbidden.

Retire the superseded data and extent **after** the metadata commit rotates its
retirement lists. They have the same lifetime as the superseded object table:
retain them until its older ping-pong record has been overwritten. Placing them
in the pre-commit dead list would reclaim them a transaction too early. This
new operation must independently check both valid commit generations.

Add held-cap descriptor kind 11 for a LENT Untyped: object field means one
4096-byte frame, rights are unchanged. It exposes type/geometry through an
already held reference, no physical address, new cap, mapping or allocation.
Owned frames and MMIO retain their existing descriptor refusal. This lets fsd
reject wrong buffer kinds before reservation rather than discovering them
through a failed DMA after mutation.

The existing coarse raw filesystem writer remains trusted. Ordinary desktop
clients receive scoped mediated file grants, never this endpoint. `put` in the
existing trusted serial shell is an independent guest test and useful command.
Historical WRITE's bounded semantics remain unchanged and are not relabeled
transactional replacement.

Qualification must include production-linked host reservation tests, guest
create/replace/shrink/empty, byte-level crash-prefix recovery, prior-generation
retention, full/fragmented refusal without disk mutation, and a deliberately
in-place production mutant that goes RED before exact restored GREEN.
