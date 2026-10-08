# 03 — Experimental C runtime prototype and native C execution milestone

Status: **experimental**, not part of the production image. Tree:
`experiments/c-runtime/` (untracked by any production build). Orchestrator:
`experiments/c-runtime/run.py`.

Evidence labels: **[GUEST]** executed under ArenaOS in QEMU, serial captured
to `experiments/c-runtime/build/guest-serial-*.log`; **[HOST]** host-only
test (not an ArenaOS execution); **[STATIC]** checked without executing.

## 1. Milestone result (the headline)

> **A freestanding, statically linked C program was compiled, linked, booted
> as a ring-3 process on ArenaOS under QEMU, printed to the console,
> allocated and modified memory, verified it, and reported
> `CRT-PROBE RESULT PASS (6/6)`.** [GUEST, both compilers]

Verbatim probe output from the GCC/GNU-ld build (`guest-serial-gcc.log`):

```
crt-probe: start (freestanding C, ArenaOS prototype runtime)
crt-probe: T1 stack [0x200001200000,0x200001240000) size=262144 sp=0x20000123fea4 tls=1
crt-probe: T1 entry-stack-tls PASS
crt-probe: T2 tls init=0x5a5b zero=2
crt-probe: T2 tls PASS
crt-probe: T3 formatted="-42|    7|12    |-00003|ff|BEEF|123456789abcdef0|-9|str|abc|Z|%|ab  |44" len=71
crt-probe: T3 format PASS
crt-probe: T4 heap reserved=1024 committed=1024 live=1 (pre 1) reuse=195 refused=2 bad_frees=2 hogs=47
crt-probe: T4 memory PASS
crt-probe: T5 memcpy/memmove/memset/memcmp/strlen/strcmp verified
crt-probe: T5 strings PASS
crt-probe: T6 clock start=7063946 us, yield-loop=315205 us, sleep(20000)=20204 us, monotonic_violations=0
crt-probe: T6 time PASS
CRT-PROBE RESULT PASS (6/6)
```

What each group proves on the guest:

| Group | Proves | Result |
|---|---|---|
| T1 | `_start` ran on the image; the 256 KiB VM stack is the active stack; TLS is installed (`arena_tls_ready()==1`) | PASS |
| T2 | `static __thread` initial data (`.tdata`) and zeroed TLS (`.tbss`) are at the correct FS-relative addresses | PASS |
| T3 | Bounded `printf` subset produces exact text (flags, width, precision, length modifiers, `%%`), with truncation reporting full length | PASS |
| T4 | 1,024-page reservation; allocations of many size classes; reuse of freed blocks; exhaustion refused; double-free and interior-free counted and ignored; counters close exactly | PASS |
| T5 | `memcpy/memmove/memset/memcmp/strlen/strcmp/strncmp` on real data | PASS |
| T6 | Monotonic clock; sleep(20 ms) measured at 20,204 µs; 0 monotonicity violations over 2,000 yields | PASS |

The milestone is **guest-executed**, not simulated. The harness stops only on
the probe's own result line, or on a timeout, or on QEMU exit. A FAIL would
have been reported as FAIL.

### Non-interference evidence

The probe is spawned once at boot, before the shell, with no capabilities.
The same unpatched tree was booted as a control (`run.py guest --baseline`):

| Check (serial counts) | Baseline (unpatched) | GCC probe | Clang probe |
|---|---|---|---|
| Milestone suites run by this boot (M1–M7, M11, M12) | 9 suites, all PASS | 9, all PASS | 9, all PASS |
| `permissiond:` lines | 2 | 2 | 2 |
| `packaged:` lines | 4 | 4 | 4 |
| `depcheck:` lines | 3 | 3 | 3 |
| `netd:` lines | 38 | 38 | 38 |
| `FAIL`/`ERROR` lines | 3 (kernel negative controls) | 3 | 3 |
| Probe starts / results | 0 / 0 | 1 / 1 | 1 / 1 |
| Shell prompt and halt | yes | yes | yes |

The 3 `FAIL`/`ERROR` lines are the kernel's own heap negative-control messages
(`free rejected: DoubleFree ...`), present in the baseline too.

**Not observed:** the probe's exit status. `main` returns 37 on success and the
kernel's `SYS_THREAD_EXIT` records it, but the serial log does not print exit
codes, so the value 37 is not independently proven. The PASS line from `main`
is the observed evidence. [GUEST, limitation F8 in report 01]

