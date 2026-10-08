# C1 final report: native C application platform (experimental)

Branch: `arena/f89ea987-arenaos`. Base: `74ace4f` (Phase-13 release). Status:
**experimental prototype. Nothing here is a production interface, and nothing
has been merged, released, or put in a PR.** Final commit: see the last section.

Evidence labels, used throughout and never mixed:

- **IMPLEMENTED**: code exists in the repository.
- **HOST-ONLY**: host build with ASan and UBSan (`run.py host`). This is not
  kernel-side memory safety, and not guest evidence.
- **STATIC**: source reading, `objdump` audit, or ABI cross-check. Nothing was
  executed under ArenaOS.
- **GUEST**: executed under ArenaOS in QEMU, through the real install and
  launch path, judged by an exact-verdict harness.
- **PROPOSED**: an ADR or design written for review. Not landed.
- **BLOCKED**: needs a decision or change outside C1.
- **GAP**: required by the assignment and not done.

## 1. Acceptance target

An ordinary signed C application launches inside ArenaOS through the real
capability lifecycle, uses granted native streams, and exits with an observable
status.

**Status: met, GUEST, on the final tree.** The signed package `zzz-cnative.apb1`
(app `org.arenaos.cnative`, flags 28 = HEADLESS | STANDARD_STREAMS |
NATIVE_SYNC) was installed through the desktop's protected install path, found in
All Applications, and launched through the normal headless lifecycle. The app
wrote to granted stdout and stderr, read stdin without blocking, allocated,
freed, and ran threads, then exited with status 57. The desktop logged
`child Process-cap exit status=57`. The stream-less negative package
`aaa-nostream.apb1` (flags 20, no STANDARD_STREAMS) was launched the same way and
exited with status 58, which shows that a missing stream grant fails cleanly.

## 2. Requirement status

| Req | Status | Evidence | Notes |
|---|---|---|---|
| C1.1 stdin/stdout/stderr through granted caps; bounded read/write; partial transfer; EOF; peer closure; no SYS_DEBUG_WRITE; no new syscalls; no wire-format change | **IMPLEMENTED**; GUEST for stdout, stderr, stdin, EOF, backpressure, and missing grant; HOST-ONLY for partial writes and write-error reporting | GUEST: 5/5 clang, 5/5 gcc (`summary-*.json`); T2 (`backpressure: 2048 bytes`), T3, `native stream channel stdout/stderr reached EOF`, stream-less exit 58. HOST-ONLY: `test_stdio_partial_writes`, `test_stdio_write_error` (2e8fae6). STATIC: no `SYS_DEBUG_WRITE` on a supported path (`arena_legacy_write` is opt-in). | Stream protocol unchanged. The u32 ring-wrap defect is pinned by a host test and is **BLOCKED** (§5, ADR-0111). |
| C1.2 signed native C app, APB1, normal lifecycle, entry, ABI validation, TLS, runtime init, stdout, allocate, time, logic, defined exit | **IMPLEMENTED**; **GUEST** 5/5 per compiler on the final tree | §4 | Uses the RFC 8032 development fixture (test-only). The boot probe is kept as a legacy regression (crt-probe, 3/3 per compiler). |
| C1.3 threads: create, join, detach, exit, yield; mutex; condvar; once; cleanup; TLS isolation; quota; stale handle | **IMPLEMENTED**; **GUEST** T8 (5 checks), T9 (34 checks) | GUEST, 5/5 clang and 5/5 gcc | See §3.3 for limits. The **independent C1.3 harness is a GAP**: T9 lives in the same app as the C1.2 groups. |
| C1.4 allocator hardening; ownership validation; realloc; invalid, interior, and freed pointers; overflow; alignment; bounded coalescing; VM failure without corruption; thread safety | **IMPLEMENTED**; HOST-ONLY (full set); GUEST for T5 (13 checks), concurrent use in T9, and crt-probe T4 (`refused=2 bad_frees=2`) | HOST-ONLY: 598 checks on `2e8fae6`, including `test_vm_commit_refusal` and `test_vm_reserve_refusal` (commit `f38ad6a`) | **GAP**: guest-side VM commit failure injection is not done. The crt-probe refusal counter hits the heap's own ceiling, not a kernel commit failure. |
| C1.5 SDL3 handoff | **IMPLEMENTED** as a document (`C1-SDL3-HANDOFF.md`) | Doc only; no SDL3 port, no build, no Arena 1 branch change | Lists exact blockers (§5). |
| CPU FP/SIMD boundary audit (core-OS dependency, Luna review after Phase 14) | **IMPLEMENTED** as an audit (`C1-FPSIMD-BOUNDARY.md`); **no kernel FP code written** | STATIC: kernel EFI 0 SIMD and x87 instructions; all C ELFs `simd_or_x87_count 0` | No FP support is claimed. The decision is recorded for Luna (§5). |

