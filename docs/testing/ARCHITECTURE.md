# Testing infrastructure architecture

This workstream adds an evidence-preserving runner beside, not instead of,
`tools/run_tests.sh`. Its design principle is:

> Optimize by avoiding proved-identical work and by selecting proved-relevant
> work. Never optimize by reducing what a passing result means.

## Components

| Component | Responsibility |
|---|---|
| `tools/test_registry.json` | Generated inventory of 150 entries: command, kind, qualification role, source files, quick membership, mutation flag, parallel class, timeout |
| `tools/gen_test_registry.py` | Derives registered entries from `run_tests.sh` and adds standalone/targeted/strict/stability/package/infra entries; holds manual overrides where static analysis is insufficient |
| `tools/test_map.json` | Conservative source-path-to-suite mapping for `affected` |
| `tools/arena_test.py` | CLI, selection, execution, environment identity, reports, evidence verification, optional worker pool |
| `tools/mtest.py` hooks | Optional per-build/per-boot evidence copies and cache consultation; inert unless environment variables opt in |
| `tools/build_cache.py` | Content-addressed whole-image build reuse with atomic publication and hash-verified restore |
| `tools/backend_controls.py` | One-command patched/unpatched independent #XM controls |
| `tools/fresh_archive_verify.py` | Fresh extraction plus packaged static and guest verification |
| `tools/test_arena_test_infra.py` | Negative controls for the infrastructure itself |

## Registry model

The registry has 150 entries:

- 121 `registered` entries equivalent to the old runner's registered set;
- 6 Phase 16 host entries;
- 10 Phase 16 targeted guest entries (GCC and Clang variants);
- 4 strict Phase 15-related entries;
- 1 stability entry;
- 1 package/fresh-archive entry;
- 1 infrastructure entry;
- 6 standalone entries.

`check-registry` verifies that suite files are covered and that the
registered count remains coherent. It is a fail-closed maintenance gate, not
just a report.

Kinds distinguish host Rust, host Python, host C, guest, guest-desktop,
build, stability, and package evidence. Qualification role and evidence kind
are deliberately separate: a real guest boot can still be development or
standalone evidence rather than phase qualification.

## Selection modes

### `quick`

A measured smoke set: fast host gates, `smoke-m1`, and infra controls. The
complete AFS2 host model is excluded by measured duration, not by guessed
importance.

### `affected`

Changed paths come from the requested Git base plus current worktree state.
Rules can select entries, qualification groups, milestone prefixes, the
changed suite itself, and known importers. The quick set is always included.
An unmapped path falls back to the registered set. Empty selection is never
treated as a valid optimization.

### `full`

All registered entries plus Phase 16 host/targeted entries and the infra
controls: 138 entries.

### `qualify`

The full set plus canonical final build, strict final and preserved Phase 15,
100-boot stability, fresh-archive package verification, and backend controls:
144 entries. It checks preconditions and returns BLOCKED rather than a weak
pass when they are unmet.

## Evidence model

Each execution creates:

```text
build/test-runs/<run-id>/
  run.json                    source/environment identity is embedded here
  selection.json              immutable selected-entry/evidence contract
  evidence-manifest.json      run-level file hashes (itself excluded)
  source.diff / source-status.txt
  attempts/<entry>/attempt-<nnn>/
    cmd.json
    output.log
    result.json
    evidence.json             attempt file hashes (itself excluded)
    builds/<nnn>-<label>/build.json + copied logs when available
    boots/<nnn>-<label>/boot.json + serial/QEMU/fixture evidence
    outputs/...
```

Important invariants:

1. Run IDs combine UTC time, mode, source commit/dirty state, PID, and random
   suffix; directory creation is exclusive.
2. Attempts are append-only. `--repeat` does not overwrite attempt 1.
3. Per-boot/per-build directories are allocated monotonically under a lock,
   including when a suite subprocess re-imports `mtest`.
4. QEMU process exit and semantic guest verdict are recorded separately.
5. Intentional kills/negative controls are metadata, not silently converted
   to ordinary passes.
6. A run fails if tracked source changes unexpectedly during execution.
   `source-status.txt` and the source object bind status/diff SHA-256 and
   byte counts, so an extracted verifier does not trust prose alone.
7. `verify-run` checks meaningful source/environment identity, the frozen
   selected entries and repeat count, every required attempt, terminal
   verdict agreement, preserved failures, completion state, and cited
   evidence hashes. It resolves paths inside the supplied run directory,
   including after extraction elsewhere.
8. Evidence expectations are entry-specific: host/static entries need
   command output; ordinary guest entries need build/boot records with
   raw serial and QEMU stderr; crash fixtures are identified by
   intentional-kill metadata; backend and extracted-archive witnesses
   declare their custom raw files. Copy failures mark evidence incomplete
   and cannot support an evidence-complete PASS.
9. A feeder/workflow assertion records the boot it was driving, including
   a distinct workflow verdict/error, before `mtest.boot()` re-raises.

The old shared `build/` outputs are still written for compatibility. They
are working files, not the durable evidence of record.

## Build-cache model

