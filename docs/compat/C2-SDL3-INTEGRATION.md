# C2 to SDL3 (Arena 1): integration contract

Status: EXPERIMENTAL migration contract. SDL3 is **not** built or tested by C2.
Nothing here is an SDL3 compatibility claim. Arena 1's branch
(`arena/13f9221b-arenaos`) was read only and is not modified.

The point of this document: Arena 1's G2 SDL3 path carries its own `_start`, its own
heap, and its own slot constants. C2 provides one shared runtime that Arena 1 can
link instead. This file says exactly what that means.

## 1. Paths (repository `experiments/c-runtime/`)

Headers (add `-I include -I include/libc`):

| Header | Purpose |
|---|---|
| `include/arena/rt.h` | Runtime entry points, startup record (`arena_startup_t`), streams, threads, sync, errors |
| `include/arena/abi.h` | ABI constants (ARST v2, capability kinds and rights, role numbers). 92 constants checked by `run.py abi` |
| `include/arena/string.h` | Runtime string helpers |
| `include/arena/version.h` | Runtime version and profile identity |
| `include/arena/profile.h` | Profile flags (static, no-FP profile) |
| `include/libc/*.h` | Standard C headers for the subset (`stdio.h stdlib.h string.h errno.h time.h threads.h assert.h`) |

Static library and startup object (built per compiler):

| Artifact | Path |
|---|---|
| Startup object (linked first) | `build/guest-gcc/crt0.o`, `build/guest-clang/crt0.o` |
| Runtime archive | `build/guest-gcc/lib/libarena_c.a`, `build/guest-clang/lib/libarena_c.a` |
| Linker script | `link/arena-user.ld` |

Build the artifacts with: `python3 run.py --cc gcc build` and `python3 run.py --cc clang build`
(`clang` uses the Zig-bundled clang, `ARENA_ZIG` must point at the `zig` binary).

Do not copy runtime sources into an application. Link the archive.

## 2. Link commands

Flags are defined in `experiments/c-runtime/run.py` (`common_flags`, `LINK_FLAGS`).
These are the link recipe for one app `app.c` (with no extra objects) to `app-<cc>.elf`:

```
# GCC
gcc -std=gnu11 -O2 -g0 -Wall -Wextra -Werror -ffreestanding -fno-builtin \
    -fno-stack-protector -fcf-protection=none -fno-pic -fno-pie \
    -fno-asynchronous-unwind-tables -fno-unwind-tables -fno-exceptions -fno-omit-frame-pointer \
    -mgeneral-regs-only -msoft-float -mno-sse -mno-sse2 -mno-mmx -mno-80387 \
    -ftls-model=local-exec -fno-strict-aliasing \
    -I include -I include/libc -I src \
    -nostdlib -static -no-pie -Wl,--no-dynamic-linker -Wl,-z,noexecstack \
    -Wl,--build-id=none -Wl,-z,max-page-size=4096 \
    -Wl,-T,link/arena-user.ld \
    build/guest-gcc/crt0.o app.o build/guest-gcc/lib/libarena_c.a \
    -o app-gcc.elf

# Clang (Zig-bundled, freestanding target)
$ARENA_ZIG cc -target x86_64-freestanding-none <same flags as above> \
    -Wl,-T,link/arena-user.ld \
    build/guest-clang/crt0.o app.o build/guest-clang/lib/libarena_c.a \
    -o app-clang.elf
```

Ordering matters: `crt0.o` first, application objects, then the archive.
The archive is linked last so its symbols resolve application references.

Archive members: the application references only the symbols it uses. Ordinary apps
do **not** pull `SYS_DEBUG_WRITE` (the legacy serial sink is weak and gated behind
`arena_legacy_serial_enable()`).

Audit after every link (both mandatory):

```
python3 experiments/c-runtime/c2_isa_audit.py app-gcc.elf      # FP/SIMD must be zero
python3 experiments/c-runtime/c2_report.py                      # symbol coverage report
```

`c2_isa_audit.py` exits nonzero on any vector-register or x87 instruction inside a
function. `python3 c2_isa_audit.py --self-test` checks it with a negative control (SSE math
must FAIL) and a positive control (no-FP profile, must PASS). An earlier raw scan of Arena 1's
G2 `SDL3App` ELF found 79 vector-operand instructions (for example `movaps %xmm3` inside
`SDL_snprintf`). That scratch ELF is no longer available, so treat it as a prior observation
to re-confirm from Arena 1's own build.