## 2. Implemented files (all experimental)

| Path | Role | Evidence |
|---|---|---|
| `include/arena/abi.h` | C mirror of ABI v1 numbers and the six-argument syscall wrapper | STATIC (cross-check, 13 constants) |
| `include/arena/rt.h` | public runtime API | compiled, used by all tests |
| `include/arena/string.h` | mem/str prototypes | compiled |
| `src/crt0.c` | `_start`, stack switch, TLS install, `.init_array`, `main`, exit | GUEST |
| `src/rt.c` | `arena_exit`, `arena_write`, clock, yield, busy sleep | GUEST |
| `src/alloc.c` | size-class allocator, 16 B to 1 MiB, 16-byte header, lazy commit | HOST + GUEST |
| `src/string.c` | rep-movsb/stosb memcpy/memset, overlap-safe memmove, loop-based compare/len | HOST + GUEST |
| `src/stdio.c` | bounded printf subset (no floating point) | HOST + GUEST |
| `src/vm.c`, `src/vm.h` | VM backend: guest uses `SYS_VM_RESERVE/COMMIT` with reply checks; hosted uses mmap | HOST + GUEST |
| `link/arena-user.ld` | static ET_EXEC script with PT_TLS and TLS symbols | STATIC + GUEST |
| `app/crt_probe.c` | the six-group freestanding probe | GUEST |
| `host/host_main.c`, `host/test_runtime.c` | host glue and the host-only test suite | HOST |
| `patches/0001-boot-probe-c-prototype.patch` | scratch-only kernel patch (see §4) | GUEST |
| `run.py` | build, audit, abi, host, guest orchestration | all |

## 3. Host-only results (not ArenaOS execution)

`python3 experiments/c-runtime/run.py host`: **HOST-ONLY RESULT PASS (42
checks)** under GCC 12.2 with AddressSanitizer and UBSan, `-fno-sanitize-recover=all`.

- Formatting: 20,000 randomized format strings, byte-identical to glibc
  `snprintf` (flags, width, precision, modifiers, `d/i/u/x/X/lu/llx`, strings).
- Memory and strings: 5,000 randomized `memmove`/`memcmp`/`memset` cases
  compared with libc; overlap cases checked by hand.
- Allocator contract: alignment, `malloc(0)`, realloc in place and realloc
  growth preserving bytes, `calloc` overflow refusal, three counted bad frees.
- Allocator stress: 200,000 operations against a shadow model. Zero corruptions,
  zero overlaps, zero unexpected bad frees, and live counts match the shadow
  (50,286 allocations, 76,606 frees, 10,923 refusals, 536 over the 1 MiB
  ceiling, reuse 92,829, committed 1,024/1,024 pages).

The 10,923 refusals are a measured **limitation**, not a pass criterion: the
4 MiB heap fragments without coalescing under this workload. See §6.

## 4. Scratch-only kernel patch (how the probe was booted)

Production files are not modified. `run.py guest` copies the tree (excluding
`.git`), applies `patches/0001-boot-probe-c-prototype.patch` in the copy,
builds the probe, builds the kernel image with `tools/build.sh --image`, and
boots it through the repository's own `tools/mtest.py` `run_qemu()` harness.

The patch makes two changes, both marked EXPERIMENTAL:

1. `spawn.rs`: a **sentinel image id** `CPROBE_IMG_ID = 0xC0DE_0001` maps to
   the probe ELF (`include_bytes!` of the scratch-built ELF) in `prepare()`.
   The `dynamic` predicate excludes it (`img_id != CPROBE_IMG_ID`), so the
   sentinel never pins an `image_registry` slot.
2. `entry.rs`: one `spawn_init(CPROBE_IMG_ID, &[], None)` immediately before
   the shell spawn.

**Why a sentinel and not an existing image slot:** `spawn.rs` maps static ids
0 to 26 to embedded images. The boot path uses these ids: for example 16
(`TIMERTEST`, spawned by `m7.rs`), 19 (`SERVICEMGR`), 20 (`DEPCHECK`),
24 (`PERMISSIOND`), 25 (`PERMAPP`) and 26 (`PACKAGED`). An earlier attempt that
overwrote id 24 therefore replaced permissiond. That run is **not evidence** (permissiond and packaged
never started, and the probe ran as the permissiond service, twice). It is
superseded and recorded here so nobody relies on it.

