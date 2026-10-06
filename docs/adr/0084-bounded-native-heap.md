# ADR-0084 — Bounded native userspace heap

**Status: Accepted for Phase 12.5 (host invariants and exact single-thread guest proof pass; general VM and multi-user-thread integration remain open).**

**Date:** 2026-10-05. **Milestone:** Phase 12, native runtime.

**Related decisions:** ADR-0002, ADR-0015, ADR-0021, ADR-0022, ADR-0080, ADR-0083, ADR-0085.

## Problem

The native runtime needs fallible dynamic allocation for applications without
adding POSIX behavior or weakening capability ownership. Existing
`SYS_ALLOC_FRAME` and `SYS_MAP_MEMORY` can grant one owned frame, map it
writable/NX into the caller's own process, consume the cap, and leave process
teardown responsible for reclaiming the mapping. They do not provide a general
reserve/commit/protect/unmap VM API. The process entry's user-range table is
per thread, so mapping a page on one thread does not make that region valid as a
syscall buffer from another thread.

## Decision

- `arena-runtime` provides a reusable `BoundedHeap` implementing Rust's
  `GlobalAlloc`; it is selected explicitly by each native binary. It is a
  native ArenaOS allocator, not libc or a POSIX `brk`/`mmap` emulation.
- The heap lazily requests up to 32 unique 4 KiB pages per process using the
  existing `SYS_ALLOC_FRAME` then `SYS_MAP_MEMORY(slot, writable=1)` path.
  These are existing capability-native syscalls; this ADR adds no kernel
  syscall or kernel limit. The frame is mapped writable and NX, and the
  one-shot Untyped cap is consumed into the current process address space.
- Cap slot 63 is reserved transiently for this allocator's frame cap. ABI-v2
  startup reserves slot 0 and permits only sequential explicit descriptor
  slots 1 through 4. Applications using `BoundedHeap` must leave slot 63 free;
  an occupied slot makes allocation return null without replacing that cap.
  A failed map destroys the still-owned frame cap before returning null.
- Each mapped page is cleared before first use to prevent stale frame contents
  from crossing process lifetimes. The allocator supports requests no larger
  than 4,080 bytes and alignment no greater than 16 bytes. It uses in-page
  boundary headers, first-fit split, free-block coalescing and reuse; malformed
  metadata, unsupported layouts, page-table/map pressure, frame exhaustion or
  the 32-page bound fail closed as a null allocation. No allocator metadata is
  allocated recursively.
- Deallocation returns/coalesces a block for reuse but does not unmap an empty
  page. Ordinary-frame unmap is not present in the accepted kernel ABI; pages,
  leaf/table frames and map registrations are reclaimed by exact process
  teardown. Thus per-process heap backing has a visible 128 KiB high-water
  bound plus page-table overhead and is never silently returned to the global
  allocator while still mapped.
- The allocator lock serializes metadata across preemption. This does **not**
  qualify shared heap pointers for a multi-user-thread process: current
  `SYS_MAP_MEMORY` registration is per thread. ABI-v2 runtime/heap use is
  qualified for the current one-user-thread process model only. Thread
  creation remains gated on a process-wide mapping/range registry or an
  equivalent exact design and its own guest tests.
- Applications requiring graceful OOM must use fallible allocation APIs
  (`alloc` directly or `try_reserve`); the allocator reports capacity failure
  as null. It does not promise that infallible collection constructors convert
  OOM into a recoverable result.

## Verification

- Host tests cover 16-byte alignment, page scrubbing, split/coalesce/reuse,
  exact 32-page capacity, a mutation-free 33rd-page refusal, invalid layouts,
  and provider OOM. The 32-page proof uses synthetic page mappings and does not
  claim physical-capacity qualification.
- The independent M12 ring-3 guest uses the actual global allocator to fill and
  verify all 32 pages, write/read distinct patterns at both ends of each
  allocation, observe a null 33rd allocation, free/coalesce/reuse, then exit.
  The kernel checks exact frames, processes, mappings and shared resources
  after teardown. The current 512 MiB artifact passes M12 6/6, including four
  startup REDs and the reserved-slot cap-collision control, plus the bundled
  m1–m7/m11 regression markers.
- The guest is one-user-thread, ET_EXEC qualification evidence only. It does
  not establish VM reserve/commit/protect, TLS uniqueness across user threads,
  heap cross-thread mapping, process groups, or the final Phase-12 resource
  budget. Basic FS-base TLS is separately specified and boot-proven in
  ADR-0085.

## Consequences

- Native Rust applications now have a bounded, fallible `GlobalAlloc` option
  built only on existing frame and mapping authority. Heap size, supported
  alignment, page-count limit, reserved runtime cap slot, and teardown
  semantics are explicit and testable.
- The 32-page runtime ceiling does not enlarge the kernel's per-thread
  `USER_REGIONS_MAX=40`; ELF-segment/stack regions and other live mappings may
  cause an earlier clean OOM. The app manager must still include heap pages and
  page tables in launch and scalability accounting.
- A future exact ordinary-VM API may replace the page provider, but only after
  an inventory/VM ADR proves current mechanisms insufficient and defines
  reserve, commit, release, protection, guard, rights and teardown behavior.
  Do not raise kernel limits or treat this heap as a complete VM subsystem.
