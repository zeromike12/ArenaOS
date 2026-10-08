# ADR-0096: Scalable lazy native application heap

**Status:** Accepted; installed-app heap proof and 20/20 T3 preservation passed.

## Context

ADR-0084 deliberately limited the Phase-12 process allocator to 32 pages. It
uses the old frame/map syscall pair and keeps a runtime-specific cap slot
reserved as a transient frame holder. That is useful for the startup ABI proof,
but too small and too coupled to a capability slot for ordinary native
applications.

ADR-0095 adds process-owned guarded VM regions, lazy zeroed commitment,
protection changes, exact release, and page accounting. The native heap can use
that API without introducing a new kernel allocator or changing the Phase-12
startup contract.

## Decision

Keep BoundedHeap and its 32-page startup proof unchanged. Add an opt-in
ScalableHeap for applications that need a larger Rust global allocator.

The heap reserves one guarded 4,096-page region (16 MiB) on its first
successful allocation. Reservation allocates no backing frames. A small
allocation commits one zeroed page on demand and uses the existing 16-byte
block header, split, and coalesce logic. Larger and over-aligned allocations
use contiguous page runs with a private header. Commit requests are divided
into operations of no more than 64 pages. Layout alignment is supported up to
2 MiB. Oversize, unsupported, metadata, and VM allocation failures return a
null pointer through GlobalAlloc; fallible Rust allocation is tested through
Vec::try_reserve_exact.

Freed small blocks coalesce. Freed large page runs return to the allocator's
free-page state. Committed physical pages are retained so later allocations
can reuse them; only exact region release or process teardown returns them to
the frame allocator. Reused allocations are uninitialized, as required by
Rust's allocator contract. First commitment is zeroed by the kernel.

Fixed per-process bookkeeping is 28,688 bytes for the 4,096-page classification
and head/size arrays plus the region base/capacity. It is ordinary application
BSS and does not commit the heap's 16 MiB virtual capacity or its backing
frames. The allocator serializes its metadata and VM operations with one
process-local spin lock so it remains valid when user threads arrive.

The allocator holds the exact non-copyable VmRegion capability returned by
the kernel. It does not use slot 63 or overwrite any occupied capability. The
heap's descriptive base and page indices do not authorize mappings; all
commitment and teardown still pass the exact held region cap.

## Consequences

- The Phase-12 32-page behavior and all existing startup red/green controls
  remain available without modification.
- Native applications can reserve up to 16 MiB of virtual heap capacity
  without upfront frame commitment. The VM API currently allows at most 8,192
  committed pages per process, 32,768 globally, and 64 pages per operation.
- Deallocation reuses committed backing but does not decommit individual heap
  pages. A process that retains its allocator also retains that backing until
  exact heap release or process teardown.
- Runtime resource queries report reserved capacity and the kernel's actual
  committed-page count alongside currently allocated small/large pages.
- This is a native allocator contract. It does not adopt POSIX allocation or
  virtual-memory semantics.

## Validation

Host tests cover small-block reuse/coalescing, a 256 KiB allocation across a
64-page operation boundary, over-alignment, page-run reuse, and mutation-free
oversize refusal. A real installed APB1 application uses Vec, touches every
page of a 256 KiB allocation, checks VM accounting, drops and reallocates the
same run, requests 64 KiB alignment, and verifies fallible OOM. The guest also
observes clean ProcessGroup teardown.
