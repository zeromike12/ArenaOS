# Testing infrastructure workstream — final report

Date: 2026-10-26; review-correction addendum 2026-10-27 (sandbox run dates
recorded as 2026-10-27 UTC in run IDs)

Branch: `arena/fd828627-arenaos`

Authority requested: M/C review. This branch is not merged to `main`, does
not start a new OS phase, and does not replace the accepted qualification
runner by fiat.

## Correction addendum — current status

C reproduced three correctness defects in the reviewed `fc943da` version.
They are fixed in:

- `71e94fd33da9a96ba3b74bab02d8e7243c073e58` — schema-2 fail-closed build
  cache, monotonic output-independent process-group timeouts, and sealed,
  portable run verification;
- `ab38d4d1c59066c860495aef969fba414d7d96ba` — absent-root-manifest cache
  scan fix;
- `30fe81047ff4a5c44a09d959bb92b7558c7891e9` — feeder-error boot records,
  archive-preflight raw evidence routing, and the Phase 13
  detailed-receipt wait.

The corrected source `30fe810` completed one uninterrupted serial `full`
with **PASS 138/138** in 6,866.0 s
(`20261027T183025Z-full-30fe810-434497-768c5a`). Two preceding complete
full attempts failed 137/138 for different, preserved causes and are
reported in detail below; neither is hidden or replaced by batches.

The raw evidence is retrievable in
`docs/testing/evidence/T-correction-2026-10-27/` as a 36,874,615-byte
sealed archive, SHA-256
`9fccf05b686fe0683ca30fef6f010a998b537adc449f5d267f2403da94c5b6ea`.
Fresh extraction passed the external checksum, the 6,317-file manifest,
and `verify-run` on all six complete extracted runs. This addendum
supersedes the pre-review cache timing, segmented-only full caveat, and
evidence-retention limitation where they conflict below.

## Executive summary

The workstream delivers a single evidence-preserving test entry point,
`tools/arena_test.py`, with `quick`, `affected`, `suite`, `full`, and
`qualify` modes over a generated 150-entry registry.

The principal improvements are:

1. **Evidence can no longer be overwritten by the next suite or retry.**
   Every invocation and attempt has a unique directory with raw output,
   per-boot serial/QEMU evidence, source state, artifact hashes, timing,
   and backend identity.
2. **Fast development signals exist without weakening full coverage.**
   `quick` passed in 25.8 s and a docs-only affected run passed in 26.2 s on
   the constrained 2-CPU sandbox.
3. **Repeated identical builds are avoided fail-closed.** The corrected
   schema-2 fingerprint covers effective Cargo/profile/toolchain inputs or
   disables reuse with a recorded blocker. At tested source `30fe810`,
   full attempt 2 recorded 118 verified hits averaging 0.819 s and 48
   miss-stored real builds averaging 11.499 s; the later passing full was
   warm-cache (166 hits, 0.922 s average). The earlier 0.25 s figure came
   from the incomplete pre-review fingerprint and is not a current claim.
4. **The warm-build defect was fixed.** Write-if-changed Phase 14 fixture
   handling stopped unconditional kernel recompilation; steady warm builds
   fell from 8.47–8.49 s to 1.91–2.01 s with the canonical EFI unchanged.
5. **Isolated parallelism is implemented and proved at small scale.**
   A warm four-entry set took 20.7 s at `--jobs 2` versus 37.4 s serial.
   Serial remains the default and qualification posture.
6. **All current registry entries were exercised successfully in the rebuilt
   environment:** pre-review segmented coverage was 150/150, and the
   corrected runner subsequently completed the 138-entry `full` selection
   as one uninterrupted PASS. No new monolithic `qualify` claim is made.

## Deliverables

| Deliverable | Location |
|---|---|
| Workstream plan | `docs/testing/PLAN.md` |
| Architecture/safety model | `docs/testing/ARCHITECTURE.md` |
| Beginner guide | `docs/testing/README.md` |
| Agent guide | `docs/testing/AGENT-GUIDE.md` |
| Inventory | `tools/test_registry.json`, `docs/testing/INVENTORY.md` |
| Affected-selection map | `tools/test_map.json` |
| Runner | `tools/arena_test.py` |
| Build cache | `tools/build_cache.py` plus opt-in `tools/mtest.py` hook |
| Backend controls | `tools/backend_controls.py` |
| Fresh-archive verification | `tools/fresh_archive_verify.py` |
| Infrastructure negative controls | `tools/test_arena_test_infra.py` |
| Baseline/after timings | `docs/testing/BASELINE.md` |
| Q-1 investigation | `docs/testing/Q1-INVESTIGATION.md` |
| Compact receipts | `docs/testing/receipts/T-2026-10-26/` |
| Sealed correction evidence | `docs/testing/evidence/T-correction-2026-10-27/` |

