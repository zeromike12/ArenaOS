# Agent guide: ArenaOS testing infrastructure

This guide is the operational contract for agents using or extending
`tools/arena_test.py`. It favours evidence over speed whenever the two appear
to conflict.

## Entry points

Run from the canonical repository root (`/workspace/ArenaOS` in the sandbox):

| Purpose | Command |
|---|---|
| Fast smoke | `python3 tools/arena_test.py quick` |
| Inspect affected selection | `python3 tools/arena_test.py affected --base main --explain` |
| Run affected selection | `python3 tools/arena_test.py affected --base main` |
| Named entries | `python3 tools/arena_test.py suite <id> [<id> ...]` |
| Repeated entries | `python3 tools/arena_test.py suite <id> --repeat N` |
| Complete development set | `python3 tools/arena_test.py full` |
| Qualification-shaped set | `python3 tools/arena_test.py qualify` |
| Registry validation | `python3 tools/arena_test.py check-registry` |
| Environment identity | `python3 tools/arena_test.py env-info` |
| Evidence verification | `python3 tools/arena_test.py verify-run build/test-runs/<id>` |

Always report the concrete run ID from `build/test-runs/`, never just
`latest`.

## Evidence rules

1. Do not delete or edit a run directory to make it agree with a later run.
2. A retry is evidence, not absolution. Use `--repeat`; any failed attempt
   keeps the run failed.
3. QEMU exit code and semantic guest verdict are separate facts. Exit `0`
   alone is not a guest pass.
4. Intentional guest failures and kills must remain labelled as negative
   controls. Do not generalize their serial text into an unexpected failure
   without checking suite metadata and output.
5. If required evidence is missing, classify the run as failed/blocked, not
   passing with a caveat.
6. Do not modify tracked source files while a run is executing. The runner
   compares source status at start/end and will fail the run.
7. Treat `selection.json` as the run's contract. Do not cite a PASS unless
   `verify-run` succeeds on that concrete directory after extraction.
8. Attempt deadlines are monotonic and output-independent. A suite that
   prints continuously, stalls without a newline, or leaves descendants
   holding its pipe still times out and is reaped as a process group.

## Backend rules

- Source the canonical environment before guest work. In the reconstructed
  sandbox this is `~/t-env/canonical-env.sh`.
- Use explicit TCG for comparisons.
- The pinned emulator is QEMU 11.1.2 patched, SHA-256
  `e0ea2b1f5ae2a5aa65a455eb232e46e1743b5c327c48ca678630c214c69f26ff`,
  through the isolated-glibc wrapper. The unpatched control binary is
  `443c81fc9482493297ae7a3f37300bef3a82a2873e428f72884cf52d16e27284`.
- A rebuilt emulator is a new backend identity unless it is byte-identical.
- Stock QEMU may be useful for harness syntax checks, but it is not strict
  Phase 15 evidence because it does not provide the required genuine #XM.

## Build cache contract

The cache is enabled by the new runner through `ARENA_TEST_BUILD_CACHE=1`.
Its key is not a Git commit. It covers, among other things:

- contents of build-relevant source files, following symlink targets;
- semantic Cargo config from the repo, ancestors and `CARGO_HOME`;
- effective profile, target, features, flags, linker/wrapper selection and
  source-declared build-affecting environment (hashed selectively, never a
  whole-environment dump);
- compiler, target-library, host-tool and participating Python-module
  identities;
- manifests, locks, build scripts, linker scripts and reachable external
  path inputs;
- absolute repository path and generated inputs that can change the image.

If any supported input cannot be identified conservatively, reuse is
disabled with explicit blockers and the real build runs. A cache entry is
published only when complete, and lookup revalidates its enabled identity,
metadata and artifact hashes. A tampered or incomplete entry must never be
a best-effort reuse.

Use `--no-build-cache` only for controlled comparisons or cache debugging,
and say so in the handoff.

## Parallel execution contract

`--jobs N` with `N > 1` is experimental and opt-in.

