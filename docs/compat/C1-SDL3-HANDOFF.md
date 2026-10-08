# C1 → Arena 1: native C runtime handoff for an SDL3 port (C1.5)

Audience: Arena 1 (graphics branch). Purpose: let Arena 1 decide what SDL3 needs
from the C runtime, and what is missing, without reverse-engineering the
prototype. **This is a handoff, not a port.** C1 did not port SDL3, did not build
it, did not implement graphics, and did not change Arena 1's branch.

Status of the runtime: **experimental**, under `experiments/c-runtime/`, proposed
in ADR-0110 (Proposed). Every statement below is labelled:
**GUEST** (run under ArenaOS by the C1 harness), **HOST-ONLY** (ASan+UBSan host
build, not kernel evidence), **STATIC** (source or disassembly), or **BLOCKED**.

Companion documents: `C1-INTEGRATION-CONTRACT.md` (what ArenaOS provides),
`C1-FINAL-REPORT.md` (evidence and hashes), `C1-FPSIMD-BOUNDARY.md` (FP
decision), `docs/adr/0110-native-c-runtime-profile-proposal.md`,
`docs/adr/0111-stream-ring-counter-wrap.md`, `docs/compat/04-compat-strategy.md`
(SDL3 verdicts).

## 0. Bottom line for SDL3

- **Not portable as-is.** SDL3 needs floating point (**BLOCKED**, §8), optional
  dynamically loaded backends (**BLOCKED**, §10), libc names that the runtime
  does not export (§9), and video and audio backends that are Arena 1's work.
- **Usable now for a non-FP, non-graphics C program** that needs: a signed
  headless app, granted stdio, a heap, threads (up to 4 user threads), mutexes,
  condition variables, and monotonic time (**GUEST**, §11).
- Report 04's decision is unchanged: (a) native source ports first, then (b) a
  bounded, broker-only library later, and (c) Linux ELF compatibility rejected.
  C1 finds that SDL3 is not reachable under (a) until blockers B1, B2, and B4
  (§13) are resolved. Those are FP, dynamic backends, and the libc shim.

## 1. Calling convention

- Target: x86-64 System V ABI, long mode, user ring 3. Static executable
  (`ET_EXEC`), **not PIE**, linked at `0x200000` by `link/arena-user.ld`.
- Entry: `_start` in `src/crt0.c` → `arena_crt_start` → startup gate (ARST v2,
  §3) → stack switch → TLS → `.init_array` → `main(argc, argv)` → `arena_exit`.
- `int main(int argc, char **argv)`. `argv[0]` is the application id on the
  headless path. `argc` and `argv` come from the verified ARST block (**GUEST**,
  T1 checks 10 items including this).
- Return value of `main` becomes the process exit status observed by the
  supervisor. The last thread to exit sets it (**GUEST**: exit receipts 57 and
  58).
- Compiler flags used for every C1 image (**STATIC**; the same list in
  `run.py` for gcc and clang; the zig command line for clang is shown):
  `-std=gnu11 -O2 -ffreestanding -fno-builtin -fno-stack-protector
  -fcf-protection=none -fno-pic -fno-pie -fno-asynchronous-unwind-tables
  -fno-unwind-tables -fno-exceptions -fno-omit-frame-pointer -mgeneral-regs-only
  -msoft-float -mno-sse -mno-sse2 -mno-mmx -mno-80387 -ftls-model=local-exec
  -fno-strict-aliasing`
- `-ftls-model=local-exec` is the only TLS model. Dynamic TLS models are
  unsupported.

## 2. Linking and toolchains

- Toolchains used: gcc 12.2.0 with GNU ld 2.40, and `zig cc` 0.17.0 (clang
  22.1.8 with LLD). Both build the same sources and pass the same static audit
  (**STATIC**, `run.py --cc gcc audit` and `--cc clang audit`).
- Linker script: `experiments/c-runtime/link/arena-user.ld`. It has `KEEP` on the
  TLS sections. Without `KEEP`, GNU ld drops an empty `.tbss` and the
  `__arena_tls_memsz` symbol is wrong (recorded during C1).
- TLS symbols exported by the linker script: `__arena_tls_image`,
  `__arena_tls_filesz`, `__arena_tls_memsz`, `__arena_tls_align`.
- The ARST loader ignores `PT_TLS` (F3). The C runtime installs the TLS block
  itself (**GUEST**, T8: four threads keep private values).
