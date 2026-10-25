# ADR-0111: Native stream ring counter wrap — versioned power-of-two layout (proposal)

Status: **Proposed.** Not accepted. Originally prepared by the C1/C2 runtime
workstream; revised 2026-10-24 during Phase 16 at C's recommendation to
evaluate a versioned layout with power-of-two capacity. **No production
wire format is changed by this revision.** Acceptance is M's decision (C
reviews); Phase 16 stream acceptance is blocked until M decides and the
chosen fix is implemented and qualified.

## Problem (reproduced, unchanged)

ADR-0104 streams (v1) keep `u32` write/read counters per channel and index
the data area with `counter % 768`. 2³² mod 768 = 256, so when a counter
wraps, the byte index jumps +256 instead of +1: unread bytes can be
overwritten and stream order breaks. Occupancy arithmetic
(`write.wrapping_sub(read)`) is wrap-correct; only the index mapping is
not. Host reproduction (still passing as a pinned defect):
`experiments/c-runtime/host/test_runtime.c` `test_ring_wrap` →
`KNOWN-DEFECT ring u32-wrap discontinuity reproduced`. Reach: ≈4 GiB on one
channel in one stream lifetime; silent corruption. Documenting a 4 GiB
limit is **not** an enforced safe boundary (nothing stops a producer at
the boundary), so a fix is required before stream acceptance.

## Proposed decision: ASTR v2 — keep u32 counters, capacity 1024

Make capacity a power of two so `index = counter & (capacity − 1)` is
continuous across *every* wrap of the u32 counter (2³² is an exact multiple
of 1024). No counter widening, no atomic-width change, no handshake.

### Exact v2 layout (one 4096-byte SharedRegion page, little-endian)

| Offset | Size | Field |
|---|---|---|
| 0 | 4 | magic `"ASTR"` (unchanged) |
| 4 | 4 | **version = 2** |
| 8 | 4 | channels = 3 |
| 12 | 4 | capacity = **1024** |
| 16 | 48 | reserved, must be zero |
| 64 | 1088 | ring 0 (stdin) |
| 1152 | 1088 | ring 1 (stdout) |
| 2240 | 1088 | ring 2 (stderr) |
| 3328 | 768 | reserved tail, must be zero |

Ring (stride **1088** = 17×64, 64-byte aligned, control first):

| Ring+ | Size | Field |
|---|---|---|
| 0 | 4 | write counter (atomic u32, producer-owned) |
| 4 | 4 | read counter (atomic u32, consumer-owned) |
| 8 | 4 | state (`WRITER_CLOSED=1`, `READER_CLOSED=2`, unchanged) |
| 12 | 52 | reserved, must be zero |
| 64 | 1024 | data bytes |

Total page usage 3328/4096 (v1: 2560/4096). Header and ring alignment are
identical to v1; only version, capacity, stride, and the index rule change.

### Arithmetic and full/empty rules (normative)

- `index(counter) = counter & 1023`. No modulo (also removes a division).
- `occupied = write.wrapping_sub(read)` (mod 2³²). Invariant:
  `occupied ≤ 1024`; `occupied > 1024` ⇒ Corrupt (unchanged error rule).
- Empty ⇔ `occupied == 0`; full ⇔ `occupied == 1024`. Because
  `occupied` is computed mod 2³² and bounded by the producer to ≤1024,
  empty/full are unambiguous at any counter age, including exactly at
  2³², 2³³, …; there is no total-lifetime limit.
- Producer: copy ≤ `1024 − occupied` bytes at `index(write)` (two-copy
  span rule as v1, with the mask), then release-store
  `write.wrapping_add(n)`. Consumer: acquire-load write, copy from
  `index(read)`, then release-store `read.wrapping_add(n)`. Ordering
  rules are unchanged from v1.
- `WRITER_CLOSED`/`READER_CLOSED`, `WouldBlock`/`BrokenPipe`/EOF rules:
  unchanged. Wake badges: unchanged.

### Compatibility and refusal