## 3. What was built and how far it goes

### 3.1 Runtime (`experiments/c-runtime/`)

- `src/startup.c`: ARST v2 gate (slot 0, one 4 KiB page, validated and then
  destroyed).
- `src/streams.c`: stdio over the granted StreamSet and StreamWake. Missing
  grant gives `ARENA_E_NO_STREAMS` (fixed in `9238c97`).
- `src/stdio.c`: the printf subset. Write errors are now reported, and short
  writes are looped (`2e8fae6`). Before that, `arena_printf` ignored the
  write result.
- `src/threads.c`, `src/sync.c`, `src/crt0.c`, `src/rt.c`, `src/vm.c`.
- `src/alloc.c`: 16 MiB reservation per process, committed in 64 KiB chunks,
  power-of-two blocks from 32 B to 16 MiB, 16-byte headers, bounded coalescing,
  one spinlock. A contended acquire yields through `arena_backoff()`, which is
  `SYS_THREAD_YIELD` in the guest and `sched_yield` on the host.
- `src/string.c`: `arena_memcpy`, `memmove`, `memset`, `memcmp`, `strlen`,
  `strnlen`, `strcmp`, `strncmp`.

### 3.2 Application and harness

- `app/c_native_app.c`: nine groups (T1 to T9), 92 group checks, plus one final
  check, which gives `checks=93`. Exits 58 if stdio setup fails and 57 on a full
  pass.
- `guest_native.py`: builds the signed packages from the RFC 8032 test fixture,
  seeds them into the desktop disk, installs and launches both through the GUI,
  and judges with exact verdicts (`[58, 57]` exit order, RESULT line, nine
  group lines). Writes `bundle-identity.json` and `summary-<cc>.json`.
- `run.py`: `build`, `host`, `abi`, `audit`, `guest`. The crt-probe guest
  requires the exact `CRT-PROBE RESULT PASS (6/6)` line.

### 3.3 Limits of the C1.3 evidence

- T9 covers create, join, detach, stale join, a second join, a fifth-thread
  quota refusal, condvar notify-one and notify-all, a timed wait, a stale mutex
  check, and once-init. It does not cover a long soak, many more threads
  than the limit over time, or thread-local destructors (not implemented).
- Thread stacks are 56 KiB usable (pages 2–15 of a 16-page region, with page 1
  as a guard). The main stack is 256 KiB.
- Detached workers must be reaped before `main` returns, because the last thread
  to exit sets the process status. The app waits, with a bound, until
  `arena_thread_count()==1`. Without that wait, a worker that outlived main
  produced exit 5 in an earlier run.

## 4. Guest evidence (final tree)

Sources at commit `bcdfb95`. The source hashes below are the ones recorded in
the summary files.

| Image | SHA-256 | Notes |
|---|---|---|
| `c-native-gcc.elf` | `c14c712d8c6c5750d534e0e5bbe5c32b67ce2c6686618d0ab553550af0201c06` | 47 592 B |
| `c-native-clang.elf` | `a81a154480754e3185990705d566029133b82767750b27d59ef3e2cac296d2fc` | 51 592 B |
| `crt-probe-gcc.elf` | `6cff914210af957e6d6e9d7896f05666a5920d2866092cd1802976b9182e94d3` | |
| `crt-probe-clang.elf` | `c5cf3124c3888bbfd2c5fde8319a6f88a2dd12b669b75148beb63c609178c3a8` | |
| native bundle, gcc (flags 28) | `480beaa8c592d4ed65a46b564a98be3492587039548d32d30b872d0e005a61d3` | 48 376 B |
| native bundle, clang (flags 28) | `a63f593d4571cf4a1135b0f22f4962e504c8bea7ef5c42cd5f59fa001c5e83a8` | 52 376 B |
| stream-less bundle, gcc (flags 20) | `c364896122fa70ea106597ffadb6b59b042d0dbdd00b4ffb41a3fa818bbaa999` | |
| stream-less bundle, clang | not recorded | GAP: `bundle-identity.json` holds only the last-built compiler's negative package. |
| boot image `arena-boot.efi` | `b208d2c96eb13734fb9adf5a5aeb7a53e4a450e0b4d5ac391f8ec285063134a4` | Same hash in `build/` and `kernel/target/`. |