## What changed in product/test code

No kernel, service, architecture, expected product hash, or qualification
threshold was changed.

Test/harness changes are limited to:

- env-gated evidence and cache hooks in `tools/mtest.py`;
- any-of readiness marker groups while preserving flat-tuple semantics;
- settlement pacing in `tools/test_m84_crash.py` and two same-shape boots in
  `tools/test_m84_stage.py` (assertions unchanged);
- write-if-changed guarded artifact writes in the Phase 14 fixture/bundle
  path;
- new runner/cache/registry/control tools listed above;
- review-correction evidence routing in the Phase 10/11 archive
  preflights, recording a boot before propagating an `mtest` feeder
  assertion, dynamic attempt-directory lookup, and a wait for Phase 13's
  detailed resource receipt (its assertions are unchanged).

`tools/run_tests.sh` is untouched. With the new environment variables unset,
the `mtest.py` evidence/cache hooks are inert.

## Validation actually performed

### Environment and artifact identity

After a sandbox reset, the environment was rebuilt from committed scripts
and official components. The runner recorded:

- Rust 1.97.0 with official hashed `libcore` components;
- patched QEMU 11.1.2
  `e0ea2b1f5ae2a5aa65a455eb232e46e1743b5c327c48ca678630c214c69f26ff`;
- explicit TCG;
- isolated glibc identities matching the accepted reconstruction;
- package OVMF identities;
- exact desktop EFI
  `7dcee4426edc4b2a204592a5680c27c5ee91d51e3231705641df2ca0d7e2bfe6`;
- exact Phase 14 PIE/APB1 identities.

### Review findings reproduced and corrected

1. **Build cache omitted effective inputs.** C showed that
   `CARGO_PROFILE_RELEASE_OPT_LEVEL=0` and a root `.cargo/config.toml`
   with rustflags did not change the old key. Schema 2 now hashes content
   through symlinks; semantic repository/ancestor/`CARGO_HOME` Cargo
   config; effective profile/compiler/linker/target/feature/flag inputs;
   source-declared environment sensitivities; selected hashes for
   `rustc`, Cargo, GCC, Zig, Python and `pyfatfs`; manifests/locks/build
   scripts and reachable external path inputs. Unidentified wrappers,
   linkers, targets, malformed configuration, or unsupported external
   references disable reuse with an explicit blocker. It does not hash
   unrelated environment or secrets. On `30fe810`, a fresh representative
   phase9 build missed/stored in 20.111 s, hit in 0.982 s, and the
   `OPT_LEVEL=0` fingerprint lookup was `None`.
2. **`verify-run` accepted an empty fabricated pass.** Runs now persist a
   selection/registry snapshot, per-attempt evidence indexes, run/source
   manifests, expected attempt counts, and suite-specific evidence
   contracts. Verification checks identities, selected attempts and
   terminal verdict consistency, preserved failures, required raw
   build/boot/custom files and hashes, completion state, interrupted
   successful claims, and resolves all evidence beneath the extracted
   run. Host-only entries do not need boots; intentional crash boots are
   labelled rather than treated as surprise semantic passes.
3. **Timeout depended on output.** The runner now uses a nonblocking
   selector with a monotonic deadline and process-group termination.
   Controls cover silence, print-then-stall, a partial line, continuous
   output, and descendants holding the pipe/group. C's 2-second child
   under a 1-second deadline now returns FAIL, `timed_out=True`, exit
   `-15`, in 1.01 s with its output preserved.

### Infrastructure controls

`tools/test_arena_test_infra.py` passes all controls, including:

- fingerprint invalidation on relevant input/environment changes;
- canonical entry cannot satisfy a mutated-input fingerprint;
- tampered artifact and incomplete cache entry rejected;
- 50 unique run directories;
- monotonic boot evidence allocation across re-imported harness processes;
- semantic guest failure rejected despite QEMU exit 0;
- killed boot never becomes success;
- fake-QEMU accelerator failure cannot pass and its real exit is preserved;
- failed first attempt remains visible beside a passing repeat;
- tampered evidence fails `verify-run`;
- empty fabricated full PASS, missing selected entry/attempt/output,
  contradictory verdicts, interrupted success, and evidence-copy failures
  fail verification;
- extracted runs verify portably from another directory;
- feeder/workflow assertions record their boot before raising;
- unknown affected path falls back to the registered set;
- changed helper suite selects importers;
- registry coverage/count checks.

### Quick/affected/suite modes

- Quick: 10/10 PASS, 25.8 s pre-review; corrected source `30fe810`
  quick `20261027T202533Z-quick-30fe810-605011-2eb4dc` passed 10/10 in
  31.1 s and verified after fresh extraction.
