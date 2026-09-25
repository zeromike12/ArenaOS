# ADR-0009: Kernel heap — host-tested free-list core, frame-backed chunks, always-on guards

Status: accepted (M2.5, 2026-09-25)
Context refs: ADR-0004 (no third-party crates), ADR-0007 (physical memory), ADR-0008 (address spaces)

## Problem

Boot-stage subsystems so far live in static structures. Everything after
M2.5 (scheduler task lists, driver buffers, page-table demand growth) needs
general-purpose memory. Requirements: real allocation semantics (free must
return memory for reuse), corruption detection from day one, and logic that
can be tested without burning a QEMU boot per experiment.

## Approaches considered

1. **Bump allocator, never free** — trivial, but every test leak is
   permanent; unusable for long-lived subsystems and hides use-after-free
   completely.
2. **Buddy allocator directly over frames** — classic, O(log n) splits,
   good for page-granular work but internally fragmentsing for small
   kernel objects (rounds every allocation to a power of two); duplicates
   machinery the frame allocator already has for large blocks.
3. **Segregated storage (slab/magazine)** — best throughput; substantial
   complexity (size classes, per-CPU caches, refill paths) that cannot be
   justified before there is any load to serve.
4. **First-fit free list with boundary tags + coalescing, chunked over
   frames** — moderate complexity, exact-size allocation, O(n) operations
   that are perfectly fine at boot scale, and small enough to fully
   verify on the host.

## Decision

Option 4, split into two pieces per the repo-boundary rules:

**`kernel/libs/heap` (crate `arena-heap`)** — the entire allocator, zero
dependencies, `no_std`, and *host-testable*: `cargo test -p arena-heap
--lib --target x86_64-unknown-linux-gnu` runs the full logic suite
natively (10 tests, including a 4000-round pseudo-random stress checked
against a `BTreeMap` reference model for overlap, corruption, and
accounting). This is the first member of `kernel/libs/` — the pattern
ADR-0007 announced for shared logic, now realized.

Design points:

- **Blocks**: 32-byte header (`footprint` with FREE flag in bit 0,
  `user_size`, front red-zone word, free-list link). Footprints are
  multiples of 16, so the low 4 bits carry flags. The free link lives in
  the *header*, not the payload — poisoning therefore covers every user
  byte, including byte 0 (a classic free-list allocator cannot poison the
  first 8 bytes of freed blocks; we can).
- **Free list**: address-ordered, first-fit, both neighbours coalesced on
  free. Invariant: no two free blocks are ever adjacent. Leftovers below
  `MIN_FREE_FOOTPRINT` (32 B) are absorbed as internal padding instead of
  creating unusable free blocks.
- **Chunks**: 64 KiB, never shrunk, never split across blocks. Supplied by
  a `ChunkProvider` trait — the boot implementation carves physically
  contiguous frame runs (16 × 4 KiB) from ADR-0007's allocator. Maximum
  single allocation ≈ `chunk_bytes − 64`.
- **Guards** (boot stage: always on): front red-zone word in the header +
  8-byte red zone immediately after user data, `0xAA` fill on alloc,
  `0xDD` poison on free. Violations are detected at the *free that
  follows* the corruption — earliest deterministic moment without
  hardware watchpoints.
- **Validation**: `free` proves the pointer is a live block header by
  walking the chunk block chains; interior, foreign, and wild pointers are
  rejected. A rejected free mutates nothing (the block stays live);
  rejections are returned as typed `HeapError` values, never panics.
- **Alignment**: requests above 16 shift the header down so the payload
  lands exactly on the boundary, splitting front slack when it can form a
  real free block.

**`kernel/boot/src/heap.rs`** — glue only: the `FrameChunkProvider`, one
`static HEAP` in a `SyncCell`, the alloc/free API, stats, and error
logging. No policy in the boot crate.

Explicit decisions *not* taken:

- **No `#[global_allocator]` yet.** The boot stage has no `Box`/`Vec`
  users; wiring the global allocator drags in OOM policy (panic vs
  abort) and lock questions that belong to the kernel proper. The
  explicit API keeps every allocation site visible. Revisit with M2.6
  locks in place.
- **No locks inside the core.** The heap documents a single-threaded
  contract; the boot stage satisfies it trivially (single CPU, IF=0). The
  kernel proper will wrap it with the M2.6 spinlock rather than paying
  atomics on every operation forever.

## Reasoning

- Correctness-first: the whole point of M2.5 is a heap you can *trust*;
  host-side model-checked stress found real bugs pre-boot (a tail-absorb
  condition that refused near-chunk allocations, and a coalescing path
  that wiped the free-list link) that would have cost serial-archaeology
  sessions as guest #PFs.
- Guards always-on at boot costs 8 bytes per allocation and two fills —
  irrelevant at this scale, and it catches corruption at its causal free
  instead of three subsystems later.
- Chunks over contiguous frames keeps the heap inside the documented
  identity-mapped RW+NX conventional memory; no paging cooperation
  required, and M2.4's tables already cover every frame.
- `kernel/libs/` split makes the allocator the project's first
  unit-tested crate and sets the template (pure logic + trait seams for
  machine-specific bits).

## Measured behavior (evidence, not promises)

Host: 10/10 tests green, including 4000-round LCG stress vs a reference
model (no overlaps, tags intact, accounting exact), growth/exhaustion,
alignment, guard rejection, and cross-provider portability (System-backed
chunks).

Guest (14/14 M2): `heap_basics` (distinct/aligned/patterned, accounting
to zero); `heap_guards` — serial shows the three deliberate probes:

```
[arena ERROR heap] free rejected: DoubleFree ptr=0x108040
[arena ERROR heap] free rejected: RedZoneBack ptr=0x108040
[arena ERROR heap] free rejected: NotFromHeap ptr=0x1dd1d0e0
```

`heap_stress`: 1920 alloc/free ops in 1270 µs (~661 ns/op under TCG,
guards + validation walks included), 1 chunk, accounting restored
exactly, post-stress 16 KiB allocation proves coalescing.

## Disadvantages / limits

- O(free list) alloc and O(blocks-in-chunks) validation walk. Fine at
  boot scale; wrong for a hot kernel allocator — segregate later.
- No chunk shrink; freed chunks stay reserved (frame-level reuse is a
  future concern).
- Single allocation cap: `chunk_bytes − 64` (65,472 B at 64 KiB chunks).
  Large blocks should come from the frame allocator directly.
- Alignment ≥ chunk usable span is unsatisfiable (returns None) — 4 KiB
  alignment inside 4 KiB host-test chunks, for instance.
- Guard fills cost bandwidth on every alloc/free; acceptable now, a
  build-time knob later.

## Revisit triggers

- Kernel proper wants `Box`/`Vec` → global-allocator wiring + OOM policy
  (with M2.6 locks).
- Allocation throughput becomes measurable overhead → size-class caches.
- Long-lived kernel needs memory returned to the frame pool → chunk
  teardown for fully-free chunks.
- SMP → locking (or per-CPU heaps) before any of the above.
