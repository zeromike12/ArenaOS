# ADR-0111: Native stream ring counter wrap (proposal, blocker)

Status: **Proposed.** Not accepted. Prepared by C1 (C runtime) for review by
the Phase-13 stream owners (M) and the C-runtime owners (C). Number 0111 is
the next free number on this branch; if another branch lands 0111 first,
renumber at integration. Accepted ADRs are never renumbered.

## Problem

The ADR-0104 native byte-stream ring keeps a write counter and a read counter
per channel as `u32` values, and indexes the 768-byte data area with
`counter % 768` (both in `arena-runtime` `streams.rs` and in the C binding
`experiments/c-runtime/src/streams.c`).

2^32 = 4 294 967 296. 4 294 967 296 mod 768 = 256, so 2^32 is **not** a
multiple of the capacity. When a counter wraps from 0xFFFFFFFF to 0, the byte
position `counter % 768` jumps by 256 (from `0xFFFFFFFF % 768 = 255` to
`0 % 768 = 0`), instead of advancing by one. Bytes that have not been read yet
can be overwritten, and the order of bytes in the stream can break.

Evidence:

- Host (HOST-ONLY): `experiments/c-runtime/host/test_runtime.c` `test_ring_wrap`
  reproduces the discontinuity and pins it as
  `KNOWN-DEFECT ring u32-wrap discontinuity reproduced: unread bytes overwritten`.
  `run.py host` reports `HOST-ONLY RESULT PASS (598 checks, known protocol defect pinned)` on commit `2e8fae6`.
  The pin is a regression detector, not a pass of correctness.
- Reach: about 4 GiB written on one channel in one stream lifetime. A
  long-running desktop or media stream can reach this. The defect is silent.
- No guest run has reached 2^32 bytes. Guest status: not reproduced in
  ArenaOS, and not expected in any C1 test.

## Options

1. **Widen the counters to u64** (wire-format version 2). Header gains a
   version bump; old readers refuse version 2 rather than misread it. The
   discontinuity does not disappear: `2^64 mod 768` is also 256, so the
   `counter % 768` mapping still jumps at the 2^64 wrap. That wrap is
   unreachable in practice (about 584 years at 1 GB/s), and this option must
   say so explicitly rather than claim the mapping is continuous. Cost: a
   header and layout change.
2. **Use a power-of-two capacity** (for example 512 or 1024 bytes per ring,
   replacing 768). `2^32` is then a multiple of the capacity and `counter %
   capacity` is continuous across the wrap. Cost: changes the ring size and
   the layout offsets (`64 + 832·i`), so it is also a wire-format change.
3. **Keep u32 counters and reset them** when a ring is empty and both sides
   are quiescent. Cost: a protocol handshake that does not exist today.

Option 1 is the smallest honest change and is recommended for review. Option
2 changes capacity and so affects the SDL3 handoff's buffering assumptions.

## Decision (requested)

This ADR asks the reviewers to choose option 1 or 2 and to schedule the
change. It does **not** land a change. The C1 prototype keeps the ADR-0104
wire format, pins the defect in host tests, and documents the blocker in
`docs/compat/C1-FINAL-REPORT.md` and `docs/compat/C1-INTEGRATION-CONTRACT.md`.

## Consequences if not fixed

- C binding and Rust binding both inherit the defect; they must not be
  advertised as unbounded-stream safe until it is fixed.
- The SDL3 handoff must list "streams are not safe past 4 GiB per channel
  lifetime" as a blocker for long-running applications.

## Review questions

1. Option 1 (u64 counters, version 2) or option 2 (power-of-two capacity)?
2. Should readers refuse unknown stream versions outright? (Recommended: yes.)
3. Who owns the Phase-13 `streams.rs` change: M or the stream owners?