- Docs-only affected: correct quick selection, 10/10 PASS, 26.2 s.
- Branch-vs-main affected explanation conservatively selected all 121
  registered entries plus backend, package, and infra entries because the
  branch changes shared harness/build paths.
- Suite repeat mode was used for Q-1 and preserves every attempt.

### Q-1-focused validation

- `m84-stage`: PASS.
- `m84-crash`: PASS; all 16 recovery boots queried only after settlement.
- `m9-client-death`: 20/20 PASS across idle and CPU-loaded repetitions in
  the rebuilt environment.
- The affected Q-1 suites also passed inside their later full-selection
  batches.

### Full-selection validation

The 138-entry `full` selection passed serially in seven `suite` batches:

| Batch | Entries | Run ID |
|---|---:|---|
| 1 | 20 | `20261027T041821Z-suite-091d7cd-122174-2d62be` |
| 2 | 20 | `20261027T043451Z-suite-590e0f7-127164-efa71f` |
| 3 | 20 | `20261027T050811Z-suite-a483914-174452-746cf5` |
| 4 | 20 | `20261027T052740Z-suite-1a9de1f-206122-86feee` |
| 5 | 20 | `20261027T054950Z-suite-bdc0857-241115-1eda3e` |
| 6 | 20 | `20261027T061240Z-suite-a987c42-283627-7920fd` |
| 7 | 18 | `20261027T062012Z-suite-4dce334-301416-aa98fc` |

Totals:

- 138/138 PASS;
- registered subset 121/121 PASS;
- 7,589.7 s summed attempt time;
- 166 recorded `mtest.build()` calls (118 cache hits, 48 real builds);
- 495 recorded guest boots.

The batch commits between these runs added only compact result receipts under
`docs/testing/receipts/`; they did not change product/test execution code.
The receipts are the seven `full-selection-chunk-*.tsv` files.

Important qualification: this pre-review validation was complete
full-selection coverage, but not one uninterrupted `arena_test.py full`
invocation in the rebuilt sandbox. An earlier single full invocation in
the pre-reset sandbox reached 86/138 with no failures before the
sandbox/process was lost; it is not counted as a completed pass here. The
correction pass then performed the monolithic attempts below.

### Corrected uninterrupted full attempts

| Attempt | Source | Result | Duration | Disposition |
|---|---|---:|---:|---|
| `20261027T141830Z-full-ab38d4d-108026-dc6887` | `ab38d4d` | FAIL 137/138 | 6,997.4 s | Phase 13 raced its detailed receipt serial line. Verification also exposed the missing feeder-error boot record and unrouted archive-preflight witness evidence. All three concrete causes were fixed in `30fe810`; focused Phase 13 and both preflights then passed and verified. |
| `20261027T162612Z-full-30fe810-259917-b23647` | `30fe810` | FAIL 137/138 | 7,117.0 s | `m9-resources` saw transient live-process count 17 rather than asserted snapshot 15, while settled 16→15 counts and exact shared/pages/maps retirement held. The failed run was evidence-complete under `verify-run`. No assertion or product deadline was changed; ten preserved repeats passed with count 15. |
| `20261027T183025Z-full-30fe810-434497-768c5a` | `30fe810` | **PASS 138/138** | 6,866.0 s | One uninterrupted serial invocation, clean source at start/end, 507 boots (84 intentional kills), 5,866.0 boot-seconds, 166 build records. It was warm-cache: all 166 builds were verified hits because attempt 2/focused runs at the same source had stored the 49 fingerprints. |

Attempt 3 and the other complete focused runs were verified again after
fresh archive extraction. The archive includes attempt 3 whole, plus the
run-level records and complete failing attempts from attempts 1–2. It
does not duplicate every passing attempt tree from the superseded failed
fulls; those directories remain in the originating workspace.

### Qualification-shaped entries

The six `qualify`-only entries also passed individually in the rebuilt
environment:

- canonical final build;
- strict Phase 15 on final artifacts (both CPU profiles);
- strict Phase 15 on preserved package artifacts;
- stability: 100/100 green in 1,034 s;
- fresh-extraction Phase 16 archive static + guest verification;
- independent #XM controls: patched PASS, unpatched expected FAIL.

The six standalone registry entries passed as well. Therefore the compact
receipt `all-registry-results.tsv` covers 150/150 current registry entries.

This report does **not** call that collection a phase qualification. The
entries were run in focused/suite batches, and project acceptance of the new
runner remains an M/C decision.

### Parallel validation

The worktree+namespace pool passed a set containing a DNS-fixture RED/GREEN
guest, a desktop/compositor guest, a Cargo host suite, and the guest smoke.
It caught and fixed three real infrastructure issues during development:

1. worker entry-script working-directory clobbering;
2. ambiguous `getcwd()` after nested bind mounts;
3. absolute-path-sensitive Rust artifact identity.

Warm timing for that set was 20.7 s at `--jobs 2`, versus 37.4 s serial.