## 3. Supported libc symbols

The authoritative list is `experiments/c-runtime/compat/c2-compat-report.json`
(110 declared, 110 defined in both archives). Per-symbol status and evidence are in
`docs/compat/C2-LIBC-COVERAGE.md`. Summary for SDL3-relevant areas:

- Memory: `malloc calloc realloc free aligned_alloc` (C1 allocator; max alignment 64 KiB).
- Strings and memory: the full `string.h` set listed in the coverage doc.
- Formatting: `printf snprintf sprintf vsnprintf vsprintf vprintf fprintf vfprintf puts putchar fputs`.
  **Floating-point conversions are refused** (`ENOTSUP`). SDL's float formatting does
  not work through this path.
- Integer conversion and math: `strtol` family, `atoi atol atoll labs llabs div ldiv lldiv abs`.
- `qsort`, `bsearch`, `rand`, `srand`, `getenv` (environment comes from the verified record).
- Time: `clock_gettime(CLOCK_MONOTONIC)`, `nanosleep`. `time()` and `clock()` are refused.
- Process: `exit`, `atexit` (32 handlers), `_Exit` and `abort` are defined but untested.

## 4. Supported thread APIs (C11 subset, not pthreads)

- Supported: `thrd_create`, `thrd_join`, `thrd_yield`, `mtx_init` (plain only),
  `mtx_lock`, `mtx_unlock`, `mtx_trylock`, `cnd_init`, `cnd_timedwait`, `call_once`.
- Implemented but untested: `thrd_detach`, `thrd_exit`, `thrd_current`, `thrd_equal`,
  `thrd_sleep`, `mtx_destroy`, `cnd_signal`, `cnd_broadcast`, `cnd_wait`, `cnd_destroy`.
- **Refused**: recursive mutexes, `mtx_timedlock`, all `tss_*` keys (`tss_get` returns `NULL`).
- Not provided: pthreads, `pthread_*`, POSIX signals.

SDL3 requires thread-local state for some subsystems and thread-specific keys for others.
Arena 1 must **not** map `tss_*` onto a global. Use `_Thread_local` (supported and isolated)
instead, or refuse the feature.

## 5. Startup requirements

- Entry: `crt0.o` runs `arena_startup_gate()` before any allocation, then the stack and
  TLS setup, then `.init_array`, then `main`, then `arena_libc_exit`.
- Gate outcomes (from `src/crt0.c` and `src/startup.c`):
  - Slot 0 empty (legacy image, no record): the gate returns 0. `main` runs with
    `present == 0`, `argc == 0`, `argv == NULL`. Any stdio call then returns
    `ARENA_E_NO_STREAMS`. An app that needs a record must check `present` and refuse.
  - Slot 0 present but invalid (wrong kind, rights, count, or layout): the process
    exits with status **122** before `main`.
  - Stack setup failure: status 120. TLS setup failure: status 121.
- A native app that requires a verified record must check `arena_startup()->present`
  and refuse to run when it is 0. C2's negative build does not run.
- `argv` and `envp` come **only** from the verified record.
- Flags: `NATIVE_SYNC` (16) must be set to use synchronization domains.
  `STANDARD_STREAMS` (8) must be set to use stdio.

## 6. Required capabilities (from the record, not from slot numbers)

| Need | Required descriptor | Missing behavior |
|---|---|---|
| stdio (`stdin/stdout/stderr`) | `StandardStreamSet` role (6) and `StreamWake` role (7) | `ARENA_E_NO_STREAMS` (`58` in the C2 negative build) |
| synchronization (mutex, condvar, once) | `SyncDomain` role (8), kind 16 | refused with `ARENA_E_NO_CAP` |
| private wait | `Notification`, rights `WRITE` | `ARENA_E_NO_WAITER` |
| graphics (surface, endpoint, clock) | **none by role** | see B-GFX-1 |

Graphical descriptors (BadgedEndpoint, SharedRegion, Notification) carry role **Other**.
The runtime can count them by kind with `arena_startup_count_kind()`, but cannot tell
which one is the surface. Arena 1 must not index them by slot number. See
`C2-FINAL-REPORT.md` (B-GFX-1).

## 7. Resource limits (kernel and runtime)