The schema-2 cache key is a fingerprint of effective build inputs, not a
commit ID. It includes:

- relevant repository/vendor tree contents, following symlinks to their
  target bytes rather than keying only the link name;
- semantic Cargo configuration from the repository, ancestors, and
  `CARGO_HOME`, so comments do not churn identity but effective rustflags,
  target, linker, wrapper, or source-replacement changes do;
- effective Cargo profile/target/feature/flag environment, selected
  source-declared compile-time variables, and build-script
  `rerun-if-env-changed` names (values are represented by hashes; the
  whole environment and unrelated secrets are not fingerprinted or put in
  receipts);
- manifests, lockfiles, linker scripts, build scripts, declared rerun
  paths, path dependencies/patches/workspace members, and other reachable
  external file inputs;
- hashes/identities for the selected Rust compiler and target libraries,
  Cargo, GCC, Zig, Python executable, and Python modules that actually
  participate (`pyfatfs`);
- absolute repo path and graphics/profile inputs.

If configuration is malformed, a custom target/wrapper/linker or external
input cannot be identified conservatively, or a remote replacement cannot
be reduced to local bytes, `fingerprint_status()` returns no reusable key
and records explicit blockers. The real build still runs; it is not
allowed to publish or consume an uncertain entry.

The absolute path is intentionally part of identity. Rust outputs embed it;
the Phase 14 PIE guard proved this by rejecting a different-path build with a
different hash.

Publication is atomic and requires a completion marker. Lookup revalidates
schema, fingerprint/config agreement, enabled reuse, complete metadata,
artifact sizes and hashes. Tampered, disabled-identity, missing, or
incomplete entries are not served.

Mutation suites are safe with respect to canonical reuse because mutated
source bytes yield different fingerprints. The infra controls test this
explicitly. `--no-build-cache` returns to historical build behaviour.

## Attempt timeout model

Each suite attempt runs in its own session/process group. The runner
drains its merged output with a nonblocking selector against one
`time.monotonic()` deadline, so silence, one partial line, a stall after
output, and continuous output all share the same expiry rule. At the
deadline it terminates and then kills/reaps the process group as needed,
records `timed_out=true`, and preserves output already received. A child
that would later exit 0, or a descendant holding the output pipe, cannot
erase or extend the deadline.

## Readiness model

`mtest.boot()` marker gates preserve the historical meaning of flat tuples:
all flat members are required. A nested tuple is an any-of subgroup. This is
for settlement points where either a success line or an explicit refusal
line proves the peer has finished deciding.

The gate never decides the suite verdict. The suite still asserts which of
the legal settlement outcomes applies to its disk/process state.

## Parallel model

Parallelism is opt-in (`--jobs N`, default 1).

For jobs above 1, pool entries run in:

- a dedicated Git worktree under `/tmp/arena-test-workers/<run-id>/wN`;
- a user, mount, and network namespace;
- a bind mount of that worktree at canonical `/workspace/ArenaOS`;
- the entry's own working directory addressed by canonical spelling;
- a shared content-addressed build cache at `/tmp/arena-test-cache` unless
  explicitly overridden.

The separate network namespace prevents collisions on fixed host fixture
ports. The separate worktree prevents build/output and source-mutation
collisions. Canonical path spelling prevents avoidable Rust artifact
identity changes.

After a worker attempt, evidence is copied into the main run directory and
the worktree is reset before reuse. Registry entries marked `serial` run
after the pool. Untracked, non-ignored files block parallel mode because a
Git snapshot worktree would not contain them.

## Environment identity

Guest evidence is backend-sensitive, so the runner records:

- QEMU invoked file, wrapper text/hash, resolved emulator binary/hash;
- emulator version and accelerator classification;
- firmware paths and hashes;
- isolated glibc libc/loader hashes when supplied;
- Rust/Cargo, GCC, Zig, Python versions;
- official sysroot `libcore` hashes;
- host uname, CPU count, memory snapshot;
- canonical-path status.

This prevents a rebuilt emulator or changed sandbox capacity from silently
inheriting an earlier backend identity.

## Failure semantics

- Suite process nonzero: FAIL.
- Timeout: FAIL.
- QEMU exit 0 with fatal/missing semantic markers: FAIL.
- Killed boot: never success by itself.
- Failed attempt followed by passing repeat: run FAIL, both preserved.
- Missing cited evidence: verification FAIL.
- Fabricated/contradictory verdicts, missing selected attempts, or an
  interrupted run claiming success: verification FAIL.
- Unidentified build configuration: cache reuse disabled with a reason;
  the real build still runs.
- Qualification preconditions missing: BLOCKED, exit code 2.
- Unknown affected mapping: conservative registered fallback.

## Intentional non-goals

- No golden-image guest reuse: many suites test state creation and mutation,
  so reusing a booted guest would change the claim.
- No compiler cache adoption in this workstream: whole-image fingerprinting
  covers repeated identical builds with clearer artifact identity.
- No parallel-by-default qualification: CPU contention can change TCG timing
  margins and needs separate project acceptance.
- No old-runner retirement: `run_tests.sh` remains until M/C accept an
  equivalence and replacement decision.
