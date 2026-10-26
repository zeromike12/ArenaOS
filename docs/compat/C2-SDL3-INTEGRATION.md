# C2 to SDL3 (Arena 1): integration contract

Status: EXPERIMENTAL migration contract. SDL3 is **not** built or tested by C2.
Nothing here is an SDL3 compatibility claim. Arena 1's branch
(`arena/13f9221b-arenaos`) was read only and is not modified.

> **Phase 16 update (2026-10-26):** streams are ASTR v2 (1,024-byte
> rings; v1 pages refused), live focused stdin and Notification
> ambiguity refusal have canonical GCC/Clang guest receipts, and B1 is
> resolved. The graphical surface/compositor identification blocker
> (B-GFX-1) and all unsupported-API refusals in this contract remain
> unchanged. Current canonical receipts are in
> `docs/phase16/receipts/canonical-2026-10-26/`; see also
> `docs/phase16/ARENA1-HANDOFF.md`. Phase 16 is not accepted by this
> note and no Phase 17/SDL3 work is started here.

The point of this document: Arena 1's G2 SDL3 path carries its own `_start`, its own
heap, and its own slot constants. C2 provides one shared runtime that Arena 1 can
link instead. This file says exactly what that means and which parts are verified.

Evidence labels used below: **guest** (asserted by an exact-PASS QEMU run),
**host** (ASan+UBSan host test), **static** (build or audit result), **refused**
(deliberate error), **untested** (no test reference), **blocked** (cannot be done
under the current contract).

Status labels (C2.1, used in section 0 and throughout). Each label needs the evidence named:

| Label | Meaning | Evidence required |
|---|---|---|
| **available/verified** | Implemented and exercised by a host test against the stated reference | host test name |
| **build-only** | Compiles and links with both compilers and appears in both archives. No test reference | `c2_report.py` defined-in-both |
| **guest-verified** | Exercised by an App A or App B group in an exact-PASS QEMU run | run receipt at a named commit |
| **unsupported** | Refused by design: returns an error or NULL with `errno`, never a fake success | refusal test or code path |
| **blocked** | Cannot be done under the current contract. Needs an architecture decision | blocker ID (see `C2-FINAL-REPORT.md`, `C2.1-B-GFX-1-HANDOFF.md`) |

Evidence for this document is C2.1 at commit `c8887a1` or later. The verification receipt
(`docs/compat/C2.1-VERIFICATION.md`) names the commit each result came from.

## 0. Shared-runtime function status (C2.1)

No symbols were added or removed in C2.1. The table covers the surface Arena 1 would use.

