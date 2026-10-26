# Phase 16 C runtime provenance and integration-change log

## Source of the import

- Upstream branch: `arena/f89ea987-arenaos`, reviewed HEAD
  `84392b8f32a0b1ce26364926185723c7bbf7a3f3`. Arena 2's evidence of record
  is `9c93f8cd3b933978aaa67357beac55a89b6a6bc1`; the commits between
  `9c93f8c` and `84392b8` are documentation-only (Arena 2's
  `C2.1-FINAL-REPORT.md` records the diff check; re-verifiable with
  `git diff --stat 9c93f8c..84392b8 -- experiments/c-runtime`).
- Import method: `git checkout 84392b8… -- experiments/c-runtime docs/compat
  docs/adr/0111-… docs/adr/0112-…` into this branch at the **same paths**.
  The historical Arena 2 branch was not merged. Nothing else was imported
  (no kernel, userspace, tools, or Arena 1 graphics code).
- Canonical location for Phase 16: `experiments/c-runtime/` (unchanged
  path, so every Arena 2 harness, receipt, and doc reference stays valid).
  Promotion out of `experiments/` is an M decision, not assumed here.
- Third-party code: `third_party/xxhash-0.8.4/` (BSD-2-Clause; see its
  `PROVENANCE.md`/`SHA256SUMS`), imported unchanged, test-app use only.

## Post-import reproduction against Phase 15 `main` (P16.2)

Run in this sandbox on the imported tree, before any integration edit:

| Check | Result | vs Arena 2 receipt |
|---|---|---|
| `run.py abi` (C constants vs `userspace/abi.rs`) | 92 constants, mismatches `[]` | identical |
| `run.py host` (ASan+UBSan) | LIBC-HOST checks=1099 failures=0; HOST-ONLY PASS (1697 checks, known defect pinned) | identical counts |
| `run.py --cc gcc/clang build` | rc 0 both | — |
| `c2-console-gcc.elf` SHA-256 | `947a56d1…a6b5` | **bit-identical** |
| `c2-console-clang.elf` SHA-256 | `1e988d28…349c` | **bit-identical** |
| `libarena_c.a` (gcc/clang) SHA-256 | `bd3b5327…38d5` / `c583a776…43c38` | **bit-identical** |
| `c2_isa_audit.py` (8 ELFs) + `--self-test` | PASS | identical |
| `guest_c2.py --self-test` | 9/9 | identical |
| Undefined symbols | gcc: none; clang: weak `arena_legacy_write` | defect reproduced → fixed, see log #2 |

Conclusion: Phase 15 changed no syscall number, startup constant, or loader
rule the runtime depends on; the runtime needs no adaptation to *build*
against `main`. Older Arena 2 statements that FP/SIMD is unavailable are
superseded by ADR-0113 and corrected wherever Phase 16 text cites the
profile.

## Integration-change log (every intentional divergence from `84392b8`)

1. **`tools/startup_abi.py` M9 alignment** (repo tool, not runtime):
   `CAP_MAX 4 → 7`, capability kind 16 (SyncDomain) accepted, roles 6/7/8
   with the native validators' kind/rights rules, `KNOWN_FLAGS` mask
   `7 → 31` with flag↔role agreement — all mirroring
   `userspace/arena-platform/src/startup.rs` and the C validator, which
   already enforced exactly this. Host-only oracle; no wire change; no
   native validation weakened. Regression tests in
   `tools/test_startup_abi.py` (7-descriptor stream+sync record
   round-trips; 8 descriptors, unknown kind 14, unknown flag, and
   flag-without-role all still refuse).
2. **Weak `arena_legacy_write` removed** (`src/rt.c`, `src/internal.h`,
   `src/legacy_debug.c`): `rt.o` now dispatches through a plain hook
   pointer that only `arena_legacy_serial_enable()` sets, so ordinary
   images carry **no** reference to the legacy sink under either compiler
   (Arena 2's proposed fix, C2.1-VERIFICATION §3). The false comment
   claiming ordinary images "never reference" the symbol is corrected.
   `SYS_DEBUG_WRITE` reachability is unchanged (still only via the
   explicit legacy opt-in object).
3. **FP/SSE2 test application + builder added** (`app/c_fp_app.c`,
   `build_fp_app.py`, `guest_fp.py`): a new guest app exercising x87/SSE2
   value preservation across C11-thread yields and blocking transitions
   under the ADR-0113 baseline profile (`-march=x86-64 -msse2 -mno-avx*`).
   The canonical runtime/app builds keep their integer-only flags and ISA
   audit unchanged; the FP app is built separately with a *positive*
   audit (SSE2 expected, AVX refused). The tightened app initializes all
   inline-assembly operands and the judge scopes skip detection to
   `[c-fp]` lines. Canonical GCC/Clang guest PASS at `d314d20…` (positive
   `66/0/0`, isolated mutant `66/19/0`).
4. **ASTR v2 coordinated cutover** (accepted ADR-0111): Rust
   `userspace/arena-runtime` stream code/tests and C headers/sources
   (`include/arena/abi.h`, `src/streams.c`, startup validation, host
   models, and guest app layout constants) moved from three 768-byte
   v1 rings to version-2, 1,024-byte power-of-two rings with wrapping
   `u32` occupancy and mask indexing. V1/unknown/wrong-capacity pages
   are explicitly refused. Host coverage includes unread bytes across
   the counter wrap and a v1-indexing negative control.
5. **Focused live stdin** (`app/c_stdin_app.c`, `guest_stdin.py`, plus
   Desktop routing outside the imported runtime): a pre-created console
   window routes keyboard bytes only to the focused owning AppInstance
   and Ctrl-D closes only that instance's stdin writer after queued
   bytes drain. The suite also proves no-grant refusal, burst/backpressure,
   two-instance isolation, EOF counts, and ASTR-v1 refusal.
6. **Self-identifying evidence runners** (`guest_phase16_dev.py`,
   `guest_phase16_canonical.py`): the development runner records its
   non-qualifying B1 classification; the canonical runner does not
   substitute `mtest.build` and records B1/component, source-cleanliness,
   protected-identity, image, backend, firmware, compiler, app, command,
   and verdict data in every summary.
7. **Notification ambiguity guest fixture** (`app/c_notification_app.c`,
   `guest_notification.py`): new signed streams-only probe and restored
   Desktop mutation suite. No production runtime selection change was
   needed; the fixture creates the otherwise unreachable two-candidate
   record only for the labelled mutant boot and asserts byte-exact
   source/artifact restoration.
8. **Integration fixes found during guest regression:** Desktop
   pre-created windows no longer spend their one window wake before app
   adoption (I-8), and Phase 13's harness waits for lagging search input
   to settle before clicking (I-9). These are recorded because they are
   Phase 16 integration divergences outside the imported C runtime tree.

The original statement that items 1–3 were the only divergences was
true at the P16.2 snapshot and is superseded by items 4–8 above. Use
`git log f103926..d314d20` and `git diff 84392b8..d314d20 --
experiments/c-runtime` (after fetching the Arena 2 branch) for the full
machine-checkable divergence; canonical evidence is indexed in
`receipts/canonical-2026-10-26/README.md`.
