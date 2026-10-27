# Testing infrastructure workstream — plan (T)

Date: 2026-10-26. Owner: **T** (Testing Infrastructure Engineer).
This is an independent workstream after Phase 16. It is not Phase 17 and
does not change qualification standards, accepted architecture, or any
milestone gate. Core-OS defects found on the way are handed to A/C with
evidence, not silently fixed here.

## Branch and baseline

- Working branch: `arena/fd828627-arenaos` (platform-pinned for this
  session), verified identical to `main` at start.
- `main` at start: `50d579ea5d0e4612e979f9caed49acdad21551f7`
  ("Merge pull request #5"), the Phase 16 mainline merge named in the
  assignment. No commits exist on `main` after that merge; the working
  branch's merge-base with `main` is that same commit.
- Qualified Phase 16 OS/test source: `5aacbc88c50d4565a7bb9f4075be27e40df27779`
  (contained in the merge; OS/test source unchanged since).
- Reference documents read before starting: `docs/phase16/FINAL-REPORT.md`,
  `docs/phase16/ISSUES.md` (Q-1), the corrected receipt set
  `docs/phase16/receipts/canonical-corrected-5aacbc8-2026-10-26/`
  (including `registered/run.json` and `attempts/registered-attempt1-5aacbc8/`),
  `docs/phase16/BACKEND-RECOVERY.md`, `tools/run_tests.sh`,
  `tools/mtest.py`, `tools/arena_env.py`, `tools/build.sh`.

## Goals (measured, in order)

1. **Evidence first.** Every invocation/attempt gets a unique, immutable
   output directory; raw serials, QEMU stderr, commands, timings, source
   identity (commit + dirty diff), artifact hashes, and backend/firmware
   identities are preserved; retries link to and never hide first
   attempts; intentional negative controls stay distinguishable.
2. **Inventory + profile.** One machine-readable registry covering the
   121 registered suites (21 host/static blocks + 100 `test_m*.py`),
   the Phase 16 targeted guest/host suites outside that registry, the
   strict Phase 15 checks, stability, and package verification; per-suite
   timing with build vs guest split where the harness exposes it.
3. **Safe speed.** Remove repeated identical work (build traversal,
   packaging, audits) behind content-established identities; parallelize
   only what is provably isolated. No assertion is removed, no timeout
   loosened, no retry is hidden.
4. **Clear modes.** `quick` / `affected` / `full` / `qualify` via one
   entry point (`tools/arena_test.py`), each report stating what ran,
   what was omitted and why, and the evidence level. Quick/affected are
   never presented as qualification. `tools/run_tests.sh` remains the
   reference runner until verdict equivalence is demonstrated.
5. **Q-1.** Bounded reproduction + root-cause evidence for the attempt-1
   failures of `test_m84_crash.py` and `test_m9_client_death.py`
   (see `Q1-INVESTIGATION.md`). Q-1 stays **OPEN** until M/C accept a
   disposition; this workstream does not close it by rerun.

## Non-goals

- No kernel/service redesign, no new OS phase, no release work.
- No merged-to-main changes, no changed expected hashes, no weakened
  guards (Phase 14 protected identities, strict vector-19, GCC+Clang,
  100/100 stability semantics and fresh-archive verification all stay).
- The graphics agent's tree is not touched; shared-harness coordination
  goes through A (harness changes here are env-gated and inert by
  default, see `ARCHITECTURE.md`).

## Baseline findings so far (this sandbox; see BASELINE.md)

- Fresh sandbox: no Rust/QEMU preinstalled, glibc 2.36, 2 vCPU / 3.8 GiB.
  `tools/dev-env/bootstrap.sh` + the committed official Rust 1.97.0
  components reproduce the Phase 16 canonical EFI **byte-for-byte**
  (`7dcee442…2bfe6`) and both protected Phase 14 identities from a
  `/workspace/ArenaOS` bind mount.
- Cold `tools/build.sh --image`: **34.0 s**. "Warm" rebuild: **8.5 s** —
  of which ~6.8 s is a full `arena-kernel`+`arena-boot` recompile on
  every invocation, because `tools/build_phase14_pie.sh` unconditionally
  rewrites the byte-identical `userspace/phase14-pie/fixture.elf`
  (embedded via `include_bytes!`), poisoning Cargo's fingerprint.
- One registered run executes **≥143** `mtest.build()` traversals of
  `build.sh` (plus direct `build.sh` calls in restored-mutation suites)
  and lower-bound guest-boot time of ~1369 s (185 timed boots) inside
  the recorded 6574 s attempt-2 total.
- Stock (unpatched) QEMU 11.0.2 fails every suite: the in-guest m15
  suite requires genuine vector 19. Guest work here therefore uses the
  pinned patched QEMU 11.1.2 (`e0ea2b1f…f26ff`) under an isolated glibc
  2.40 built by `tools/dev-env/build-isolated-glibc.sh`, per
  `docs/phase16/BACKEND-RECOVERY.md`; identities are recorded per run.

## Deliverables map

| # | Item | Location |
|---|---|---|
| 1 | Plan + measured baseline | this file, `BASELINE.md` |
| 2 | Inventory + time breakdown | `tools/test_registry.json`, `INVENTORY.md` (generated) |
| 3 | Runner + optimizations | `tools/arena_test.py`, `tools/build_cache.py`, env-gated `tools/mtest.py` hooks, write-if-changed fixture writes |
| 4 | Q-1 status | `Q1-INVESTIGATION.md` |
| 5 | Beginner guide | `README.md` (this directory) |
| 6 | Agent guide (add a test, isolation hazards) | `AGENT-GUIDE.md` |
| 7 | Before/after timings | `BASELINE.md` + `receipts/T-2026-10-26/` |
| 8 | Validation + limitations | `FINAL-REPORT.md`, infra self-tests `tools/test_arena_test_infra.py` |
| 9 | Review-ready branch + final report | `FINAL-REPORT.md` (this branch; no merge by T) |
