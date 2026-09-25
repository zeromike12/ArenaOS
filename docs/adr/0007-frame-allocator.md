# ADR-0007: Physical frame allocator — flat bitmap over conventional memory

Status: accepted (M2.3, 2026-09)

## Problem

The kernel needs a physical memory allocator the moment it owns memory:
page-table pages (M2.4), kernel heap (M2.5), and everything after. The
source of truth is the UEFI memory map: only EfiConventionalMemory is
allocatable pre-reclaim; runtime services, ACPI NVS, reserved/MMIO spans,
and firmware-owned boot-services memory (until ExitBootServices) must never
be handed out. Requirements at this stage: correctness over cleverness,
zero dependencies, works in the boot stage's static-memory world, honest
failure modes (double free, exhaustion), and measurable throughput.

## Approaches considered

1. **Flat bitmap, one bit per 4 KiB frame** over the physical span
   [0, 4 GiB): 128 KiB static .bss; regions "punched in" at init;
   first-fit with a roving 64-bit-word hint; O(1) free; double free and
   out-of-span frees are checked errors.
2. **Buddy allocator** over region lists: O(log n) alloc/free, natural
   split/merge, the classic kernel choice (Linux, FreeBSD). Cost: real
   complexity (region spanning, order bookkeeping, coalescing invariants),
   per-order free lists needing their own storage, and it shines when
   large aligned blocks and fragmentation pressure dominate — which they do
   not at M2 scale.
3. **Sorted hole list** (first-fit over free extents): tiny metadata, but
   O(holes) operations with mutation-heavy bookkeeping — the worst
   correctness risk per line of code of the three.

## Decision

Flat bitmap (approach 1), with the core bit logic written as operations on
a plain `[u64]` so it can move wholesale into a host-testable crate at the
M2.7 boot split.

## Reasoning

- **Correctness first.** The bitmap has one invariant (bit set ⇔ frame not
  hand-out-able) that is trivially auditable against the memory map — the
  M2 test recomputes the managed frame count from the raw map and demands
  exact equality, then verifies uniqueness, in-region placement, real-RAM
  pattern round-trips, exact accounting across a 200-round stress, and
  double-free rejection.
- **Measured performance is a non-issue at this scale.** In-guest benchmark
  (QEMU TCG, logged by `m2:test:frame_allocator`): ~100 ns per hot-path
  alloc+free pair (2048 pairs in 206 µs). Boot and M2–M4 workloads allocate
  thousands of frames, not millions per second.
- **Metadata cost is fixed and tiny**: 128 KiB for 4 GiB of coverage
  (0.003%), versus buddy's per-region/per-order structures.
- **It degrades predictably**: worst case is a full 16384-word scan
  (~µs), bounded and rare thanks to the roving hint.

## Disadvantages accepted

- **4 GiB coverage cap** at M2 (bitmap is static .bss sized for it). On
  this platform (512 MiB VM) nothing conventional lives above 4 GiB; the
  init clips honestly and the cap is documented in code. High-memory
  support = second-level bitmap or dynamic storage — see revisit triggers.
- **First-fit fragmentation**: long-lived small allocations can sprinkle
  the map; `alloc_contiguous` is a linear scan, fine for the small runs
  boot needs (page-table pages are single frames), not fine for huge
  contiguous demands.
- **Single-CPU only**: no locking by design (boot contract). The kernel
  proper replaces this with a synchronized or per-CPU allocator (M2.6's
  lock primitives + M3 SMP structures) — bitmap *layout* survives that
  change; the mutation discipline does not.

## Revisit triggers

- Physical memory above 4 GiB becomes reachable on a target.
- The heap (M2.5) or graphics (Phase 9+) shows real demand for large
  contiguous runs (> 64 frames) at rate.
- SMP boot (M3+): allocator gets locking or per-CPU magazines; re-benchmark
  bitmap vs buddy *then*, with the workload that exists then.

## Implications

- `frames.rs` is the single gateway to physical memory: nothing else may
  hand out physical addresses (tests included — they go through it).
- The allocator's own storage is in the boot image (LoaderData), so it can
  never overlap a managed frame — no self-hosting bootstrap problem.
- M2.4 page tables, M2.5 heap, and M2.7's boot-info record all consume
  `frames::alloc()`; the boot-info record must carry enough map detail for
  the kernel proper to *rebuild or inherit* this allocator after
  ExitBootServices (reclaim adds boot-services spans back in).