- `version` is the only selector. v2 readers **refuse version ≠ 2** (and
  mismatched channels/capacity/reserved bytes) with `BadFormat` — as v1
  already refuses version ≠ 1 (`streams.rs::attach`). There is **no
  dual-format reader**: mixed old/new endpoints on one page must fail
  loudly at attach, never misinterpret.
- Cutover is atomic with the OS image: stream pages are created per
  launch and are not persisted, so no migration of live v1 pages exists.
  Every component below must land in the same commit range that flips
  the constants; a partial flip creates attach failures, not corruption.
- Unknown future versions refuse identically.

### Coordinated change inventory (all must agree)

| Component | Change |
|---|---|
| `userspace/arena-runtime/src/streams.rs` | `VERSION 1→2`, `STREAM_CAPACITY 768→1024`, `%`→`&`, tail-reserved validation, docs |
| `userspace/desktop` (`app_client.rs`, `bin/desktop.rs`) | producer (stdin) / consumer (stdout, stderr) endpoints — inherit via the crate; verify no local 768 assumptions |
| `userspace/phase13-app`, `userspace/phase13-headless` | guest consumers/producers — inherit via the crate; retest |
| C runtime `src/streams.c`, `src/stdio.c` (+ `include/arena/abi.h` capacity constant, `run.py abi`) | header/capacity/stride constants, mask indexing, v2-only accept |
| Test/diagnostic tooling and harnesses referencing 768/832 (grep-gated in the landing commit) | update constants and expectations |

Kernel: **no change** (the kernel never interprets the page). ADRs 0104/0105
get a dated addendum pointing here when accepted; they are not edited
retrospectively.

### Resource/buffering consequences

Per-stream buffering rises 768→1024 B/channel (2304→3072 B per direction
set); page count unchanged (1). Copy chunking above ring size is unchanged
in shape. Mask indexing is cheaper than modulo. No limit was raised to
make a test pass; 1024 is the smallest power of two ≥ v1 capacity that
keeps all three rings + header in one page (2048 would need 6400 B > 4096).

### Tests (prepared now; landing tests run at adoption)

Prepared without touching the production format:

1. **v1 pin (exists, keeps passing as KNOWN-DEFECT):** the host wrap test
   above; at adoption it converts into the negative control (the same
   seeded scenario under v1 logic must corrupt, under v2 must not).
2. **Standalone v2 model test (Phase 16, host, `test_stream_v2_model.c`):**
   implements the proposed layout in isolation and proves, with counters
   *seeded* at 2³²−16: 64 bytes written across the wrap are read back in
   order with unread bytes intact; full at exactly 1024 across the wrap;
   empty/full across two full wraps (counters seeded, then 3 MiB pumped);
   and the identical scenario under v1 arithmetic is detected corrupt
   (negative control). Seeding counters directly is legitimate: counters
   are plain page fields the kernel never interprets.
3. **At adoption (required evidence):** Rust + C host suites with seeded
   wrap cases; guest test with a self-created stream page seeded near the
   wrap (desktop-mediated streams cannot be seeded by the app, so the
   guest wrap proof uses an arena-runtime `StreamSet::initialize` page in
   a test process, plus the normal desktop stream suites for behavior);
   v1-page refusal by v2 readers (guest + host); full registered
   regression re-run.

## Alternatives (kept from the original proposal)

1. **u64 counters (v2-alt):** index still discontinuous at 2⁶⁴
   (2⁶⁴ mod 768 = 256); unreachable in practice but the mapping remains
   non-continuous by construction, and atomics widen on the shared page.
2. **Quiescent reset handshake:** adds a protocol round that does not
   exist today; more states to qualify.
3. **Enforced 4 GiB boundary on v1** (producer refuses at 2³²−768):
   honest but creates a hard stream-lifetime failure SDL3-era apps would
   hit; the layout fix costs the same coordination and removes the bound.

## Decision requested

M: accept ASTR v2 as specified (capacity 1024, mask indexing) for
implementation in Phase 16, or select an alternative. Until then this ADR
stays Proposed, production remains v1, and Phase 16 presents streams as
**not acceptance-ready**.
