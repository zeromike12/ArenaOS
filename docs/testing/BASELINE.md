# Measured baseline and post-change timings

This file separates three kinds of numbers that are often accidentally
conflated:

1. accepted Phase 16 receipt timings;
2. direct component measurements on the testing-workstream sandbox;
3. end-to-end timings from the new runner after the sandbox was rebuilt.

They are not all normalized to identical host performance. Where a comparison
is not like-for-like, this document says so rather than manufacturing a
speedup percentage.

## Reference environment for the rebuilt-sandbox measurements

- Repository path: `/workspace/ArenaOS` (bind mount of `/home/user/ArenaOS`)
- Host visible to the runner: Linux 6.1.158+, x86-64, 2 CPUs, about 4 GiB RAM
- Rust: 1.97.0 (`2d8144b78`), official hashed `libcore` components for
  `x86_64-unknown-none` and `x86_64-unknown-uefi`
- Emulator: pinned patched QEMU 11.1.2,
  `e0ea2b1f5ae2a5aa65a455eb232e46e1743b5c327c48ca678630c214c69f26ff`
- Accelerator: explicit TCG
- Isolated glibc: 2.40, libc
  `eef95f26903e6f9ac6b8cb92df9ba5306ff01b4058ddf7f9ad6aaa986bb7f35f`,
  loader
  `63704534fba7799d309e9e16992ea7d11e709906bd44b72060c007461b970458`
- Firmware: OVMF CODE
  `7e34e3b83224968bb0714c0ebc88f75c5c37b4cc892c902e511ccc7a7c107f94`,
  VARS template
  `5d2ac383371b408398accee7ec27c8c09ea5b74a0de0ceea6513388b15be5d1e`
- Desktop EFI reproduced exactly:
  `7dcee4426edc4b2a204592a5680c27c5ee91d51e3231705641df2ca0d7e2bfe6`
- Protected Phase 14 identities reproduced exactly:
  PIE `d7b0a8cf9c735c3898a867d824563f06b0d949df80fa1a9c96f9180a395fea2e`;
  APB1 `04add54f09042353fc42511978c82f8b8efb87f7d5f0657f9116f456276b3dfa`

Each current run records this identity in its own `run.json`; the summary
receipts are under `receipts/T-2026-10-26/`, and the sealed correction
evidence is under `evidence/T-correction-2026-10-27/`.

## Correction-pass measurements (current cache identity)

C's review invalidated the pre-review fingerprint as a safety claim, so
its 0.25 s hit mean below is historical only. On corrected source
`30fe810`, with the same 2-CPU pinned-TCG host:

| Measurement | Result |
|---|---:|
| Canonical fingerprint computation | 0.636 s over 1,419 tree files |
| Direct canonical build, cold | 36.447 s |
| Direct canonical build, warm | 1.784 s |
| Representative phase9 isolated-cache miss/store | 20.111 s outer |
| Representative phase9 hit | 0.982 s outer |
| Corrected quick | 31.1 s, 10/10 |

Uninterrupted corrected full attempts:

| Attempt | Result | Wall | Build records |
|---|---|---:|---|
| `20261027T141830Z-full-ab38d4d-108026-dc6887` | FAIL 137/138, Phase 13 pacing | 6,997.4 s | 48 miss-stored (558.4 s, 11.633 s mean); 118 hits (100.5 s, 0.852 s mean) |
| `20261027T162612Z-full-30fe810-259917-b23647` | FAIL 137/138, transient M9-resources count | 7,117.0 s | 48 miss-stored (552.0 s, 11.499 s mean); 118 hits (96.7 s, 0.819 s mean) |
| `20261027T183025Z-full-30fe810-434497-768c5a` | **PASS 138/138** | 6,866.0 s | Warm cache: 166 hits (153.1 s, 0.922 s mean), no misses |

The passing attempt recorded 507 boots and 5,866.0 s of boot time. These
are host- and cache-state-specific measurements, not a normalized speed
ratio against Phase 16 or another machine.

## Accepted Phase 16 baseline (old runner)

From
`docs/phase16/receipts/canonical-corrected-5aacbc8-2026-10-26/registered/run.json`:

| Registered attempt | Duration | Outcome |
|---|---:|---|
| Attempt 1 | 6,543 s | 119/121, with Q-1 failures |
| Attempt 2 | 6,574 s | 121/121 |

The old runner did not emit a complete per-entry duration table. The new
registry and receipts close that observability gap; see
`receipts/T-2026-10-26/timing-breakdown.tsv`.

## Root-cause measurements

### Warm build was not actually warm

Before the workstream change, `tools/build_phase14_pie.sh` rewrote the
byte-identical `userspace/phase14-pie/fixture.elf` on every traversal. That
file is embedded into the kernel build, so Cargo repeatedly recompiled
`arena-kernel` and relinked `arena-boot`.

| Measurement | Before | After write-if-changed |
|---|---:|---:|
| Warm `tools/build.sh --image` | 8.47–8.49 s | 1.91–2.01 s in rebuilt sandbox |
| `arena-kernel` compilation | every warm call | absent on steady warm calls |
| Desktop EFI | canonical | unchanged, canonical |

The fix does not bypass the Phase 14 hash guards. A deliberately corrupted
fixture was restored byte-for-byte by the guarded build path.

### Repeated identical builds dominated build calls (pre-review identity)

In the seven pre-review batches covering the 138-entry `full` selection,
the harness recorded 166 `mtest.build()` calls. The counts explain the
optimization opportunity, but the hit timing is not reused as a safety or
performance claim for the corrected fingerprint:

| Cache result | Calls | Total build-call time | Mean |
|---|---:|---:|---:|
| Miss, real build, stored | 48 | 578.6 s | 12.05 s |
| Hit, verified restore | 118 | 29.5 s | 0.25 s |
| Total | 166 | 608.1 s | 3.66 s |

There were 49 distinct fingerprints. Two canonical profiles accounted for
115 calls. Of the hits, 85 followed the same fingerprint immediately and 33
followed a fingerprint switch (the historic profile-toggle/relink case).

The pre-review fingerprint measurement was 0.186 s over 1,411 relevant
files in the pre-reset sandbox. Schema 2 deliberately spends more (0.636 s
in the current canonical report) to identify effective configuration and
toolchain inputs. A corrected hit is therefore not "free": roughly
0.82–0.92 s in the full attempts instead of an 11.5 s average real build.

### Guest execution is the main end-to-end cost

For the 138 full-selection entries, preserved boot/build records sum to:

| Component | Time |
|---|---:|
| All attempt wall time | 7,589.7 s |
| `mtest.build()` calls (hit + miss) | 608.1 s |
| Recorded guest boots (495) | 5,779.4 s |
| Other host work/waits/suite logic | about 1,202.2 s |

This is why build caching alone cannot turn the complete regression into a
few-minute run. The large wins are:

- not rebuilding identical images;
- not running the full set for a narrow, correctly mapped change;
- overlapping only provably isolated work.

## Current end-to-end results

### Quick and affected

| Command shape | Result | Wall time |
|---|---|---:|
| `arena_test.py quick` | 10/10 pass | 25.8 s pre-review; 31.1 s corrected |
| Docs-only `arena_test.py affected --base HEAD^` | 10/10 pass | 26.2 s pre-review |

The smoke entry took 20.8 s in the measured quick run; the host entries and
infra controls together were only a few seconds. Quick remains under a
practical one-minute development budget on this constrained sandbox.

### Full selection, segmented serially

The 138-entry `full` selection passed in seven serial `suite` batches after
the sandbox rebuild:

| Batch | Entries | Attempt time |
|---|---:|---:|
| 1 | 20 | 925.8 s |
| 2 | 20 | 1,989.0 s |
| 3 | 20 | 1,157.3 s |
| 4 | 20 | 1,317.5 s |
| 5 | 20 | 1,358.9 s |
| 6 | 20 | 440.5 s |
| 7 | 18 | 532.9 s |
| Total | 138/138 | 7,589.7 s attempt time |

The registered 121 subset sums to 7,121.7 s in those batches. That is **not**
claimed as a like-for-like improvement over Phase 16 attempt 2's 6,574 s:
the rebuilt sandbox exposes only 2 CPUs, host throughput is not normalized,
and the new harness copies substantially more evidence. The component-level
build savings above are real; whole-run wall time on this small host remains
guest/TCG-bound.

### Qualification-only entries

All six entries in the 144-entry `qualify` selection that are outside `full`
also passed:

| Entry | Duration |
|---|---:|
| canonical-final-build | 10.7 s |
| strict-m15-final | 15.6 s |
| strict-m15-preserved | 16.3 s |
| stability-100 | 1,034.6 s (100/100) |
| fresh-archive-phase16 | 18.2 s |
| backend-xm-controls | 4.7 s |

The six standalone entries outside both modes also passed, so summaries in
`receipts/T-2026-10-26/all-registry-results.tsv` cover 150/150 registry
entries.

## Parallel measurement

For a representative four-entry set
(`m9-host-dns-red`, `m9-compositor-input`, `host-heap`, `smoke-m1`):

| Mode | Result |
|---|---:|
| `--jobs 2`, cold worker caches | 75.6 s |
| `--jobs 2`, warm shared cache | 20.7 s |
| Serial | 37.4 s |

Cold workers duplicate one real build per fingerprint. Warm parallel
execution was 1.8× faster for this small set. This is a proof of isolated
execution, not a promise that a four-worker full run is 1.8× faster or
qualification-safe on a 2-CPU/4-GiB host.

## Slowest entries in the current full selection

| Entry | Attempt | Main recorded cost |
|---|---:|---|
| `m10-boundaries-red` | 693.4 s | 200.7 s builds, 100.7 s boots, substantial host-side mutation/oracle work |
| `phase13-registry-guest` | 505.0 s | 478.6 s guest boots |
| `m82-policy-crash` | 368.4 s | 366.1 s across 45 boots |
| `m10-multi-red` | 327.1 s | 303.6 s guest boots |
| `m84-crash` | 296.3 s | 294.1 s across 34 boots |
| `host-afs2` | 232.3 s | single-threaded host crash-prefix model |

The per-entry build/boot/other split is in
`receipts/T-2026-10-26/timing-breakdown.tsv`.

## Interpretation

- The old 6,574-second registered run and new segmented run cannot be used
  as a normalized before/after wall-clock ratio.
- Warm-build and corrected build-cache gains are directly measured at the
  component where they occur; correctness costs about 0.6–0.9 s per hit
  on this host and is not hidden.
- The passing corrected full was warm-cache because the immediately
  preceding same-source full/focused runs had populated all 49
  fingerprints. Attempt 2 provides the same-source miss/hit split.
- The dominant remaining costs are real guest boots, deliberate mutation
  builds, interactive desktop waits, and the AFS2 host model.
- Quick/affected modes provide the practical developer-time improvement;
  full remains intentionally expensive because it preserves coverage.
