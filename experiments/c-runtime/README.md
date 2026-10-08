# ArenaOS C runtime prototype (EXPERIMENTAL)

This directory is an experiment. It is **not** part of the production image,
and no production build reads it. Analysis and decisions are in
[`docs/compat/`](../../docs/compat/README.md); the proposed contract is in
[ADR-0110](../../docs/adr/0110-native-c-runtime-profile-proposal.md) (Proposed).

## What is here

| Path | Purpose |
|---|---|
| `include/arena/` | C mirror of ABI v1 (`abi.h`) and the runtime API |
| `src/` | `crt0.c` (startup/TLS), `rt.c` (exit, write, clock), `alloc.c` (allocator), `string.c`, `stdio.c` (bounded printf), `vm.c` (VM backend) |
| `link/arena-user.ld` | static ET_EXEC link script with PT_TLS and `__arena_tls_*` symbols |
| `app/crt_probe.c` | freestanding guest probe: T1–T6, ends with `CRT-PROBE RESULT` |
| `host/` | host-only tests (glibc differential, ASan+UBSan) |
| `patches/0001-boot-probe-c-prototype.patch` | scratch-only kernel patch used by `guest`; applied to a copy, never to the repo |
| `run.py` | orchestrator |

## Commands

Requires the repo dev environment (`source tools/dev-env/env.sh`) and, for the
guest path, QEMU/EDK2 as used by `tools/mtest.py`.

```
python3 experiments/c-runtime/run.py build                 # both compilers (gcc, clang via zig)
python3 experiments/c-runtime/run.py audit                 # STATIC: loader rules, no SSE/x87, TLS symbols
python3 experiments/c-runtime/run.py abi                   # STATIC: C ABI numbers vs userspace/abi.rs
python3 experiments/c-runtime/run.py host                  # HOST-ONLY: 42 checks, ASan+UBSan
python3 experiments/c-runtime/run.py --cc gcc guest        # GUEST: scratch tree, boot, probe
python3 experiments/c-runtime/run.py --cc gcc guest --baseline   # GUEST control, unpatched tree
```

Each subcommand runs once per invocation. `--cc` can repeat (`--cc gcc --cc clang`).

Outputs go to `experiments/c-runtime/build/` (git-ignored by the repo's
`build/` rule). `build/summary.json` holds the last run's verdicts;
`build/guest-serial-<cc>.log` holds the serial log of the last guest run.

## Evidence labels

- **GUEST**: executed under ArenaOS in QEMU. `guest` reports `guest executed:
  True` only when a `CRT-PROBE RESULT` line was captured from serial. The
  PASS/FAIL verdict is the probe's own printed line, read from that log.
- **HOST-ONLY**: the host test binary. It is not an ArenaOS execution.
- **STATIC**: checked from the ELF or disassembly without running it.

## Known limits

- The guest path depends on `tools/mtest.py` and the full M1–M12 fixture set.
  Each compiler run takes several minutes of QEMU time.
- `SYS_DEBUG_WRITE` is used for stdout. The kernel does not check its
  capability (report 01, F1). Do not treat it as a C contract.
- The allocator has no coalescing. It refuses under heavy churn (report 03 §6).
- No threads, no files, no sockets, no argv/env, no floating point.
- Exit status is not printed to serial (report 01, F8).

## Scope boundaries

- No Linux binary compatibility. No dynamic linker. No PIE or ASLR (Luna's
  Phase-14 scope).
- No change to `kernel/`, `userspace/`, `tools/`, or other production paths.
