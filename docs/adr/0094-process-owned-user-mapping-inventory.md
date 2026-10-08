# ADR-0094 — Process-owned user mapping inventory

**Status:** Accepted; process-owned mapping guest regressions and 20/20 clean boots passed.
**Date:** 2026-10-07.
**Related decisions:** ADR-0014, ADR-0021, ADR-0075, ADR-0088, ADR-0089.

## Problem

The PML4 and capability space belong to `Process`, but the page-granular
regions used by every syscall buffer check were stored in `KThread`. A second
thread in the same process would therefore have the same page tables and a
different view of which pointers were valid. This also made `SYS_MAP_MEMORY`
and `SYS_SHARED_MAP` mutate metadata that only the calling thread could see.
That ownership mismatch must be resolved before ring-3 threads can safely
share an address space.

## Options considered

1. Keep per-thread ranges and copy every mapping mutation to all current and
   future threads. This duplicates state, creates partial-update failure
   cases, and requires thread creation to inherit a synchronized snapshot.
2. Move only syscall validation to the PML4 page tables. This would require
   walking page tables for each user buffer and still would not distinguish
   normal RAM, device mappings, guard reservations, or unmapped reservations.
3. Store the bounded pointer-validation ranges on the owning Process and let
   all process threads resolve that one inventory.

## Decision

- Keep the existing 80-span bound and move its production owner to
  `Process::user_regions`. Do not raise process, mapping, page, or capability
  limits in this change.
- `sched::current_user_regions`, `set_current_user_regions`, append, and exact
  removal route through the current thread's Process when `proc_id != 0`.
  `user_range_ok` and both mapping syscalls therefore see the same ranges from
  every thread that shares the address space.
- Remove the 80-span array from every `KThread`. Kernel-owned ring-3 self-test
  contexts (`proc_id == 0`) use one separate scheduler-global range table;
  production userspace cannot enter that context or use it as ambient memory
  authority.
- Existing SYS_MAP_MEMORY and SharedRegion mapping behavior remains intact.
  This ADR changes mapping metadata ownership only; it does not yet add
  reserve, commit, protection, guard, or private-release operations.
- Process destruction continues to tear down the PML4 and SharedRegion pins
  through their existing owners. The range inventory disappears with the
  Process table entry.
- The kernel is single-core today, so IF=0 still makes map selection and
  registry mutation one atomic decision. Any SMP scheduler must replace this
  exclusion with address-space synchronization before enabling concurrent
  mapping syscalls.

## Reasoning

The process is already the owner of both the address space and capability
space. Keeping the region inventory beside those resources gives every thread
one coherent view and avoids inheritance races. Moving the array replaces an
equally bounded per-thread allocation with a per-process allocation, so the
fixed maximum number of range records remains the same rather than multiplying
with the number of user threads.

## Downsides accepted

Kernel-only ring-3 self-tests retain a separate global range set because they
do not have a Process object. Those tests execute serially; if kernel-managed
ring-3 contexts ever run concurrently, they will need explicit owners too.
Every syscall pointer check still copies the bounded 80-entry table, matching
the prior copy cost. A later mapping object table will carry permissions,
reservation state, and resource accounting without changing this ownership
rule.

## Future implications

This is a prerequisite, not a complete VM interface. The next native VM
decision must define process-owned reservation/mapping records, lazy page
commit, release and protection changes, guard ranges, W^X, and accounting.
User-thread creation must inherit the process address-space root and use this
same table for pointer validation; it must not create a second process or a
per-thread mapping view.

## Evidence

The unchanged M12 startup guest passed 7/7 startup checks plus M1–M7 and M11.
The Phase-12 32-session guest passed its mutation-free 33rd refusal, 16-close /
16-reuse cycle, and exact identity cleanup; it observed 77 retained page-table
frames after teardown, within the existing budget. The Phase-13 installed-app
guest passed signed install, registry launch, exact document handoff, and
ProcessGroup cleanup on this kernel. The source-built EFI passed 20/20 fresh
stability boots with zero failures; receipt:
`256a03461bf1eb6f2c85595f9ff347fd42dd7272e7328d7b1f27088a02bbf515`.