| Area | Functions | Status | Evidence |
|---|---|---|---|
| Startup gate and record | `arena_startup()`, `arena_startup_count_kind`, `arena_startup_pick_notification` | available/verified (count and notification selection); guest-verified (gate, App A G8) | host (`test_notification_selection`), guest |
| Allocation | `malloc calloc realloc free`, `aligned_alloc` (power-of-two alignment, size a multiple of alignment) | guest-verified (G3); aligned contract host-verified | host, guest |
| Strings and memory | `string.h` set | available/verified | host (glibc differential) |
| Formatting | `printf snprintf sprintf vsnprintf vsprintf vprintf fprintf vfprintf puts putchar fputs` | available/verified (integer conversions); unsupported (floating point) | host, guest (`printf snprintf puts putchar fputs`, G1) |
| Streams | `fwrite` (complete elements), `fputc putc`, `fgets` (NULL on read error), `ungetc`, `setvbuf` (`_IONBF` only) | available/verified; `setvbuf` `_IOFBF`/`_IOLBF` **unsupported** (EINVAL) | host (C2.1.3 tests), guest (G1, G7) |
| Input | `getchar`, `fgetc`, `getc`, `fread` | available/verified (pushback, errors); exact stdio live reads not individually guest-tested, but the underlying focused live-stdin stream path is guest-verified in Phase 16 | host, guest (G2 classification), Phase 16 `guest_stdin.py` |
| Integers | `strtol` family, `atoi atol atoll abs labs llabs div ldiv lldiv` | available/verified; `abs`/`div` at MIN defined (no UB) | host (glibc differential, boundary tests) |
| Time | `clock_gettime(CLOCK_MONOTONIC)`, `nanosleep` | guest-verified (G4) | guest |
| Time | `time`, `clock`, `CLOCK_REALTIME` | unsupported (ENOSYS) | host (`test_time_refusals`) |
| Process exit | `exit`, `atexit` (32 slots), `_Exit`, `abort`, `assert` | available/verified; `atexit` guest-verified (G9) | host (forked children), guest |
| Process exit | `_exit` | **unsupported**: not defined. Use `exit` or `_Exit`. The Arena 1 G2.1 handoff lists it as needed; C2.1 adds no symbols | static (`nm`, source search) |
| Threads | `thrd_create join detach current equal sleep yield`, `thrd_exit` | guest-verified (G5, G6) | guest |
| Mutex | `mtx_init` (`mtx_plain` only), `mtx_lock unlock trylock destroy` | guest-verified (G5, G6) | guest |
| Mutex | recursive mutex, `mtx_timed`, `mtx_timedlock` | unsupported (`thrd_error`) | guest (G6) |
| Condition | `cnd_init wait signal broadcast destroy`, `cnd_timedwait` | guest-verified (G5, timeout). G6 verifies: over-bound wait refused with the mutex still held; `tv_nsec` = 10^9 refused; expired deadline returns `thrd_timedout` with the mutex reacquired. **Not guest-verified:** the seconds-to-microseconds overflow refusal and the negative-seconds refusal (code-inspected only) | guest |
| Once | `call_once` | guest-verified (G5) | guest |
| Thread-specific | `tss_create`, `tss_set`, `tss_get` | unsupported (`thrd_error`, or NULL from `tss_get`); `tss_delete` is a no-op because no key can exist | guest (G6) |
| Floating point and SIMD | any | **blocked** (no FP state in the kernel; no soft-float path; see section 9) | static (ISA audit) |
| Filesystem | `fopen freopen remove rename` | unsupported (ENOSYS) | host |

Deadline base for `cnd_timedwait`: the runtime interprets `struct timespec` deadlines on
the **monotonic** clock, because no wall-clock grant exists. C11 specifies absolute
`TIME_UTC` time. A caller that builds deadlines from wall time is not supported. Use
relative deadlines from `clock_gettime(CLOCK_MONOTONIC)`.

## 1. Paths (repository `experiments/c-runtime/`)

Headers (add `-I include -I include/libc`):

| Header | Purpose |
|---|---|
| `include/arena/rt.h` | Runtime entry points, startup record (`arena_startup_t`), streams, threads, sync, errors |
| `include/arena/abi.h` | ABI constants (ARST v2, capability kinds and rights, role numbers). 92 constants checked by `run.py abi` (static) |
| `include/arena/string.h` | Runtime string helpers |
| `include/arena/version.h` | Runtime version and profile identity |
| `include/arena/profile.h` | Profile flags (static, no-FP profile) |
| `include/libc/*.h` | Standard C headers for the subset: `stdio.h stdlib.h string.h errno.h time.h threads.h assert.h` |

Static library and startup object (built per compiler):

| Artifact | Path |
|---|---|
| Startup object (linked first) | `build/guest-gcc/crt0.o`, `build/guest-clang/crt0.o` |
| Runtime archive | `build/guest-gcc/lib/libarena_c.a`, `build/guest-clang/lib/libarena_c.a` |
| Linker script | `link/arena-user.ld` |

Build the artifacts with `python3 experiments/c-runtime/run.py --cc gcc build` and
`python3 experiments/c-runtime/run.py --cc clang build`. The clang build uses the
Zig-bundled clang; `ARENA_ZIG` must point at the `zig` binary.

Do not copy runtime sources into an application. Link the archive.

## 2. Link commands

Flags are defined in `experiments/c-runtime/run.py` (`common_flags`, `LINK_FLAGS`).
This is the link recipe for one app `app.c` (no extra objects) to `app-<cc>.elf`,
run from `experiments/c-runtime/`:

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