Source hashes (`sha256`): `src/stdio.c` `aacb806e…`, `src/streams.c` `69623fbc…`,
`src/alloc.c` `52232784…`, `src/vm.c` `06a1e381…`, `src/sync.c` `e76e8d37…`,
`src/threads.c` `6fa2a264…`, `src/startup.c` `498d0289…`, `app/c_native_app.c`
`ec98574b…`, `guest_native.py` `1d0ba061…`, `run.py` `177c7ef6…`,
`link/arena-user.ld` `6d3d2bbd…`.

| Run set | Result | Exact PASS count | Artifacts |
|---|---|---|---|
| Native app, clang, `--runs 5` | **5/5**, each `RESULT PASS groups=9/9 checks=93 failed=0`, exit lines `[58, 57]`, boot rc 0 | 5/5 | `build/guest-native/summary-clang.json`, `serial-c1-native-clang-run{1..5}.log` |
| Native app, gcc, `--runs 5` | **5/5**, same verdict | 5/5 | `summary-gcc.json`, `serial-c1-native-gcc-run{1..5}.log` |
| crt-probe, gcc (`run.py --cc gcc guest`) | **3/3**, `CRT-PROBE RESULT PASS (6/6)` | 3/3 | `/tmp/crt5-gcc*.txt` (scratch) |
| crt-probe, clang | **3/3**, same | 3/3 | `/tmp/crt5-clang*.txt` (scratch) |

Each native run also showed: `APB1 installed; signed version=1`, both `application
retired` lines, `backpressure: 2048 bytes`, stdout and stderr EOF, and exit
receipts 58 and 57.

### 4.1 Failure history on the same family, reported in full

- Before the judge fix, `guest_native.py --cc clang --runs 1` on the same ELFs
  failed once with `missing group T2 granted-streams`, while the RESULT line said
  9/9. The full T2 line was in the serial log. The desktop had relayed it in two
  chunks, with a kernel `spawn` log line between them (`T2 granted-st[arena INFO
  sched] spawn(...)` then `reams PASS checks=5`). The stdout ring wraps mid-line,
  so the desktop drains two pieces. This is console interleaving, not dropped
  output. `arena_stream_write` already loops until every byte is accepted.
- The judge now matches app lines on a view with whole kernel and desktop log
  fragments removed. Exit receipts and the RESULT line still use exact matching
  on the raw log. A split that cannot be repaired fails the run; it cannot pass.
  Re-judging the saved failing log gave `pass: true`. Five fresh runs per
  compiler then passed.
- Earlier stale-evidence history (ELFs before `streams.c` and `c_native_app.c`
  changed) is superseded and is not counted above.

## 5. Proposed, blocked, and open