- Threads: 4 user threads per process (kernel); last-thread-exit rule.
- Heap: 16 MiB (`HEAP_PAGES` 4096), 64 KiB chunks, 16-byte default alignment. One heap per process; operations are serialized by an internal lock.
- Aligned allocation: 16 to 65536 bytes. Larger is refused.
- atexit handlers: 32.
- Streams: ring capacity 768 bytes per direction.
- Capability slots: 128 per process. Descriptors per startup record: 7 (`ARENA_ARST_CAPABILITY_MAX`).
- VM: 8 regions per process, 4096 pages per region, 8192 committed pages per process.
- Timers: 4 per process. Sync: 64 domains, 32 keys per domain, 2048 keys total.
- Sync timeout maximum: 86 400 000 000 µs.

## 8. Unsupported functionality (C2)

- File system of any kind. `fopen`, `freopen`, `remove`, `rename` are refused.
  Standard streams are not a filesystem. A broker interface for files is a dependency
  C2 does not provide.
- Floating-point formatting and conversion (`%f %e %g %a`, `long double`).
- Any SIMD. Any x87. Kernel FP/SIMD state is not implemented.
- Dynamic loading, `dlopen`, plugin loading. SDL3 optional backends are dlopen-based.
- Locale beyond C. `setjmp`/`longjmp` (not declared). Signals. `fork`/`exec`.
- Wall-clock time (`time`, `CLOCK_REALTIME`). CPU time (`clock`).
- pthreads, TSS keys, recursive mutexes, timed mutex locks.

## 9. Remaining FP/SIMD blockers for SDL3

1. **SDL3 is written with floating point and SIMD by default.** Arena 1's vendored
   SDL3 path has 79 vector-operand instructions in a static audit (`SDL3App`, read-only
   inspection of `9311dd0`, prior observation). Examples include `SDL_snprintf`. The C2 ISA audit would reject such code.
2. **No soft-float runtime.** `-msoft-float` makes the compiler emit calls such as
   `__adddf3` and `__floatsidf`. Those come from `libgcc`. C2 does not link `libgcc`
   (`-nostdlib`). Any SDL code that uses `float` or `double` therefore fails at link
   time. That is the intended fail-closed behavior, but it means **SDL3 cannot link
   as-is**. Providing a verified soft-float helper library is a separate, unapproved
   piece of work.
3. **Execution contract.** The kernel saves no FP or SIMD state (`switch_context` saves none;
   CR4.OSFXSR is not set). Any SSE or x87 instruction in a process is a hazard. This is
   not a C2 claim that the kernel is safe for SSE.
4. **Blender** (for context): its documented Linux baseline is SSE4.2. It is outside the
   no-FP profile.

## 10. Concrete steps for Arena 1 to replace its duplicated runtime

1. Remove SDL3's custom `_start` and custom heap from the ArenaOS build. Link
   `build/<cc>/crt0.o`, `libarena_c.a`, and `link/arena-user.ld` as in section 2.
2. Replace every `malloc`/`free`/`realloc` call site with the C2 libc. Do not keep a
   second allocator.
3. Replace stdio and exit with the C2 stdio and exit. Do not write to a debug syscall.
4. Replace slot-number capability lookups (`SERVICE_ENDPOINT=1`, `SURFACE=2`,
   `CLOCK=3`, `DIAGNOSTICS=4`) with lookups over `arena_startup()->caps[]` by role and
   kind. Refuse the app if the match is absent or ambiguous. Do not guess.
5. Keep the graphics protocol (surface layout, input, ADSK-v1 on `SYS_IPC_CALL`) under
   Arena 1's own specification. C2 does not define it.
6. Build SDL3 with no SIMD, no optional backends, and no dynamic loading. Use
   `-mgeneral-regs-only -msoft-float -mno-sse -mno-sse2 -mno-mmx -mno-80387`. Do not
   enable `-msse*`.
7. Gate the output with `c2_isa_audit.py` (must PASS) and `c2_report.py` (every
   referenced libc symbol must be `defined` in both compilers).
8. Install through the signed APB1 path and launch from All Applications, following
   `experiments/c-runtime/guest_c2.py` for the install and judging pattern. Use an exact
   verdict, not a substring match.
9. Do not add a privileged launch path, a fake capability, or a debug-syscall shortcut.

## 11. What C2 does not give Arena 1

- No SDL3 build or port.
- No graphics protocol, no compositor integration, no display or input testing.
- No verified graphical startup (B-GFX-1 blocks identifying a surface by contract).