- Pool workers use separate Git worktrees under `/tmp/arena-test-workers`.
- Each worker enters its own user, mount, and network namespace.
- The worktree is bind-mounted at `/workspace/ArenaOS`, and the entry command
  changes directory using that canonical spelling. This is required because
  Rust artifacts embed absolute paths.
- Workers share `/tmp/arena-test-cache` unless
  `ARENA_TEST_BUILD_CACHE_DIR` is explicitly supplied.
- Entries whose registry metadata says `serial` do not enter the pool.
  Source-mutating `worktree` entries may enter only because the mutation is
  confined to that worker checkout; the runner records status around the
  attempt and resets the worktree before its next suite.
- Parallel mode refuses untracked non-ignored files because workers are
  snapshot worktrees.
- Do not use parallel results as qualification evidence without explicit
  project acceptance. CPU contention can alter timing-sensitive behaviour.

## Adding or changing a test

1. Add or modify the suite itself.
2. If it belongs to `run_tests.sh`, update that runner only if the task is
   explicitly authorized to preserve old-runner equivalence.
3. Regenerate the registry:

   ```sh
   python3 tools/gen_test_registry.py
   python3 tools/arena_test.py check-registry
   ```

4. Inspect the generated entry. The generator decomposes `run_tests.sh`, but
   subprocess-launched guests and source mutations may need explicit
   overrides in `tools/gen_test_registry.py`.
5. Add or adjust `tools/test_map.json` when a new source area should select
   the suite. Prefer broad, understandable rules. Unknown paths must continue
   to fall back conservatively.
6. Run at least:
   - the new/changed entry directly;
   - `tools/test_arena_test_infra.py` for runner or harness changes;
   - `arena_test.py check-registry`;
   - an affected-selection `--explain` for a representative changed path.

Never add a test to `quick` solely because it is conceptually small. Quick
membership is a measured latency budget. For example, the complete AFS2 host
model remains full-only because it takes about 210 seconds in the reference
environment.

## Writing guest-driving code

- Gate commands on semantic readiness, not sleeps.
- A flat marker tuple is a conjunction. A nested tuple is an any-of group:

  ```python
  settle = (
      b"arena>",
      (b"packaged READY", b"boot namespace scan refused"),
  )
  ```

  In the example, either successful readiness or the explicit fail-closed
  refusal proves settlement. The suite's later assertions still decide which
  outcome is legal for that disk state.
- Do not weaken an assertion to accommodate pacing. Fix the gate, preserve
  the original failing evidence, and state the distinction.
- Record intentional kills as intentional. A killed boot must never become a
  pass through timeout handling.

## Source-mutation suites

Mutation suites are safety-critical test infrastructure. They must:

- prove the intended mutation changed the intended source;
- preserve mutation and restored-build evidence separately;
- restore byte-exact content even on failure paths;
- fail if restoration cannot be proven;
- remain serial in a shared checkout; use the isolated worker pool only when
  the registry marks the entry `worktree` and that isolation has been proven
  for its fixture/port behaviour.

Do not let the build cache confuse canonical and mutant builds. Their source
contents differ, therefore their fingerprints must differ; if they do not,
that is an infrastructure failure.

## Triage order for a failed run

1. Read `run.json` and `selection.json`: source commit/dirtiness,
   environment, selected entries/evidence expectations, omitted count,
   completion state, and final verdict.
2. Read the failing attempt's `result.json` and `cmd.json`.
3. Read `output.log` for the suite's own diagnosis.
4. Read each `boots/*/boot.json`, then its `serial.log` and
   `qemu-stderr.log`.
5. Compare `builds/*/build.json` hashes against the artifacts the suite
   expected.
6. Verify the run with `verify-run` before citing it.
7. State whether the evidence supports product failure, harness failure,
   backend/environment failure, or an intentional negative control.

## Qualification language

Use these terms exactly:

- Quick pass: development smoke only.
- Affected pass: selected development evidence only.
- Suite pass: focused evidence only.
- Full pass: complete development set, not automatically a phase
  qualification.
- Qualify pass: qualification-shaped evidence only if preconditions were met
  without `--force` and the applicable project authority accepts the runner.

Never describe an omitted suite as passed. The report's omitted count and
selection reason are part of the result.