**Harness:** the boot runs every milestone suite before the services and the
shell, so the probe can only run after the full fixture set passes. The
harness (`tools/mtest.py` `run_qemu`) attaches the scratch disk, the NIC with
host DNS/TCP fixtures, RNG, keyboard and the console (virtio-serial). The
console chardev is `server=on,wait=on`, so QEMU does not finish starting until
its console actor is connected; a hand-rolled boot without that actor hangs
before firmware runs. The harness feeds `shutdown` only after the probe's
result line (`PROBE_FEED`).

## 5. Bugs found and fixed (the guest run earned its keep)

| # | Symptom | Root cause | Fix | Caught by |
|---|---|---|---|---|
| B1 | clang build contained 271 SSE instructions (74% from auto-vectorization, 26% from aggregate lowering) | `-mgeneral-regs-only` did not stop zig clang 22.1.8 emitting SSE | added `-mno-sse -mno-sse2 -mno-mmx -mno-80387`; audit enforces 0 | STATIC |
| B2 | probe T2 `tls init=0x81 zero=23132` | linker script `__arena_tls_memsz = SIZEOF(.tdata)+SIZEOF(.tbss)` omitted alignment padding (16 vs 24); crt0 placed TP 16 bytes too high | memsz spans `.tdata` to end of `.tbss`; audit compares symbols with PT_TLS; negative control reproduces the failure | GUEST, then STATIC negative control |
| B3 | probe T4 `live=1` failure | the probe measured against zero, but crt0's own TLS block is a legitimate live allocation | probe measures deltas against its own start | GUEST |
| B4 | M5 halt, then M7 halt (`no DNS response`) | the first boot harness omitted the devices and fixtures `mtest` attaches by default | boot via `mtest.run_qemu()` from the scratch tree | GUEST (boot logs) |
| B5 | probe ran as permissiond (see §4) | image id 24 is used at boot | sentinel id | GUEST (baseline comparison) |

## 6. Limitations (measured or known)

- **Allocator fragmentation.** No coalescing: 10,923 refusals in the host
  stress run. A C program with churny, mixed-size allocations can exhaust its
  4 MiB reservation even while far below the total. Fix direction: coalescing
  free runs or a larger region with growth, both in the runtime (no ABI change).
- **stdout is the serial backdoor.** `SYS_DEBUG_WRITE` has no capability
  check (report 01, F1). The prototype uses it because no C stream binding
  exists. It must not be the production contract.
- **No C threads, mutexes or futex-equivalents** (report 01, F6).
- **No files, sockets, time zones, argv, environment or stdin.**
- **No floating point** (the kernel saves no SSE state; the contract forbids it).
- **TLS is runtime-managed.** The loader ignores PT_TLS (F3).
- **Exit status not observed on serial** (F8).
- **Sleep is a yield-polling busy wait** (`arena_busy_sleep_us` loops on the
  clock with `arena_yield`). It does not use `SYS_TIMER_ARM`, so the 20 ms
  measurement shows that the clock advances and that yielding works. It is not
  evidence of timer accuracy.
- **T6 is timing-sensitive under emulation.** In the first probe run the
  shell's `shutdown` arrived before T6 printed, so T6 was cut off. The
  result-gated feed (`PROBE_FEED`) fixed that. The passing run measured
  sleep(20 ms) as 20,204 µs under QEMU TCG.
- **Single run per compiler.** The two guest runs are a sample, not a
  stability study. `run.py guest` should be repeated before anyone treats
  this as a regression baseline.
- **`abi` cross-check** accepts kernel-missing constants (F9).

## 7. Not done in this milestone (deliberately)

- No Linux compatibility, no dynamic linker, no PIE/ASLR (Luna's Phase-14 scope).
- No change to production kernel or userspace.
- No ABI change landed. The proposed ABI changes are in ADR-0110 (Proposed).
- No SDL3, SuperTuxKart or Blender port (report 04 covers dependencies only).

## 8. How to reproduce

```
source tools/dev-env/env.sh
python3 experiments/c-runtime/run.py build           # both compilers by default
python3 experiments/c-runtime/run.py audit           # STATIC checks
python3 experiments/c-runtime/run.py abi             # ABI cross-check
python3 experiments/c-runtime/run.py host            # HOST-ONLY tests
python3 experiments/c-runtime/run.py --cc gcc guest  # GUEST (several minutes)
python3 experiments/c-runtime/run.py --cc gcc guest --baseline   # control
```