## Timing conclusions

Directly measured:

- warm build: 8.47–8.49 s → 1.91–2.01 s;
- corrected cache hit build call: 0.819 s mean at `30fe810` in full
  attempt 2 (0.922 s in warm attempt 3), versus 11.499 s mean for real
  miss-stored builds;
- canonical direct build on the correction host: 36.447 s cold,
  1.784 s warm, canonical EFI unchanged;
- quick: 25.8 s pre-review and 31.1 s on corrected source;
- warm 4-entry parallel set: 20.7 s versus 37.4 s serial.

Not claimed:

- a normalized full-run speedup over Phase 16 attempt 2.

The current registered subset's pre-review segmented attempt sum
(7,121.7 s) is slower than the accepted Phase 16 attempt 2 wall time
(6,574 s), despite component build savings. The corrected warm-cache
138-entry full took 6,866.0 s, but it has different selection/cache state
and host throughput is not normalized (the rebuilt sandbox exposes
2 CPUs), so neither is a like-for-like Phase 16 speed ratio. The passing
full remains dominated by 5,866.0 s of recorded guest boot time. See
`BASELINE.md` for the breakdown and the reasoning.

## Q-1 disposition

### `m84-crash`

The preserved attempt-1 serial proves the host typed `pkg query` before
packaged-service settlement and shut down after the answer but before
`packaged READY`. The guest's eligibility answer was the expected one; the
test's unchanged conjunction failed because shutdown outraced the READY
line.

Corrective action: pace pkg recovery boots on
`packaged READY` **or** the explicit `boot namespace scan refused`
settlement. Assertions unchanged.

**Request:** close this leg as a harness pacing race.

### `m9-client-death`

The original failing serial was overwritten by the next suite. A 2-second
kernel retirement deadline makes a liveness/timing hypothesis plausible, but
the exact fatal reason is not preserved. Twenty evidence-preserving repeats
in the rebuilt environment passed, including under CPU load.

**Request:** keep this leg open under A/C. Do not label it harmless or widen
the deadline based on rerun passes. The new evidence machinery is the
tripwire if it recurs.

## Known limitations

1. A single uninterrupted corrected `full` now passed, but no new
   monolithic corrected `qualify` invocation was run. Product code and the
   canonical image were unchanged, so another 100-boot stability run was
   not required merely for runner/docs corrections; `qualify` acceptance
   remains an explicit M/C decision.
2. The old runner was not rerun end-to-end in the rebuilt sandbox. Equivalence
   evidence is registry derivation from `run_tests.sh`, per-entry commands,
   121/121 registered outcomes, and unchanged default harness behaviour—not
   a fresh same-day old-vs-new full wall-clock race.
3. Parallel mode is proved only at small scale. It needs a full
   serial-vs-parallel comparison and resource policy before any
   qualification use.
4. ESP image bytes can vary between ordinary rebuilds even when the EFI is
   byte-identical; this workstream did not change ESP generation. Artifact
   hashes are recorded per build so the consumed bytes are always explicit.
5. Affected mapping is intentionally conservative and incomplete by design:
   unknown paths select the registered set. It should be refined only with
   evidence that a narrower selection is semantically complete.
6. The sealed repository archive contains the passing full whole, but the
   two failed fulls are represented by run-level records plus their
   failing attempts rather than every passing tree. M/C should decide
   whether whole superseded failed runs must also be published; the raw
   directories are retained in the originating workspace.
7. `m9-resources` failed once because an unsynchronized compositor
   diagnostic reported 17 live processes rather than the test's exact
   snapshot 15, even though settled counts and retirement accounting were
   exact. Ten repeats and the next full passed without changing the
   assertion. This is a separate intermittent snapshot/scheduling issue
   for A/C to disposition if it recurs; it is not evidence that
   `m9-client-death` is fixed or harmless.

## Recommended acceptance path

1. Accept the corrected fail-closed evidence model, registry, schema-2
   cache, output-independent timeout enforcement, quick/affected/suite
   modes, and the m84 settlement pacing for development use.
2. Keep `tools/run_tests.sh` authoritative for the next formal qualification
   unless M/C explicitly bless `arena_test.py qualify` after an uninterrupted
   comparison run.
3. Close Q-1's m84 leg; keep the m9 leg open with the evidence requirements
   in `Q1-INVESTIGATION.md`.
4. Before parallel qualification is considered, run a complete
   serial-vs-parallel comparison on a host with documented CPU/memory and
   investigate any timing-sensitive discrepancy as a potential product or
   harness issue, not as noise.
5. Decide whether the sealed-archive representation of superseded failed
   fulls is sufficient or whole failed-run publication is required, and
   disposition the separate `m9-resources` transient-count observation.
   Do not merge this PR as part of Agent T's correction task.
