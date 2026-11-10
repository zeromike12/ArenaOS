# V1 acceptance evidence index — candidate `7a187b0`

## Decision

**V1 is NOT ACCEPTED. The full gate is BLOCKED solely by Requirement 4b, which remains OPEN.** No M-recorded disposition or waiver is present, and no qualifying other-session surface fixture was recorded. The predicate-only, synthetic identity mutation, and real 301/300-grant evidence remain separate; none is substituted for the missing 4b proof. Do not interpret any PASS below as a V1 qualification PASS.

The full gate's process exit was **3**, the aggregate's blocked status—not a failure in the executed stage checks. M retains acceptance, waiver, merge, and release authority.

## Candidate and canonical environment

- Tested source commit: `7a187b06099013d280c004116971ff1c926bfb7b` on `arena/af044828-arenaos`.
- Canonical build root: `/workspace/ArenaOS`, verified as a bind mount of the checkout (`/dev/vda[/home/user/ArenaOS]`, ext4). Offline Cargo mode was used.
- V1 code fingerprint recorded by the gate: `9ab1ebaf85c06023cce66bf9c1ae01fb18491a27b6659cb43e843a54f04da2c8`.
- Canonical desktop EFI SHA-256: `b2fc4a295bfa5ef33c141f2fcfd099af5a761c25250592371996a02d6eba377a`.
- Canonical ESP SHA-256: `a2f9821206c8efb22f13a6b3bcb92d5c6bd30b4bd60cef9bf3673b6280950551`.
- The candidate was rebuilt after the formal regression and matched both pre-regression artifact hashes exactly.

The evidence worktree was not globally clean: `tools/run_tests.sh` printed `QUALIFICATION SOURCE CLEAN: no (development run)` because the preserved evidence/protected-PPM work makes `git status` nonempty. The V1 gate receipts record an unchanged code fingerprint and empty qualification `dirty_paths` (the docs/evidence paths are excluded from the source fingerprint). No V1 source, protected hash guard, launch restriction, or capability authority was changed for these runs. The four protected PPM edits and restored V2 evidence were not included in the V1 packages.

## Full gate r4 — completed stages

Command mode was `V1_GATE_MODE=full`, with `V1_STABILITY_BOOTS=100`. Every executed stage passed:

- Canonical build, environment, gate controls and aggregate controls.
- GCC and Clang guest proofs, refusal matrices, and real-grant layout integrations.
- Ten lifecycle cycles.
- Phase-15 guest validation on baseline and AVX-capable CPU profiles.
- Formal regression command, followed by exact candidate restoration.
- Final-image stability: **100/100 fully green boots**, zero failures, 1,069 seconds. Receipt: `b2fc4a295bfa5ef33c141f2fcfd099af5a761c25250592371996a02d6eba377a 100/100`.
- Strict structured evidence package and fresh-extraction verification: **208 files, zero problems**.

The final requirement table is in the external receipt and full-gate archive. It records Requirements 1–4, 5, 5b–5g, 6, and 7 PASS, and **4b OPEN**. Consequently the aggregate states `V1 QUALIFICATION: BLOCKED`.

### Formal regression count label

The actual `tools/run_tests.sh` transcript ended with **`ALL TESTS PASSED (123 test suites)`**, exit 0. The enclosing `v1_gate_regression_restore.sh` has an informational hard-coded line saying `121-suite command exit=0`, and the gate labels Requirement 5d “formal 121-suite regression.” The count-label discrepancy is preserved here rather than silently edited; the run completed with 123/123 reported suites passing, and the post-suite canonical EFI/ESP restoration passed.

### Published r4 artifacts

- Strict package: [`v1-acceptance-7a187b0-r4.tar.gz`](v1-acceptance-7a187b0-r4.tar.gz) — SHA-256 `5ec849d1951fd70031ff93707912b892fdf46e459fb94ab037727d1d94ec2ee4`.
- Publication receipt, stage receipts/logs, final aggregate and fresh-retrieval result: [`v1-acceptance-7a187b0-r4-packaging/`](v1-acceptance-7a187b0-r4-packaging/), especially `external-receipt.txt`.
- Supplemental stability raw evidence: [`v1-acceptance-7a187b0-r4-stability-raw.tar.gz`](v1-acceptance-7a187b0-r4-stability-raw.tar.gz) — all 100 QMP pixel/lifecycle receipts and DNS fixture logs, first/last genuine QEMU screenshots, final boot serial/fixture outputs, stability receipt and stage receipt. Its README accurately notes that the upstream loop retains only the final successful serial transcript, not separate serial logs for boots 1–99.
- Supplemental original PPM captures excluded from the strict PNG package: [`v1-acceptance-7a187b0-r4-raw-ppm.tar.gz`](v1-acceptance-7a187b0-r4-raw-ppm.tar.gz), 84 files.

## Phase 13 regression — 600-second timeout unchanged

Two successful runs are retained:

1. Focused run `7a187b0-20261110-r2`: **PASS**, main guest boot 415.699 seconds with `timeout_s: 600`; favorites reload 9.130 seconds with its 90-second limit. The command, artifacts, complete serial/QMP/wait traces, and raw attempts are in [`v1-phase13-7a187b0-r2.tar.gz`](v1-phase13-7a187b0-r2.tar.gz), SHA-256 `78c1f54135e799c1ddaa2c465130f3a0b1d5e2bab079db716eac44b7237d3ead`.
2. Formal-suite sub-run `20261110T161825Z-152540`: **PASS**, main guest boot 391.726 seconds with the same 600-second timeout; favorites reload 9.028 seconds. Its raw evidence is included inside the r4 strict package at `phase13-formal-regression-20261110T161825Z-152540/`.

Both runs record the candidate commit and exact EFI/ESP hashes. Neither changes the timeout, erases earlier failures, or closes 4b.

## Guest-only gate r3

The corrected bind-mount guest run is separately recorded as **PASS — PARTIAL**. GCC/Clang guest proofs, refusals, real-grant integrations, lifecycle cycles, packaging and verification passed. Requirement 4b remained OPEN; stability, formal regression and Phase 15 were not yet run in that guest-only gate. Its strict archive is [`v1-acceptance-7a187b0-r3.tar.gz`](v1-acceptance-7a187b0-r3.tar.gz), SHA-256 `471ab93f67a045bf790bdb07218d64fa396a67f4af49bcc5c04b6c4c273392d5`; 155 files were checked with zero problems. The separate raw PPM supplement is [`v1-acceptance-7a187b0-r3-raw-ppm.tar.gz`](v1-acceptance-7a187b0-r3-raw-ppm.tar.gz), 84 files.

## Historical failures retained

- The r2 symlink-path canonical build refusal remains preserved under `v1-canonical-build-7a187b0-r2-hash-mismatch/`: expected PIE fixture SHA-256 `d7b0a8cf9c735c3898a867d824563f06b0d949df80fa1a9c96f9180a395fea2e`; generated diagnostic ELF SHA-256 `a65f3a1e9bc29158b509e5c267cbbacf53ae6ba0a39b2b5972f1cad56e086c05`. It ran no guest tests and produced no EFI/ESP. The later bind-mount rebuild reproduced the expected fixture; that does not rewrite the historical refusal.
- The earlier `91748da` full gate remains incomplete, including the recorded regression exit 143. It is historical and is not erased by r4.
- Earlier 7a partial results remain partial; r3 and r4 are separately identified, later runs.

## Current remaining action

Requirement 4b needs its authorized real other-session fixture or M's explicit recorded disposition. Until then, qualification remains **BLOCKED / NOT ACCEPTED**, regardless of the PASS results for all other applicable gates.