# Clang (Zig-bundled, freestanding target): same flags, different driver
$ARENA_ZIG cc -target x86_64-freestanding-none <same flags as above> \
    -Wl,-T,link/arena-user.ld \
    build/guest-clang/crt0.o app.o build/guest-clang/lib/libarena_c.a \
    -o app-clang.elf
```

Link set (C2.1, unchanged by C2.1 because no symbols were added): `crt0.o`, the application
objects, and `libarena_c.a`. `_exit` is not in the set. Call `exit` or `_Exit`.

Ordering matters: `crt0.o` first, application objects, then the archive. The archive is
linked last so its symbols resolve application references. Ordinary apps do **not** pull
`SYS_DEBUG_WRITE`: the legacy serial sink is in its own object and is gated behind
`arena_legacy_serial_enable()` (guest harness checks this statically).

Audit after every link (both mandatory):

```
python3 experiments/c-runtime/c2_isa_audit.py app-gcc.elf     # FP/SIMD/x87 must be zero
python3 experiments/c-runtime/c2_report.py                    # symbol coverage report
```

**The compile flags alone do not enforce the no-FP profile. The ISA audit does.** C2.8
probe (static, this session): with the flags above, Zig's clang compiled a `volatile double`
multiply to x87 instructions (`fldl`, `fmuls`, `fstpl`) and linked it without error. GCC
instead refuses the same kind of code at compile time (`SSE register return with SSE
disabled`). `c2_isa_audit.py` FAILs the clang ELF and lists 11 x87 instructions. Its `--self-test` now
checks three controls: an SSE negative control (must FAIL), an x87 negative control (must
FAIL with x87 hits), and an integer-only freestanding positive control (must PASS).
Any Arena 1 build must pass the audit, not just compile with these flags.

An earlier raw scan of Arena 1's G2 `SDL3App` ELF found 79 vector-operand instructions. That
scratch ELF is no longer available in this sandbox, so this is a prior observation to re-confirm
from Arena 1's own build.

## 3. Supported libc symbols

The authoritative list is `experiments/c-runtime/compat/c2-compat-report.json`
(110 declared, 110 defined in both archives; static). Per-symbol status and evidence are in
`docs/compat/C2-LIBC-COVERAGE.md`. Summary for SDL3-relevant areas:

- Memory: `malloc calloc realloc free aligned_alloc` (C1 allocator; alignment up to 64 KiB).
  Host and guest (App A G3).
- Strings and memory: the `string.h` set in the coverage doc. Host-tested against glibc.
- Formatting: `printf snprintf sprintf vsnprintf vsprintf vprintf fprintf vfprintf puts putchar fputs`.
  Host-tested; guest-tested for `printf snprintf puts putchar fputs` (App A G1).
  **Floating-point conversions are refused** (`ENOTSUP`). SDL's float formatting does not
  work through this path.
- Integer conversion and math: `strtol` family, `atoi atol atoll labs llabs div ldiv lldiv abs`. Host-tested.
- `qsort`, `bsearch`, `rand`, `srand`, `getenv` (environment comes only from the verified record). Host-tested.
- Streams: `fputc putc fwrite feof ferror clearerr setvbuf perror` (guest, App A G7);
  `fgetc getc fgets fread` refusal paths host-tested; Phase 16 guest-tests
  live stdin at the `arena_stream_read`/`arena_stream_read_some` level,
  not each stdio wrapper individually.
- Time: `clock_gettime(CLOCK_MONOTONIC)` and `nanosleep` (guest, App A G4).
  `time()` and `clock()` are refused with `ENOSYS`; no test reference.
- Process: `exit` and `atexit` (32 handlers, reverse order, each run at most once per process;
  guest G9 and host). C2.1.4: a registration reserves its slot by compare-and-swap and publishes
  it with a release store, so no handler is visible before it is complete. Re-entry from a
  handler on the same thread is a no-op. Another thread that calls the run waits for the run in
  progress. Host tests run the concurrency cases in forked children. `_Exit` and `abort` end
  the process with 7 and 134 (host, forked child). They run no handlers. Not guest-tested.
- `assert` failure prints and calls `abort` (host, forked child, status 134).

## 4. Supported thread APIs (C11 subset, not pthreads)

Evidence is guest (App A G5 and G6) unless noted. The labels follow section 0.

- **Available and guest-verified:** `thrd_create`, `thrd_join`, `thrd_detach`, `thrd_current`,
  `thrd_equal`, `thrd_sleep`, `thrd_exit`. `thrd_yield` is called by G5 but its effect is not
  asserted. `mtx_init` (`mtx_plain` only),
  `mtx_lock`, `mtx_unlock`, `mtx_trylock` (free succeeds; self-held returns `thrd_busy`),
  `mtx_destroy`; `cnd_init`, `cnd_wait`, `cnd_signal`, `cnd_broadcast`, `cnd_destroy`,
  `cnd_timedwait`; `call_once`.
- **C2.1 corrections (static, then guest):**
  - `mtx_init` accepts only `mtx_plain`. Other types return `thrd_error` (no capability is faked).
  - `cnd_timedwait` refuses a wait longer than the kernel bound `ARENA_SYNC_MAX_TIMEOUT_US`
    (86 400 000 000 µs), and a field out of range (`tv_nsec` ≥ 10^9). **Guest-verified (G6):**
    both refusals happen **before** the mutex is released (the mutex is still held afterwards),
    and an expired deadline returns `thrd_timedout` with the mutex reacquired.
    **Code-inspected only (no test):** the refusal of a negative `tv_sec` and of a
    seconds-to-microseconds overflow, and the rule that an expired deadline is submitted as a
    1 µs wait, so zero never reaches the kernel (zero means infinite there).
  - Detached threads: the thread record is reference counted (handle and worker). A detached
    worker is released when it finishes, with no use after free and no double free. A refused
    detach keeps the handle valid (**code-inspected only**: the record is released only after a
    successful detach). Stress (64 create-and-detach cycles, App A G6) checks that
    every worker ran, the live thread count returns to baseline, and the allocator's live blocks
    return to baseline. Termination is observed through the kernel thread count, not a flag.
  - `thrd_exit` ends only the calling worker (App A G6: status 7, application continues).
- **Refused (unsupported, guest-verified):** recursive mutexes (`mtx_init` returns `thrd_error`),
  `mtx_timed` at init, `mtx_timedlock` (`thrd_error`), `tss_create` and `tss_set` (`thrd_error`).
  `tss_get` returns `NULL`, a quiet refusal, not an error code. The C2.1 tss key in tests is
  initialized to 0 and is never a live key.
- **`tss_delete`:** a no-op. No key can exist, because `tss_create` always refuses. It is
  unreachable in practice and has no test reference of its own.
- **Not provided:** pthreads, `pthread_*`, POSIX signals.

SDL3 uses thread-local state and thread-specific keys in parts of its code. Those assumptions
must be checked against the pinned SDL source before porting. Arena 1 must **not** map
`tss_*` onto a global. Use `_Thread_local` (supported and isolated, C1 native group T8) or
refuse the feature.

## 5. Startup requirements

- Entry: `crt0.o` (`src/crt0.c`) runs `arena_startup_gate()` **before any allocation**, then
  stack setup and a switch to the new stack, then TLS setup, then `.init_array`, then `main`,
  then `arena_libc_exit(rc)`.
- Gate outcomes (static, from `src/crt0.c` and `src/startup.c`):
  - **Slot 0 empty** (legacy image, no record): the gate returns 0. `main` runs with
    `argc == 0`, `argv == NULL`. `arena_startup()->present == 0`. Any stdio call then returns
    `ARENA_E_NO_STREAMS`. An app that needs a verified record must check `present` and refuse.
  - **Record present but invalid** (wrong kind, rights, count, or layout): the gate fails and
    the process exits with status **122** before `main`.
  - **Stack setup failure:** status **120**. **TLS setup failure:** status **121**.
- `argv` and `envp` come **only** from the verified record.
- Flags must agree with the descriptors (`src/startup.c`, around lines 248–257; a mismatch in
  either direction fails the gate with 122):
  - `STANDARD_STREAMS` (8) must match the presence of the StreamSet and StreamWake descriptors.
  - `NATIVE_SYNC` (16) must match the presence of a SyncDomain descriptor.

## 6. Required capabilities (from the record, not from slot numbers)

| Need | Required descriptor | Missing behavior |
|---|---|---|
| stdio (`stdin/stdout/stderr`) | StreamSet role (6) and StreamWake role (7), flag `STANDARD_STREAMS` | `ARENA_E_NO_STREAMS` (`58` in the C2 negative build) |
| blocking stream wait | a private read-write Notification (kind 3, role **Other**, rights RW); the runtime selects the **first** such entry | `ARENA_E_NO_WAITER` |
| synchronization (mutex, condvar, once) | SyncDomain role (8), kind 16, flag `NATIVE_SYNC` | `ARENA_E_NO_CAP` |
| graphics (surface, endpoint, clock) | **none by role** | see B-GFX-1 |

Two hazards for Arena 1:

- **Notification selection (C2.1.5, host-verified).** The stream-wait notification is chosen by
  `arena_startup_pick_notification`. A candidate is role **Other**, kind **Notification**,
  rights exactly READ|WRITE. Exactly one candidate is selected. Zero, two or more, or a NULL
  table selects nothing, and a blocking stream wait then returns `ARENA_E_NO_WAITER`. The
  runtime never binds to a guessed notification. Arena 1 must still keep the stream-wait
  notification unambiguous in its own record, or it gets no blocking wait.
- **Graphical descriptors** (BadgedEndpoint, SharedRegion, Notification) carry role **Other**.
  The runtime can count them by kind with `arena_startup_count_kind()`, but cannot tell which
  one is the surface. Arena 1 must not index them by slot number. Status: **blocked** (B-GFX-1).
  Options and a recommendation are in `docs/compat/C2.1-B-GFX-1-HANDOFF.md`.

## 7. Resource limits (kernel and runtime)

- Threads: 4 user threads per process (kernel). A create is refused while four are live; the
  detached stress test waits for a worker to exit before creating the next. Process status is
  the last thread to exit.
- Heap: 16 MiB (`HEAP_PAGES` 4096), 64 KiB chunks, 16-byte default alignment. One heap per
  process. All allocator state is serialized by one spinlock (static; host-tested).
- Aligned allocation: alignments up to 16 use the normal path. 32 to 65536 use validated
  aligned blocks. Larger, or non-power-of-two, is refused with NULL and `errno`.
- atexit handlers: 32 (reserved by CAS, published by release store; overflow refused).
- Streams: ASTR v2 ring capacity 1,024 bytes per channel (three channels
  in one page); ASTR-v1 (768-byte) pages are refused.
- Capability slots: 128 per process. Descriptors per startup record: 7 (`ARENA_ARST_CAPABILITY_MAX`).
- VM: 8 regions per process, 4096 pages per region, 8192 committed pages per process.
- Timers: 4 per process. Sync: 64 domains, 32 keys per domain, 2048 keys total.
- Sync timeout maximum: 86 400 000 000 µs (longer waits are refused with `thrd_error`).

## 8. Unsupported functionality (C2)

- File system of any kind. `fopen`, `freopen`, `remove`, `rename` are refused (`ENOSYS`).
  Standard streams are not a filesystem. A broker interface for files is a dependency C2
  does not provide.
- Floating-point formatting and conversion (`%f %e %g %a`, `long double`), and all
  floating-point arithmetic (see section 9).
- Any SIMD. Any x87. Kernel FP/SIMD state is not implemented.
- Dynamic loading, `dlopen`, plugin loading. SDL3 optional backends are dlopen-based.
- Locale beyond C. `setjmp`/`longjmp` (`setjmp.h` does not exist). Signals. `fork`/`exec`.
- Wall-clock time (`time`, `CLOCK_REALTIME`). CPU time (`clock`).
- pthreads, TSS keys, recursive mutexes, timed mutex locks.

## 9. Remaining FP/SIMD blockers for SDL3

1. **SDL3 is written with floating point and SIMD by default.** Arena 1's vendored SDL3 path had
   79 vector-operand instructions in a prior static scan (`SDL3App`, read-only inspection of
   `9311dd0`). That ELF is not available now. The C2 ISA audit would reject such code.
2. **No soft-float runtime, and no way to build floating-point code under this profile.**
   C2 measured the two compilers (static, C2.8):
   - GCC with `-mno-sse` refuses SSE-requiring double code at compile time.
   - Zig's clang compiles the same double code to **x87** instructions, which the audit rejects.
   Neither produces a soft-float build. C2 links no `libgcc` (`-nostdlib`). A soft-float helper
   library would be separate, unapproved work. Until it exists, SDL code that uses `float` or
   `double` **cannot pass the audit**.
3. **Execution contract.** The kernel saves no FP or SIMD state: `switch_context`
   (`kernel/kernel/src/arch/x86_64/context.rs`) saves only general-purpose registers and RFLAGS.
   The kernel changes CR4 only to set SMEP and SMAP (`kernel/kernel/src/arch/x86_64/syscall.rs`,
   around lines 532–541). It never sets CR4.OSFXSR. Any SSE or x87 instruction in a process is a
   hazard. This is not a C2 claim that the kernel is safe for SSE.
4. **Blender** (for context): its documented Linux baseline is SSE4.2, which is outside the
   no-FP profile.

## 10. Concrete steps for Arena 1 to replace its duplicated runtime

1. Remove SDL3's custom `_start` and custom heap from the ArenaOS build. Link
   `build/<cc>/crt0.o`, `build/<cc>/lib/libarena_c.a`, and `link/arena-user.ld` as in section 2.
2. Replace every `malloc`/`free`/`realloc` call site with the C2 libc. Do not keep a second allocator.
3. Replace stdio and exit with the C2 stdio and exit. Do not write to a debug syscall.
4. Replace the slot-number constants in Arena 1's `arenaos_syscalls.h` (`SERVICE_ENDPOINT 1`,
   `SURFACE_SLOT 2`, `CLOCK_SLOT 3`, `DIAGNOSTICS_SLOT 4`; checked at Arena 1 head `34ccfe5`)
   with lookups over `arena_startup()->caps[]` by kind and rights. Refuse the app if the match
   is absent or ambiguous. Do not guess. The stream-wait notification must be unambiguous (section 6).
5. Keep the graphics protocol (surface layout, input, ADSK-v1 on `SYS_IPC_CALL`) under
   Arena 1's own specification. C2 does not define it.
6. Build SDL3 with no SIMD, no optional backends, and no dynamic loading. Use
   `-mgeneral-regs-only -msoft-float -mno-sse -mno-sse2 -mno-mmx -mno-80387`. Do not enable `-msse*`.
   Expect blocker 9.2: floating-point SDL code will not pass the audit until a separate
   soft-float or no-FP path exists.
7. Gate the output with `c2_isa_audit.py` (must PASS) and `c2_report.py` (every referenced libc
   symbol must be `defined` in both compilers).
8. Install through the signed APB1 path and launch from All Applications, following
   `experiments/c-runtime/guest_c2.py` for the install and judging pattern. Use an exact verdict,
   not a substring match.
9. Do not add a privileged launch path, a fake capability, or a debug-syscall shortcut.

## 11. What C2 does not give Arena 1

- No SDL3 build or port.
- No graphics protocol, no compositor integration, no display or input testing.
- No verified graphical startup (B-GFX-1 blocks identifying a surface by contract).
- No floating-point path. The no-FP profile is the contract, and C2 does not change it.

## 12. Arena 1 coordination (C2.1.8, read-only)

Arena 1's branch `arena/13f9221b-arenaos` was read only (head `8e4438e` on the remote at
inspection). Its G2.1 handoff (`docs/graphics/G2.1-C-RUNTIME-HANDOFF.md`, at `34ccfe5`) was
read with `git show`. Nothing on that branch was changed.

**What Arena 1's handoff says, and what C2.1 finds:**

| Arena 1 statement | C2.1 finding | Status |
|---|---|---|
| `malloc calloc realloc free` aliases present (`src/libc_names.c`) | present; C1 allocator underneath; no second allocator | available/verified |
| integer-only `snprintf`; no floating point | present; floating-point conversions refused (`ENOTSUP`) | available/verified (integers); unsupported (floating point) |
| monotonic `arena_clock_us` | present (`include/arena/rt.h`, `src/rt.c`) | guest-verified (via G4) |
| `_exit` required by SDL3, missing from the handoff's runtime | **not defined** in C2.1. C2.1 adds no symbols. Use `exit` or `_Exit` | unsupported (gap, recorded) |
| `HAVE_MALLOC`, `SDL_FILESYSTEM_DUMMY`, `SDL_LOADSO_DUMMY` | Arena 1's SDL configuration. **Not verified by C2.** SDL is not built here | untested (not verified by C2) |
| Blocked on Luna for FP/SIMD | confirmed: no FP path under the no-FP profile (section 9) | blocked |
| Desktop service BadgedEndpoint (kind 12, WRITE) | descriptor counted by kind; not identified (B-GFX-1) | blocked |
| Window surface SharedRegion (kind 7, READ\|WRITE) | counted by kind; not identified (B-GFX-1) | blocked |
| Clock device (kind 3) | a Notification (kind 3, role Other). Only the stream-wait notification is selected, and only when it is the sole RW candidate | blocked (identification) |
| Names this proposal "ADR-0110" | that number is Luna's accepted ADR. The proposal is **ADR-0112** on this branch | corrected in this branch |

**Exact compile and link instructions against C2.1** (the same recipe as section 2; Arena 1
must link the archive and must not copy the sources):

```
# 1. Build the runtime for both compilers (from the ArenaOS repository root)
python3 experiments/c-runtime/run.py --cc gcc build
python3 experiments/c-runtime/run.py --cc clang build      # needs ARENA_ZIG=<zig binary>