- No dynamic linker, no `dlopen`, no shared libraries, no relocation (**BLOCKED**
  for SDL3's optional backends, §10).

Sizes of C1 images (**STATIC**, built on the final tree):

| Image | ELF size | Instructions | TLS filesz / memsz / align |
|---|---|---|---|
| `c-native` gcc | 47 592 B | 8 883 | 4 / 32 / 16 |
| `c-native` clang | 51 592 B | 7 770 | 4 / 32 / 16 |
| `crt-probe` gcc | (see final report) | 5 386 | 8 / 48 / 16 |
| `crt-probe` clang | (see final report) | 5 523 | 8 / 48 / 16 |

## 3. Startup model

1. The loader maps the image from a signed APB1 bundle (`tools/apb1_format.py`,
   Ed25519 with domain prefix; the test fixture is RFC 8032 and test-only).
2. The desktop launches it headless with manifest flags. For stdio the manifest
   must include `STANDARD_STREAMS` (8). For sync, `NATIVE_SYNC` (16) is
   required. C1's probe uses 28 = HEADLESS | STANDARD_STREAMS | NATIVE_SYNC.
3. The ARST v2 block sits in slot 0: one 4 KiB page, header 128 B, roles
   (CWD 1, Stdin 2, Stdout 3, Stderr 4, Other 5, StreamSet 6, StreamWake 7,
   SyncDomain 8). The runtime maps the page, validates offsets, role uniqueness,
   strings, reserved zeros, and the live table, unmaps it, and destroys slot 0.
   Failure gives `ARENA_E_STARTUP`, and `main` is never reached (**GUEST** for
   the valid path; refusal paths are covered by Phase-13 tests, not re-run in
   C1).
4. Main stack: 64 pages = 256 KiB, switched to by `arena_switch_stack`.

## 4. Allocation

- Public API: `arena_malloc`, `arena_calloc`, `arena_realloc`, `arena_free`,
  `arena_heap_stats` (`include/arena/rt.h`). Names carry the `arena_` prefix (§9).
- One lazily reserved 16 MiB region per process (4096 pages). Committed in
  64 KiB chunks. Power-of-two blocks from 32 B to 16 MiB. 16-byte headers.
  Largest payload is `16 MiB - 16 B`. Oversize requests are refused, not
  truncated (**HOST-ONLY**, `test_allocator_contract`).
- Ownership is validated before any metadata read. Interior, freed, and foreign
  pointers are refused. Size arithmetic is overflow-checked. Coalescing is
  bounded (**HOST-ONLY**, `test_allocator_hardening`, stress with 200 000 ops).
- Thread safety: one spinlock. A contended acquire yields through
  `arena_backoff()` (`SYS_THREAD_YIELD` in the guest). The lock is held across
  the VM commit and reserve calls, which are assumed not to block (a review
  point). Concurrent use is **GUEST** (T5 and T9).
- **Not provable here:** kernel-side memory safety. Host ASan/UBSan results are
  not evidence about the kernel or the guest VM.
- Refused VM commit leaves the allocator unchanged: **HOST-ONLY** only. There is
  no guest test for it (C1 gap, see the final report).
- Not exposed: the kernel's `ScalableHeap` (F5). The C heap is separate.

Memory limits (**STATIC**, from the kernel constants and the C runtime):

| Limit | Value |
|---|---|
| C heap | 16 MiB per process, fixed |
| Main stack | 256 KiB |
| Thread stack | 56 KiB usable per thread (16-page region, guard page 1) |
| VM regions per process | 8 |
| Pages per region | 4 096 |
| Committed pages per process | 8 192 (32 MiB) |
| Pages per VM operation | 64 |
| Dynamic image cap | 256 KiB / 128 pages (F10, from `01-architecture-audit.md`) |
| Segments per image | 8 |
| Capability slots per process | 128 |

## 5. Supported libc subset

Public names are the `arena_` family. Standard names are exported only where the
compiler needs them.

- **Exported under standard names (guest build):** `memcpy`, `memmove`,
  `memset`, `memcmp`, `strlen`, `strcmp` (aliases to `arena_*`; `string.c`
  `#ifndef ARENA_HOSTED`).
- **Strings:** `arena_memcpy`, `arena_memmove`, `arena_memset`, `arena_memcmp`,
  `arena_strlen`, `arena_strnlen`, `arena_strcmp`, `arena_strncmp`.
- **Formatting (`stdio.c`):** `arena_snprintf`, `arena_vsnprintf`,
  `arena_printf`, `arena_vprintf`. Conversions: `%%`, `%c`, `%s`, `%d`, `%i`,
  `%u`, `%x`, `%X`, `%o`, `%p`. Flags `-` and `0`, decimal width, `.` precision
  for strings, length modifiers `hh h l ll z j t`. Unsupported conversions are
  echoed, not dropped. **No floating-point conversion.**
- **Errors:** write errors from `arena_printf` are returned as negative
  `ARENA_E_*` values (commit `2e8fae6`, HOST-ONLY).
- **Not present:** `malloc`, `free`, `calloc`, `realloc`, `printf`, `snprintf`,
  `exit`, `abort`, `errno`, `FILE`/`stdio` streams, `strtol`/`strtod`, `qsort`,
  locale, `math.h`, `setjmp`, signals, `time.h`, `pthread`, and POSIX file or
  socket APIs. An SDL3 port needs a shim that maps these names onto `arena_*`,
  or a change to the SDL3 sources. Both are work for Arena 1. None is in C1.

## 6. Standard streams

- Discovery: `arena_stdio_init()`. It returns `ARENA_E_NO_STREAMS` when no grant
  exists, and the app exits 58 in that case (**GUEST**, stream-less package).
- File descriptors 0, 1, and 2 map to the granted roles only. FD numbers are
  local and grant nothing. Writing to fd 0 is refused (T2 check
  `stdin is not writable`, **GUEST**). Fd 7 is refused as out of range.
- Writes: `arena_stream_write(fd, buf, len)` loops until all bytes are accepted
  or an error is returned. `arena_stream_write_some` returns the short count.
  `ARENA_E_WOULD_BLOCK` means the ring is full. The writer then waits on the
  granted notification (`SYS_WAIT`), then retries. `ARENA_E_BROKEN_PIPE` means
  the desktop closed its read side.
- Reads: `arena_stream_read` and `arena_stream_read_some` on fd 0. A return of 0
  means EOF, which happens only after the writer closed and the ring is empty.
- Ring: 768 bytes per channel, 3 channels, ADR-0104 layout. Counters are `u32`
  and index with `counter % 768`. **BLOCKED defect:** the mapping jumps at the
  2^32 wrap (`2^32 mod 768 = 256`). Unread bytes can be overwritten after about
  4 GiB on one channel. Pinned by a host test; ADR-0111 proposes the fix. Do not
  advertise streams as safe for unbounded output until ADR-0111 is decided.
- Backpressure: **GUEST**. A 2 KiB transfer through a 768-byte ring completes
  (`backpressure: 2048 bytes`).
- No `SYS_DEBUG_WRITE` on any supported stdio path. `arena_write` routes to
  streams unless `arena_legacy_serial_enable()` is called, which only the legacy
  boot probe does (**STATIC**).
- Console note: the desktop relays stdout and stderr in chunks, and kernel log
  lines can interleave with them. The C1 harness therefore judges app lines on a
  view with log fragments removed (**GUEST**). Arena 1's own tooling should not
  rely on line integrity of the combined serial console.

## 7. Threads and synchronization

| API | Notes |
|---|---|
| `arena_thread_create(t, fn, arg)` | `int (*fn)(void *)`. Returns `ARENA_STATUS_QUOTA` for the fifth user thread (**GUEST**, T9). |
| `arena_thread_join(t, &status)` | Writes the 8-byte status. Second join or join after detach gives `ARENA_E_STALE` (**GUEST**, T9). |
| `arena_thread_detach(t)` | The worker is reaped by the scheduler when it exits. |
| `arena_thread_count()` | Live user threads. Main waits until it reads 1 before returning. |
| `arena_yield()` | `SYS_THREAD_YIELD`. |
| `arena_mutex_init/lock/trylock/unlock/destroy/check` | Fast path is local. The kernel is consulted only when parking. |
| `arena_condvar_init/wait/wait_timeout/notify_one/notify_all/destroy` | **GUEST**, T9: notify-one, notify-all, timed wait. |
| `arena_once_init/call/destroy` | One-time initialization. **GUEST**, T9. |
| `arena_sync_init()` | Needs a sync domain grant (`SyncDomain` role; `NATIVE_SYNC` manifest flag). |

- Limits: 4 user threads per process, initial thread excluded (kernel
  `MAX_USER_THREADS_PER_PROCESS`). 64 threads system-wide. Sync domains 64 system-
  wide, 32 keys per domain, 2 048 keys total. Timeout maximum 86 400 s
  (`MAX_TIMEOUT_US`).
- Stale and wrong-generation tokens give `ARENA_E_STALE` or `BAD_ARG`. Key
  exhaustion gives `QUOTA`. Destroy with a parked waiter gives `BUSY`.
- Not implemented: thread-local destructors, cancellation, signals, pthread-
  style attributes, and Linux futex (not used anywhere).
- Per-thread TLS: each thread gets its own block (**GUEST**, T8).
- Guest-tested concurrency: T9 runs real worker threads with a mutex-protected
  counter and condvar wakeups. It is **not** a soak test.

## 8. Floating point and SIMD

- **Not supported.** The kernel does not save or restore XMM, x87, or MXCSR state,
  and does not enable CR4.OSFXSR (**STATIC**; `C1-FPSIMD-BOUNDARY.md`).
- C images are compiled with `-mno-sse` and the rest of §1, and the static audit
  fails any image with an XMM, YMM, ZMM, MM, or x87 operand (**STATIC**). The C
  images contain no such instructions.
- **SDL3 uses floating point heavily.** Porting it requires the FP decision
  (Luna, after Phase 14): keep the no-FP contract, or enable FP with per-thread
  state through a kernel ADR (`C1-FPSIMD-BOUNDARY.md` §4). **BLOCKED.**
- A scalar `double` program would fault or corrupt state. C1 does not claim FP
  works, and no FP program was run.
- Firmware-left CR4.OSFXSR, CR4.OSXSAVE, CR0.EM, and CR0.TS are not checked
  (**OPEN**).

## 9. Missing symbols and naming (summary)

Use `arena_*` names, plus the six standard aliases in §5. Anything else an SDL3
source file calls must be shimmed or removed. That includes the libc set in §5,
the dynamic loader calls (`dlopen`, `dlsym`), `pthread_*`, `clock_gettime`,
`nanosleep`, `getenv`, `setlocale`, and file or socket calls. Arena 1 owns the
shim decision. C1 did not implement any of these.

## 10. Dynamic loading and optional backends

SDL's Linux documentation says SDL links only glibc by default, and that other
features (X11, ALSA, D-Bus, and so on) are enabled dynamically at runtime if
present (`github.com/libsdl-org/SDL`, `docs/README-linux.md`, read 2026-10-08).
That needs a dlopen-style facility, which ArenaOS does not have. A static build
with no optional backends is the only shape that fits, and it must be checked
against SDL's source before Arena 1 commits to it. **BLOCKED** until that build
is defined and shown to link with no dynamic loading.

SDL's `SDL_Init` documentation (wiki.libsdl.org, secondary source) says file I/O
and threading initialize by default, and that video initialization must run on
the main thread. Both points need checking against the source in Arena 1's build.

## 11. Timing

- `arena_clock_us()` returns monotonic microseconds from `SYS_CLOCK_NOW`
  (**GUEST**, T6: non-decreasing).
- `arena_busy_sleep_us(n)` is a coarse sleep. **GUEST**: ten logged runs asked
  for 20 000 µs and slept between 121 185 and 181 398 µs. The timer is coarse on
  this VM. Frame pacing and audio timing are **BLOCKED** until a finer timer API
  is designed. The kernel has per-process timers (limit 4), but C does not
  expose them.

## 12. Required capabilities (what an SDL3 app would need from the launcher)

| Need | Capability or grant | C1 status |
|---|---|---|
| Launch and identity | APB1 signed package via the protected install path; desktop launch | GUEST |
| stdout, stderr, stdin | StreamSet (SharedRegion READ\|WRITE) and StreamWake (Notification WRITE) | GUEST |
| Blocking waits | Notification with WAIT | GUEST (used by backpressure) |
| Sync primitives | SyncDomain grant with `NATIVE_SYNC` | GUEST |
| Threads | `SYS_THREAD_*` within the 4-thread quota | GUEST |
| Heap | `SYS_VM_*` within the process VM quota | GUEST |
| Video, audio, input, GPU | **Not designed.** Arena 1's graphics and audio capabilities | NOT IN C1 |
| Files and paths | None granted. Paths and PIDs grant no authority | Not applicable |

No new ambient authority was added. C1 uses only grants the desktop already gives.

## 13. Missing interfaces and blockers (exact)

| # | Blocker | Owner | Needed for SDL3 |
|---|---|---|---|
| B1 | FP/SIMD: no state save, no OSFXSR; SDL3 requires FP | Luna (after Phase 14) | Yes |
| B2 | Dynamic loading and optional backends; static no-optional-backend build undefined | Arena 1 and C | Yes |
| B3 | Stream ring u32 wrap (ADR-0111) | M and C | Long-running output only |
| B4 | libc name shim (`malloc`, `snprintf`, `pthread_*`, `clock_gettime`, file I/O) | Arena 1 | Yes |
| B5 | Video and audio backends, GPU | Arena 1 | Yes |
| B6 | Fine-grained timer for frame pacing | Core OS and C | Yes, for games |
| B7 | `SYS_DEBUG_WRITE` has no capability check (ADR-0110 D3) | Core OS | Not for SDL3; C avoids it |
| B8 | PT_TLS ignored by the loader | Luna loader owners | Not blocking; the runtime installs TLS |
| B9 | No guest-side VM commit-failure test | C | Not blocking; host-tested |

Nothing here was resolved by guest-unverified code.

## 14. Commands (to reproduce the C1 evidence)

```
cd experiments/c-runtime
python3 run.py build
python3 run.py host                       # HOST-ONLY
python3 run.py --cc gcc audit             # STATIC
python3 run.py --cc clang audit           # STATIC
python3 guest_native.py --cc clang --runs 5   # GUEST
python3 guest_native.py --cc gcc --runs 5     # GUEST
```

Arena 1 should take the source from the C1 branch commit recorded in the final
report, and must not port from the working tree.
