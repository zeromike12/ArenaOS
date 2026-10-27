# Running ArenaOS tests

ArenaOS now has one small test entry point:

```sh
python3 tools/arena_test.py quick
```

Run commands from the repository root. In the development sandbox, use the
canonical path `/workspace/ArenaOS`, because some reproducible build identities
include the absolute build path.

The existing `tools/run_tests.sh` has not been removed or changed. It remains
the reference runner until the project explicitly accepts the new runner for
qualification use.

## Choose the right command

### 1. I need a fast smoke signal

```sh
python3 tools/arena_test.py quick
```

`quick` runs the fast host checks, one real guest boot, and the
test-infrastructure controls. It is useful while editing.

A quick pass is **not qualification**. The terminal report and `run.json`
state the evidence level explicitly.

### 2. I changed some files and want relevant tests

First look at the selection:

```sh
python3 tools/arena_test.py affected --base main --explain
```

Then run it:

```sh
python3 tools/arena_test.py affected --base main
```

Selection is driven by `tools/test_map.json` and the source paths in
`tools/test_registry.json`. If the mapper does not understand a changed path,
it chooses the full registered set rather than guessing narrowly.
Documentation-only changes explicitly select the quick set.

### 3. I want one named test

List the registry IDs:

```sh
python3 tools/arena_test.py list
```

Run one or more:

```sh
python3 tools/arena_test.py suite m9-client-death
python3 tools/arena_test.py suite m84-crash m84-stage
```

To repeat a suite while preserving every attempt:

```sh
python3 tools/arena_test.py suite m9-client-death --repeat 5
```

If any repetition fails, the whole run is a failure. A later pass does not
hide an earlier failure.

### 4. I want the complete development run

```sh
python3 tools/arena_test.py full
```

`full` runs the registered set plus entries that previously lived outside the
old runner: Phase 16 targeted/host suites, the Phase 14 guest checks, and the
infrastructure controls. Use `list --mode full` to see the exact selection.

### 5. I need the qualification-shaped run

```sh
python3 tools/arena_test.py qualify
```

`qualify` adds the qualification-only gates and checks preconditions before
it starts: clean committed tree, canonical checkout path, and the pinned
patched-QEMU/isolated-glibc backend. If those conditions are not met, the
result is `BLOCKED`; it is never relabelled as a pass.

Do not use `--force` for a real qualification claim. It exists only to make
an explicitly blocked exercise visible in evidence.

## Sandbox environment

Guest tests need the pinned patched QEMU and firmware identities. Stock QEMU
is not a substitute: it lacks the genuine vector-19 behaviour required by the
strict Phase 15 checks.

In this workstream's reconstructed sandbox:

```sh
cd /workspace/ArenaOS
source ~/t-env/canonical-env.sh
python3 tools/arena_test.py env-info
```

`env-info` prints the CPU count, toolchain, QEMU identity, accelerator,
firmware paths, and related environment that the runner records. If those
values are absent or point at a different backend, fix the environment before
interpreting a guest result.

## Where the evidence is

Every invocation creates a new directory:

```text
build/test-runs/<run-id>/
```

Important files include:

```text
run.json                         results, timing, source/environment identity
environment.json                 backend and toolchain identity
source.diff                      uncommitted source state, when present
source-status.txt                Git status used for the run
attempts/<entry>/attempt-001/
  result.json                    attempt verdict and process exit
  output.log                     complete suite output
  cmd.json                       exact command, cwd, timeout, env overrides
  boots/<nnn>-<label>/
    boot.json                    QEMU exit, duration, kill/intent flags, hashes
    serial.log                   raw guest serial output
    qemu-stderr.log              emulator diagnostics
  builds/<nnn>-<label>/
    build.json                   build result and artifact identities
  outputs/                       files produced or changed under build/
```

The runner also maintains `build/test-runs/latest` as a convenience pointer.
Do not cite `latest` in a handoff; cite the concrete run ID.

Check that a preserved run is internally complete:

```sh
python3 tools/arena_test.py verify-run build/test-runs/<run-id>
```

Missing or altered evidence makes verification fail.

## How to read a result

- `PASS`: every selected entry and every recorded attempt passed, and the
  source tree did not change unexpectedly during the run.
- `FAIL`: at least one entry or attempt failed, timed out, lacked required
  evidence, or the source changed during execution.
- `BLOCKED`: mandatory preconditions were not met. This is an incomplete
  evidence state, not a test pass.

QEMU process exit `0` by itself does **not** mean the guest passed. Guest
suites must see their required semantic markers and must not see fatal guest
markers. Intentional negative controls are marked as expected failures or
intentional kills in metadata; their suites pass only when the negative
control behaves as specified and the restored/green path passes.

## Build cache and parallel runs

The new runner enables a content-addressed build cache by default. A cached
EFI/ESP is reused only when source-file contents, build environment,
toolchain identity, and absolute build path all match, and restored artifacts
are hash-checked. To compare against the historical behaviour:

```sh
python3 tools/arena_test.py quick --no-build-cache
```

Parallel execution is available but is not the default:

```sh
python3 tools/arena_test.py suite smoke-m1 host-heap --jobs 2
```

Parallel entries run in isolated Git worktrees and network namespaces.
Source-mutating and explicitly serial entries remain serial. Use serial
execution for qualification evidence unless the project has accepted a
parallel qualification methodology.

## More detail

- Test inventory: `INVENTORY.md`
- Baseline and timing evidence: `BASELINE.md`
- Q-1 investigation: `Q1-INVESTIGATION.md`
- Agent-focused rules: `AGENT-GUIDE.md`
- Workstream outcome and limitations: `FINAL-REPORT.md`