# 2. Compile the application (GCC shown; flags as in section 2)
gcc -std=gnu11 -O2 -g0 -Wall -Wextra -Werror -ffreestanding -fno-builtin \
    -fno-stack-protector -fcf-protection=none -fno-pic -fno-pie \
    -fno-asynchronous-unwind-tables -fno-unwind-tables -fno-exceptions -fno-omit-frame-pointer \
    -mgeneral-regs-only -msoft-float -mno-sse -mno-sse2 -mno-mmx -mno-80387 \
    -ftls-model=local-exec -fno-strict-aliasing \
    -I experiments/c-runtime/include -I experiments/c-runtime/include/libc \
    -c app.c -o app.o

# 3. Link (crt0 first, the application, then the archive; no other objects)
gcc -nostdlib -static -no-pie -Wl,--no-dynamic-linker -Wl,-z,noexecstack \
    -Wl,--build-id=none -Wl,-z,max-page-size=4096 -Wl,-T,experiments/c-runtime/link/arena-user.ld \
    experiments/c-runtime/build/guest-gcc/crt0.o app.o \
    experiments/c-runtime/build/guest-gcc/lib/libarena_c.a -o app-gcc.elf

# 4. Audit (mandatory; a build that fails the audit is not ArenaOS-compatible)
python3 experiments/c-runtime/c2_isa_audit.py app-gcc.elf
```

Those commands were not run against an SDL3 tree, so SDL3 compile and link are **untested**.
The flags are the same ones `run.py` uses to build App A and App B, which are guest-verified
(see `docs/compat/C2.1-VERIFICATION.md` for the commit).

**Contract gaps and blockers Arena 1 must hold:**

1. `_exit` is not provided. Do not add a second `_exit`. Route process termination through `exit`.
2. Floating point and SIMD: SDL3 cannot pass the ISA audit (section 9, blocker B-FP-1 and B-FP-2).
3. Graphical identification: B-GFX-1 (see `docs/compat/C2.1-B-GFX-1-HANDOFF.md`).
4. Do not index graphical descriptors by slot number. Use kind and rights, and refuse on ambiguity.
5. `_Thread_local` is the supported thread-local mechanism. `tss_*` is refused.