| Item | Status | Owner / decision needed |
|---|---|---|
| **Stream ring counters wrap at 2^32** (`counter % 768` jumps by 256; `2^32 mod 768 = 256`). Unread bytes can be overwritten. Pinned as KNOWN-DEFECT in host tests. | **BLOCKED** on a wire-format change. **PROPOSED**: `docs/adr/0111-stream-ring-counter-wrap.md`, options u64 counters (version 2) or power-of-two capacity. | M (Phase-13 streams) and C review. No change landed. |
| **`SYS_DEBUG_WRITE` has no capability check** (F1). C1 avoids it, but the kernel still exposes it. | **PROPOSED** (ADR-0110 Decision 3). Not landed. | Core OS and architecture review. |
| **Loader ignores `PT_TLS`** (F3). The C runtime installs TLS itself. | **OPEN** question in ADR-0110. | Luna loader owners. |
| **No FP/SIMD for C.** The kernel has no FP state save or restore. C images are audited to contain no SSE or x87. | **BLOCKED** for FP. Decision for Luna after Phase 14: keep the no-FP contract (recommended now), or enable FP with per-thread state (kernel ADR). | Luna. `C1-FPSIMD-BOUNDARY.md`. |
| **Firmware CR4/CR0 FP bits unchecked** (CR4.OSFXSR, OSXSAVE, CR0.EM, CR0.TS are not read or asserted). | **OPEN** (no effect while no code uses FP). | Core OS, with the FP decision. |
| **Target-feature baseline of the kernel target** (earlier note). | **UNVERIFIED** in this session: `rustc` is not on the sandbox PATH. Not relied on. | Re-check when the toolchain is available. |
| **Guest-side VM commit failure** (C1.4). | **GAP**: host injection is done. A guest test would need a kernel-side fault or quota that is not reachable from userspace today. | C1 follow-up or Core OS. |
| **Independent C1.3 harness.** | **GAP**: T9 is inside the C1.2 app. | C1 follow-up. |
| **Stream-less clang bundle hash** not recorded. | **GAP** in the harness. | C1 follow-up. |
| **`tools/startup_abi.py` capability maximum is 4**; the Rust platform and the C header use 7. The Python validator refuses 5 to 7 capabilities, and no gate checks that file. | **FOUND, not changed** (M9 in the integration contract). | M (Phase-13 tooling): decide, then align and add a test. |
| **Kernel-side memory safety of the C heap.** | Not proven. HOST-ONLY results are not kernel evidence. | Core OS. |
| **SDL3.** Needs a static, no-optional-backend build (or a dlopen-style loader), FP (blocked), and Arena 1's video and audio backends. Not started. | **BLOCKED**, listed in `C1-SDL3-HANDOFF.md`. | Arena 1. |

ADR-0110 remains **Proposed**. C1 added the status addendum only. ADR-0111 is new
and **Proposed**.

## 6. Reproduction

```
cd experiments/c-runtime
python3 run.py build                     # rebuilds ELFs; rc 0
python3 run.py host                      # HOST-ONLY; expect "HOST-ONLY RESULT PASS (598 checks, known protocol defect pinned)"
python3 run.py abi                       # ABI constants vs userspace/abi.rs; rc 0
python3 run.py --cc gcc audit            # STATIC; rc 0, simd_or_x87_count 0
python3 run.py --cc clang audit          # STATIC; rc 0
python3 guest_native.py --cc clang --runs 5   # GUEST; expect "5/5 exact PASS"
python3 guest_native.py --cc gcc --runs 5     # GUEST; expect "5/5 exact PASS"
python3 run.py --cc gcc guest            # crt-probe regression; expect "CRT-PROBE RESULT PASS (6/6)"
python3 run.py --cc clang guest
```

The guest commands need QEMU and the normal Phase-13 disk images. Serial logs,
summaries, and ELFs are generated under `experiments/c-runtime/build/`, which is
not committed.

## 7. Commits on this branch

| Commit | Scope | Evidence |
|---|---|---|
| `fe07eab` | prior prototype and docs | historical |
| `da0bfa4` | runtime, allocator hardening, host tests (**host-only**) | HOST-ONLY |
| `c244239` | C1.2 harness and native app | superseded GUEST |
| `b4e790c` | crt-probe follows the C1 heap contract (legacy regression kept) | GUEST 1/1 per compiler at the time (superseded by the 3/3 runs above) |
| `871f178` | C1.4 concurrent allocator check (T9) | superseded GUEST |
| `f38ad6a` | C1.4 host VM reserve and commit fault injection | HOST-ONLY |
| `9238c97` | C1.1 missing stream grant fails cleanly | HOST-ONLY |
| `2e8fae6` | C1.1 printf short-write loop and error reporting | HOST-ONLY |
| `76a55ee` | audit rule: explicit x87/FP-state mnemonic list | STATIC |
| `bcdfb95` | stream-less package, stderr-first probe, console-safe judge | GUEST 5/5 + 5/5 |

Nothing was pushed to `main`, merged, released, or opened as a PR. Luna's branch
`arena/phase14-native-pie-aslr` and Arena 1's graphics branch were not touched.
