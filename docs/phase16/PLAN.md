# Phase 16 — Native C runtime integration: plan

Status: **IN PROGRESS. Not accepted.** M is project lead and final
architecture/acceptance authority; C is technical advisor and independent
reviewer. This plan does not accept ADR-0111, ADR-0112, or any wire-format,
capability, or architecture change. Nothing here starts Phase 17.

**Status update (2026-10-25):** M has since accepted ADR-0111 (ASTR v2)
and ADR-0112 (bounded native C-runtime profile) as architecture/contracts
only. That acceptance does **not** accept the implementation, qualify
Phase 16, or authorize merging. ASTR v2, the tightened FP contract, the
focused live-stdin console path, self-identifying development evidence,
and a Phase 13 harness synchronization fix are implemented in commits
`d5b8c00`, `1513ba2`, `0cfb6de`, `faccf87`, and `a7ad4c2`. Fresh
development-only guest receipts are in
`receipts/dev-guest-2026-10-25-reconstructed/`; B1 (official Rust target
components) still blocks canonical qualification. The pinned-QEMU backend
blocker B2 is resolved through the isolated glibc runtime (host glibc
untouched). Statements below that call the ADRs merely proposed or both
blockers unresolved are the original plan text and are superseded by this
update where they conflict.

**Canonical qualification update (2026-10-26):** C supplied the
official Rust 1.97.0 components in `02a20fc`; B1 is resolved. At clean
source `d314d204aa86ec4018659f3f211cd3a488d30d87`, the protected Phase
14 identities reproduced from a physical `/workspace/ArenaOS` bind
mount; canonical targeted C1/C2/FP/live-stdin/Notification suites pass
for GCC and Clang; registered regressions pass 121/121; strict Phase
15 passes on the final and preserved packages; and the final EFI/ESP
completed 100/100 stability boots. Receipts are in
`receipts/canonical-2026-10-26/`; the engineering submission is
`FINAL-REPORT.md`, with `ARENA1-HANDOFF.md` for the proposed next
phase. This is a qualification candidate for M/C review only — it is
not acceptance and does not authorize merging, a release, or Phase 17.

## 0. Branch note (read first)

The assignment directs work onto `main`. This engineering session is pinned
by the Arena platform to branch `arena/2b4ae1a5-arenaos` (Arena tracks the
session by that branch; the agent cannot switch, create, or push to another
branch). All Phase 16 commits therefore land on `arena/2b4ae1a5-arenaos`,
based exactly on `main` at `f103926d6b28d546fd6a9a32d2bf5f63a16d618b`
(merge of PR #4), and are offered to `main` by pull request for M/C review.
No other agent's branch is modified; no history is rewritten; no release is
published; the milestone is not declared accepted.

## 1. Goal (bounded)

Integrate Arena 2's existing native C runtime (`arena/f89ea987-arenaos` @
`84392b8f32a0b1ce26364926185723c7bbf7a3f3`, evidence commit
`9c93f8cd3b933978aaa67357beac55a89b6a6bc1`) into the accepted Phase 15
foundation as **one canonical C runtime** and demonstrate it executing
correctly inside ArenaOS through the existing signed APB1 package + launch
path — no kernel-only test shortcut, no privileged serial fallback for the
positive demonstration.

In scope: validated native startup ABI (ARST v2); stack/TLS/argv/init/`main`;
shared allocator over existing native VM bindings; the documented C library
subset (`docs/compat/C2-LIBC-COVERAGE.md`); capability-backed standard
streams; C11-subset native threads/synchronization bindings; exit status,
termination, cleanup; baseline x87/SSE2 execution under ADR-0113.

Out of scope: full POSIX, complete C11, C++, dynamic linking, audio, GPU
acceleration, major application ports, genuine upstream SDL3 execution
(Phase 17, proposed), new syscalls or kernel object types (per proposed
ADR-0112; any exception needs M/C decision first).

## 2. Steps and exit evidence

| # | Step | Exit evidence |
|---|---|---|
| P16.0 | Baseline: record starting commit, environment, backend identities; smoke-boot the merged Phase 15 baseline | `BASELINE.md`, raw serial logs, screenshot 01 |
| P16.1 | Selective import of `experiments/c-runtime/`, `docs/compat/`, proposed ADR-0111/0112 at recorded source commits; provenance + integration-change log | `PROVENANCE.md`; import commit; bit-identical rebuild check vs Arena 2 receipt hashes |
| P16.2 | Reproduce Arena 2 host/ABI/build evidence against Phase 15 `main` sources (GCC + Clang) | host ASan/UBSan counts, ABI check, ISA audit, judge self-tests |
| P16.3 | Issue register: reproduce/classify the seven assigned issues; fix in-contract defects with regression tests; ADR proposals (only) for wire changes | `ISSUES.md`; named regression tests failing before / passing after |
| P16.4 | Phase 15 FP profile: SSE2/x87 C-thread preservation test app built with the ADR-0113 baseline profile; static ISA checks (SSE2 present, AVX absent, kernel integer-only audit untouched) | app + build script + audit output; guest run when kernel build unblocks |
| P16.5 | Guest integration: C2/C1 suites on the rebuilt kernel via signed APB1 bundles; capability-refusal negatives; strict Phase 15 suite; registered regression suite (121 at Phase 15 acceptance — recount at run time) | raw guest logs + exact-PASS receipts; screenshots 02/03 |
| P16.6 | Qualification: final-artifact-bound 100/100, fresh archive extraction + checksum + representative guest run; demo package; roadmap update; final report + Arena 1 (SDL3) handoff | `FINAL-REPORT.md`, `ARENA1-HANDOFF.md`, screenshot 04 |

P16.5/P16.6 are **blocked in this sandbox** until the two environment
blockers in `BASELINE.md` §4 are resolved (official `rust-std` components
for the protected Phase 14 fixture identity; glibc ≥ 2.38 for the pinned
patched-QEMU binary). No hash guard, signature check, or validator will be
weakened to route around them.

## 3. Rules held throughout

- One canonical runtime at `experiments/c-runtime/` until M approves a
  promotion path; no per-app allocators/startup/thread layers.
- Unsupported C functions keep refusing (`ENOSYS`/`thrd_error`/`EINVAL`);
  no fake successes. Resource limits are not raised to make tests pass.
- ADR-0111 (stream ring wrap) and ADR-0112 (C runtime profile) remain
  **Proposed**. The stream defect stays pinned by its regression test and
  documented as a limitation; it is not silently carried into an accepted
  stream implementation.
- Kernel stays integer-only (ADR-0012/0113); SSE2 is a ring-3 C profile.
- QEMU exit 0 is never sufficient: semantic guest assertions, clean
  shutdown, and resource/lifecycle receipts are required.
- Failures, interruptions, and reruns are recorded, not hidden.
