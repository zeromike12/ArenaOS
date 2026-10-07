# ADR-0095: Process-owned native VM reservations

**Status:** Accepted for Phase 13 implementation

**Date:** 2026-10-07

## Context

The Phase-12 runtime can allocate individual frames and ask the kernel to map
them at a kernel-selected address. That path has no release or protection
operation and its pointer-validation inventory was stored per scheduler thread.
Phase 13 moved that inventory to `Process` (ADR-0094), but a scalable heap,
guarded stacks, and ordinary process-wide mappings still need explicit
reservation and mapping lifetimes.

ArenaOS must keep authority in held capabilities and preserve the native ABI.
An address or reservation number alone must not authorize a mapping. The
kernel already owns each process PML4, enforces W^X in its user mapper, and
reclaims all user leaves when a process exits.

## Decision

Add a small native VM syscall family. It is not named after a POSIX interface:

| Operation | Effect |
|---|---|
| `SYS_VM_RESERVE` | Reserve a bounded range in the caller's own address space and return a non-copyable `VmRegion` cap, base, and page count. One unmapped guard page is placed on each side. |
| `SYS_VM_COMMIT` | Allocate zeroed private frames and map a bounded page run inside an exact held `VmRegion`. |
| `SYS_VM_PROTECT` | Change permissions for a fully committed page run in that held region. |
| `SYS_VM_RELEASE` | Destroy the exact held region cap, unmap and free its committed pages, and release its user-range record. |
| `SYS_VM_QUERY` | Describe one held region and current committed-page totals. |

Every reservation belongs to the process that created it. The cap has
READ|WRITE|DESTROY and no COPY right; no numeric ID, VA, or another process's
cap slot can operate on it. Commit and protection accept only READ, READ|WRITE,
or READ|EXECUTE. No writable-executable state is representable. MMIO and
SharedRegion capabilities are not accepted by this API.

The first implementation bounds a reservation at 4,096 pages (16 MiB), eight
regions and 8,192 committed pages per process, 128 live regions globally,
32,768 committed pages globally, and 64 pages per commit/protect operation.
These are admission limits, not eager allocations. A reservation consumes
metadata and virtual address space only; frames are allocated and zeroed on
commit. The VA arena is separate from the existing kernel-selected mapping
window and keeps guard and body ranges in the process-owned pointer inventory.

Admission checks cap-table, process-range, registry, VM budget, and page-table
capacity before publication. A failed commit frees every staged frame and
leaves PTEs and accounting unchanged. Once mapping begins, the IF=0 single-core
window and conservative page-table-frame preflight make the leaf installation
non-failing. Protection validates the full run before changing any PTE.
Release validates every committed leaf before unmapping any; an impossible
post-preflight frame-free error is a kernel invariant failure, not a partial
user-visible refusal. Process teardown frees leaves through the existing page
table walk and then forgets VM metadata, so backing pages are never freed
twice.

Pointer validation now requires both a registered process range and a present
user PTE for every page touched. This lets a reservation occupy its full
address range to protect it from competing mappings while uncommitted pages
and both guards remain invalid syscall buffers. Release removes the range and
the PTEs before returning, so stale pointers fail validation.

## Consequences

- Native runtimes can request larger lazy address ranges and release them
  without consuming a reserved capability slot as a transient frame holder.
- Heap and future thread-stack ownership can be tied to one process rather
  than a scheduler thread.
- Fixed metadata and commit limits make memory pressure visible and bounded.
- Page-table frames may remain cached after release under the existing page
  table policy; only identity-bearing mappings and backing frames must return
  to baseline.
- This ADR does not add ELF PIE, a dynamic linker, foreign ABI behavior, or
  POSIX `mmap` semantics.

## Validation

Phase-13 guest proof must reserve, commit, write/read, change protection,
refuse W^X and guard access, release, reject stale authority, and observe
identity-bearing resources return to baseline. The proof must run from ring 3
and use actual page tables and frame accounting.

The installed APB1 guest proof passed on QEMU 10.0.11 / OVMF 2025.02. It
checked zeroed lazy commitment, guard and uncommitted pointer refusal, RO/RX
protection, W^X and wrong-kind-cap refusal, exact release, stale-cap and
stale-pointer refusal, and restored global committed-page and region counts.
The current image also passed the M12 startup and 32-session regressions plus
20/20 fresh stability boots. The TLB invalidation path compares a changed
process root with the actual CR3, not the bootstrap kernel root.
