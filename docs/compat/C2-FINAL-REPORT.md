# C2 final report: reusable native C runtime and library compatibility

Branch: `arena/f89ea987-arenaos`. Status: **EXPERIMENTAL**. Nothing here is a production
ABI, a production broker change, or an SDL3 compatibility claim. No PR is opened and nothing
is merged or released.

## 0. History, commits, and which evidence applies to which code

The sandbox was re-cloned after the early C2 foundation work. Four local commit objects
(`e3e63c0`, `215ae17`, `76548cd`, `d4d665c`) were lost with the clone. **They are not cited
as present anywhere in this report, and no evidence depends on them.** The branch was reset to
`0350f33` (C1 final, `origin/arena/f89ea987-arenaos`). The working files were restored from a
tar snapshot taken before the re-clone and verified before commit.

C2 commits on the branch, in order:

| Commit | Content |
|---|---|
| `2d0b1cd` | C2.0 plan and integration audit |
| `9d622a3` | C2.1–C2.7 code: runtime, libc, threads, graphical contract, demos, harness, report, ISA audit |
| `feda000` | Coverage and SDL3 docs, `9d622a3` receipts, `c2_isa_audit.py --self-test` |
| `7b27c67` | App A extended to nine groups; `guest_c2.py` expectations for nine groups |
| `e1616f1` | Host coverage for refusal and exit paths; report counting fixes; x87 control in the audit self-test |
| `b4b5a66` | Receipts at source commit `7b27c67` (replace the `9d622a3` receipts) |
| `f31c538` | Coverage and SDL3 integration docs corrected to source and evidence |
| (this commit) | Plan status and this final report |

"C2.8" in this report labels the closeout pass that produced `e1616f1` through this commit: the guest extension, the report and audit corrections, the receipts, and the documentation.

The **final SHA is the head of this branch after the last commit**, reported in chat. A
commit cannot name its own SHA inside its own files.

**Which evidence applies to which code.** The guest receipts were produced from binaries built
at `7b27c67`. The guest-input tree (`experiments/c-runtime/src`, `app`, `include`, `link`,
`run.py`, the two guest harnesses, and `third_party`) has had **zero diff lines** since
`7b27c67`. The later commits change only host tests, the report and audit tooling, and docs.
The receipts record `worktree_dirty_paths` for those non-guest files. Each receipt therefore
cites `7b27c67` for the guest binaries. The `9d622a3` receipts are superseded and no longer
cited for App A.

## 1. Status summary

Evidence classes: **implemented**, **host** (ASan+UBSan host test), **static** (build, ABI,
symbol, or ISA check), **guest** (exact-PASS QEMU run), **proposed** (documented, not built),
**blocked** (needs architecture approval), **remaining** (not done in C2).

| Area | Status | Evidence |
|---|---|---|
| C2.0 audit and integration plan | Done | document |
| C2.1 reusable static runtime (`libarena_c.a`, `crt0.o`, headers) | Implemented, built with GCC and Clang, guest-tested. No separate installed SDK tree was produced; the headers and archive are used in place | static + guest |
| C2.2 libc subset (110 declared, 110 defined in both archives) | Implemented. 69 host-referenced, 48 guest-referenced, 10 refused by design, 5 without a test reference | host + static + guest (partial) |
| C2.3 threads and sync (C11 subset, not pthreads) | Implemented. Create, join, detach, mutex, trylock, condvar, timed wait, `call_once`, TLS isolation guest-tested. `thrd_exit` and `tss_delete` untested | guest + host |
| C2.4 graphical runtime contract | **Blocked** for service identification (B-GFX-1). The count-only query is implemented and host-tested. No graphical guest run | host + doc, blocked |
| C2.5 two native demo apps | Done: App A (nine groups, console) and App B (xxHash v0.8.4, unmodified). Both guest-tested on both compilers | guest |
| C2.6 toolchain validation and compatibility report | Done for symbol agreement, ABI constants, and the ISA audit. Finding: Zig clang compiles double math to x87 under the no-FP flags; the audit is the enforcement | static + host |
| C2.7 gates | Host green; guest suites green on both compilers; receipts at `7b27c67` | host + guest |
| Graphical smoke test | **Not attempted.** Not permitted by the current graphical contract (B-GFX-1) | not run |
| SDL3 build or port | **Not done** (out of scope). Migration contract written, not applied to Arena 1 | document only |

## 2. Evidence (separated)

### 2.1 Host-only (ASan + UBSan, glibc reference where meaningful)

Receipt: `experiments/c-runtime/receipts/host-run.txt` (from `python3 run.py host`).

- **LIBC-HOST checks=988 failures=0.** **HOST-ONLY RESULT PASS (1586 checks, known protocol
  defect pinned).** The pinned defect is ADR-0111, the ring counter wrap, which is documented
  and not fixed.
- Coverage includes: the formatter differential against glibc; the `strtol` family
  differential; string and memory tests; `qsort`; aligned allocation and invalid-alignment
  refusal; `calloc` overflow; invalid free; realloc failure; atexit order; the `div` family;
  `srand` and `rand`; startup count semantics.
- New in C2.8 (`e1616f1`): `vsnprintf`, `vsprintf`, `vprintf`, and `vfprintf` through real
  `va_list` wrappers; `__arena_errno_location`; `fopen`, `freopen`, `remove`, `rename` refused
  with `ENOSYS`; `fgetc`, `fgets`, `fread`, `fclose`, `getc` refuse a NULL stream with `EBADF`
  (none of these reads can block); `_Exit`, `abort`, and a failed assertion, each checked in a
  forked child for exit status 7, 134, and 134.
- xxHash reference (glibc, host): 319 inputs, digest `1df15d5d2925ddb0`, 7 streaming checks.

Host evidence is **not** kernel-side memory-safety proof.

### 2.2 Static (build, ABI, symbols, ISA)

Receipts: `receipts/abi-check.txt`, `receipts/audit-gcc.txt`, `receipts/audit-clang.txt`,
`receipts/c2-isa-audit.txt`, and `experiments/c-runtime/compat/c2-compat-report.json`.

- `run.py abi`: **92 constants checked, mismatches []**.
- `run.py --cc gcc build` and `--cc clang build`: rc 0. Both archives built at `7b27c67`.
- Compatibility report (`c2_report.py`): declared 110; defined in both archives 110;
  missing 0; defined in one compiler only 0; **GCC and Clang symbol sets identical**.
  Host-referenced 69; guest-referenced 48; refused by design 10; no test reference 5
  (`clock`, `time`, `getchar`, `thrd_exit`, `tss_delete`).
- ISA audit (function-scoped, `c2_isa_audit.py`): **PASS on all 8 guest ELFs** (C2 and C1,
  both compilers; 0 vector-register operands and 0 x87 instructions inside functions).
- Boot-image audit (`run.py --cc X audit`): rc 0 for GCC and Clang; 0 SIMD and 0 x87.
- Scanner controls (`python3 c2_isa_audit.py --self-test`): **PASS**. Three controls:
  - an SSE negative control FAILs (11,358 vector hits, mostly glibc startup code);
  - an **x87 negative control** FAILs with x87 hits (added in C2.8 because the audit had no
    x87 control before);
  - an integer-only freestanding positive control PASSes (8 instructions).

**Finding (static, C2.8): the compile flags do not enforce the no-FP profile for clang.**
With the no-FP flags (`-mgeneral-regs-only -msoft-float -mno-sse -mno-sse2 -mno-mmx -mno-80387`),
Zig's clang (`$ARENA_ZIG cc -target x86_64-freestanding-none`) compiled a `volatile double`
multiply to `fldl`, `fmuls`, and `fstpl` (x87) and linked it without error. The
audit FAILs that ELF and lists 11 x87 instructions. GCC refuses the same multiply at compile
time with `SSE register return with SSE disabled`. So the ISA audit is the gate that enforces
the profile, not the flags. This was measured with a scratch probe; the probe is not part of
the repository. Reproduction: compile a one-function C file containing a `volatile double`
multiply with the flags above, link it with the freestanding recipe in
`C2-SDL3-INTEGRATION.md` section 2, and run `c2_isa_audit.py` on the ELF.

**Prior observation (not current evidence).** Before the re-clone, a raw scan of Arena 1's G2
`SDL3App` ELF found 79 vector-operand instructions. That scratch ELF is not present in this
sandbox and cannot be reproduced here. It is excluded from every current claim. Arena 1's
branch was fetched read-only (`git fetch --refmap=`, which creates no ref) at head
`34ccfe5`. The path `experimental/sdl3-app/src/main.c` does not resolve at that head, so the
observation cannot be re-checked from this checkout.

### 2.3 Guest (QEMU, signed APB1 install, launch from All Applications)

Receipts: `receipts/guest-suite-verdicts.txt`, `receipts/c2-guest-summary-{gcc,clang}.json`,
`receipts/c1-native-guest-summary-{gcc,clang}.json`, `receipts/c2-bundle-identity-{gcc,clang}.json`,
`receipts/c2-artifact-identity.txt` (sha256 for `crt0.o`, `libarena_c.a`, and every guest ELF).

| Suite (source `7b27c67`) | GCC | Clang | Verdict |
|---|---|---|---|
| C2 multi-app (`guest_c2.py --runs 5`) | **5/5** | **5/5** | exact PASS; exit sequence `[58, 58, 61, 63]` in every run; 66 console checks each; boot rc 0; `failures: []` |
| C1 native app (`guest_native.py --runs 5`) | **5/5** | **5/5** | exact PASS; 93 checks each; exit `[58, 57]` |
| Legacy crt-probe (`run.py guest`, 3 runs) | **3/3** | **3/3** | `CRT-PROBE RESULT PASS (6/6)` |

What the C2 guest suite proves, each with an exact verdict:

- Four bundles installed through protected APB1 (two positive, two stream-less negative),
  using the normal install and launch path (All Applications search, Enter, wait for
  stream output or stream-less exit).
- **App A (console), nine groups** (`RESULT PASS groups=9/9 checks=66 failed=0`):
  - G1 stdio: `puts`, `fputs`, `printf`, `snprintf`, `putchar`, and the float refusal.
  - G2 stdin: granted read via `arena_stream_read_some` (would-block class observed, not blocked).
  - G3 allocator: `malloc`, `calloc`, `realloc`, `free`, `aligned_alloc`, overflow, and `errno`.
  - G4 time: `CLOCK_MONOTONIC` advances at least 2 ms across `nanosleep`; `CLOCK_REALTIME` refused with `ENOSYS`.
  - G5 threads: three threads, 500 mutex increments each (1500 total); `call_once`; timed condvar timeout; TLS.
  - G6 threads, extended: `thrd_current` and `thrd_equal`; `mtx_trylock` free succeeds and self-held returns `thrd_busy`; `cnd_wait` is woken by `cnd_broadcast`, and the waiter returns its status 7; `cnd_signal` with no waiters succeeds; `thrd_detach` and the worker run; `thrd_sleep` 1 ms; refusals for recursive mutex init, `mtx_timedlock`, `tss_create`, `tss_set`, and `tss_get` returning `NULL`.
  - G7 stdio, extended: `fputc`, `putc`, `fwrite`; `ferror`, `feof`, `clearerr`; `setvbuf` valid mode and bad-mode `EINVAL`; `perror` output visible.
  - G8 init: a constructor in `.init_array` ran before `main`.
  - G9 exit: the atexit handler runs after `main` returns. Its output appears after the RESULT line. G9's own `checks=1` is a registration check, not an `EXPECT`, so it is not in the RESULT total of 66 (which is the sum of G1–G8).
  - Exit status 61 when every group passes.
- **App B (xxHash v0.8.4, unmodified upstream):** 319-input corpus digest `1df15d5d2925ddb0`,
  the same value as the glibc host reference; exit status 63.
- **Negative builds:** a stream-less build of each app exits 58 (`ARENA_E_NO_STREAMS` path)
  before any output.
- **Two apps on one runtime:** each positive app links the same `libarena_c.a` (no per-app copy),
  and both run in the same boot.
- **No debug syscall:** the static precheck confirms ordinary app ELFs do not link the legacy
  `SYS_DEBUG_WRITE` path.

Test-only signing: every guest bundle is signed with the **RFC 8032 development fixture,
test-only, not a production key**. This is recorded in the bundle identity receipts.

Caveats recorded for App A (not hidden; not yet fixed, because fixing them would change the
evidence commit):

- **Detach and exit status.** G6's detached worker sets a flag and returns; the main thread
  waits on the flag, not on the worker's exit. The last-thread exit rule could in principle
  change the process status if the worker exited after main. Observed: 61 in all 10 C2 runs.
  That is observation, not a proof. The runtime has no join-free way to observe termination.
- **Uninitialized key.** G6 calls `tss_get(key)` on a `tss_t` whose `tss_create` was refused,
  so the key was never initialized. The call is harmless, but it passes an indeterminate
  value. The fix is an initializer. Recorded for the next App A change.

Failures during this work, all fixed and re-run (none is hidden):

1. Static precheck failed: the legacy debug path was linked into ordinary apps. Fixed by moving it
   to `legacy_debug.c` behind a weak reference.
2. Install row mapping: rows were assigned in display-name order, not file-name order. Fixed.
3. All Applications launch failed for the stream-less app (a stray Terminal window took focus).
   Fixed by dismissing open windows, opening the Apps menu, and waiting before typing.
4. **Found by the guest run, not the host run:**
   - `calloc` overflow and `aligned_alloc` refusal did not set `errno` on NULL. Fixed (the
     malloc family now sets `EINVAL` for a bad alignment and `ENOMEM` otherwise).
   - `atexit` output was lost because the app closed the streams before the handlers ran.
     The exit order is now handlers, then flush, then close, then end (C11 7.22.4). Fixed.
   - A test expectation (a `snprintf` length) was wrong. Replaced with `strlen`.
5. Host run found `div`, `ldiv`, and `lldiv` declared but not defined. Implemented.
6. Static ISA check (C2.8): clang emitted x87 for double math under the no-FP flags. The audit
   caught it. The audit now has an x87 negative control, and no runtime code uses floating point.
7. C2.8 report review: the coverage report counted comments and string literals as test
   references, which overstated coverage (for example, `time()` appeared covered because of the
   word "time" in a comment). It now strips comments, strings, and includes, and host matching
   uses the internal `arena_*` names. The refusal set was corrected (`fclose` removed, four
   thread and TSS refusals added).

## 3. C2.4 graphical runtime contract: findings and blocker

**Validated (against real descriptors, read-only):**

- The production desktop launches graphical apps with ARST v2 descriptors. Slot 1 is a
  BadgedEndpoint, slot 2 a SharedRegion (surface), slot 3 a Notification (clock). Slots 4 to 7
  are placeholders or optional. Streams add StreamSet and StreamWake. NATIVE_SYNC adds
  SyncDomain. Maximum 7 descriptors. **Every graphical descriptor has role `Other`**
  (`/tmp/desktop74.rs`, `startup_cap_descriptor`; scratch, from the earlier session).
- The runtime validates each descriptor's kind, rights, and role (`startup.c` `role_ok`). It
  exposes stream, sync, and notification indices by role. It does not expose a surface or
  endpoint index.
- Arena 1's G2 `SDL3App` (read-only, `9311dd0`, prior observation): own `_start` with no TLS and
  no ARST validation; slot constants `SERVICE_ENDPOINT 1`, `SURFACE_SLOT 2`, `CLOCK_SLOT 3`,
  `DIAGNOSTICS_SLOT 4`, not ABI-guaranteed. The same constants are present at Arena 1 head
  `34ccfe5` in `experimental/sdl3/src/platform/arenaos/arenaos_syscalls.h`.

**Added (experimental, host-tested):** `arena_startup_count_kind(record, kind, rights_mask)` in
`src/startup_query.c` and `include/arena/rt.h`. It counts verified descriptors of one kind with
the given rights. It returns -1 when no verified record is present. It does **not** identify a
service. This is an addition to the experimental SDK header, not a change to the production
startup ABI, the broker, or `tools/startup_abi.py`.

**Blocker B-GFX-1: service identity is not in the Startup ABI v2 contract.**

- Problem: a graphical app receives BadgedEndpoint, SharedRegion, and Notification descriptors,
  all with role `Other`. Nothing in the record says which SharedRegion is the surface, or which
  endpoint is the compositor. Two SharedRegions of the same kind and rights are
  indistinguishable. Choosing by slot number is an ABI assumption, which the task forbids.
- Consequence: a native runtime cannot discover a graphical service through the contract alone.
  It must either guess (not permitted), rely on a per-protocol handshake that Arena 1 owns, or
  wait for an ABI-level tag.
- Options for architecture review (not implemented; a wire change requires an ADR and review):
  (a) add a service-tag field or a new role to the descriptor (ABI change, coordinated Rust and
  C); (b) a protocol-level handshake on the endpoint that names the surface, defined by Arena 1
  under its own spec (no ABI change); (c) keep the positional convention as a documented,
  versioned contract (requires architecture approval).
- Recommendation: option (b) for Arena 1 now, and option (a) only with an ADR, because it
  touches the production wire.

**Related hazard (new in C2.8, static): stream-wait notification selection.** `src/startup.c`
(around line 310) sets the notification used for blocking stream waits to the **first**
`Other`-role Notification with RW rights. An app that holds another `Other`-role RW Notification
could have its stream waits bound to the wrong one. This is the static behavior, not a tested
failure. It is recorded as B-NTF-1 below.

**Not attempted:** the optional graphical smoke test. It would need either a privileged path
(forbidden) or an identification that B-GFX-1 does not allow.

**Unsupported profiles:** if a record is absent, the runtime reports `present == 0` and the app
must refuse. A present but invalid record exits with status 122 before `main`. A graphical
profile without its descriptors is therefore refused by the app, not guessed.

## 4. Blocker list (updated at C2.8)

| ID | Blocker | Owner | Status |
|---|---|---|---|
| B-GFX-1 | No service identity for graphical descriptors (role `Other`). Surface and endpoint cannot be found by contract | Architecture, Arena 1 | **Blocked**; options in §3 |
| B-NTF-1 | Stream-wait notification is the first `Other`-role RW Notification (`startup.c` ~line 310). An app with several such descriptors can bind waits to the wrong one | Runtime, Arena 1 | Open; static finding. Fix options: a documented rule Arena 1 follows (one such descriptor), or a role change (ADR) |
| B-FP-1 | No soft-float runtime, and no floating-point path under this profile. GCC refuses SSE double code at compile time; Zig clang emits x87, which the audit rejects. SDL float or double code cannot pass the audit | Runtime | Open; a verified helper library needs approval and would still need an audit path |
| B-FP-2 | Kernel saves no FP or SIMD state (`switch_context` saves general registers and RFLAGS only; the kernel sets only SMEP and SMAP in CR4, never OSFXSR). Any SSE or x87 is a hazard. This is the reason for the no-FP profile | Kernel | Open (ADR-level). C2 does not implement FPU state |
| B-FP-3 | Arena 1's `SDL3App` used SIMD in a prior static observation (79 vector-operand instructions; scratch ELF not present; path not at head `34ccfe5`). Must be re-confirmed and rebuilt with no SIMD | Arena 1 | Open |
| B-FP-4 | Blender requires SSE4.2 (documented baseline) and is outside the no-FP profile | Product | Open; not in scope |
| B-FP-5 | The compile flags alone do not enforce the no-FP profile for clang (x87 emitted). The ISA audit is the enforcement, and the audit must run on every Arena 1 build | Toolchain | Open; mitigated by the audit and its x87 control |
| B-FS-1 | No file authority. `fopen`, `freopen`, `remove`, `rename` refused. Files need a broker interface that does not exist | Broker | Open; C2 stops here by design |
| B-TIME-1 | No wall-clock grant (`time`, `CLOCK_REALTIME` refused). No CPU time (`clock`). `time` and `clock` have no test reference (`src/libc_time.c` is not in the host build) | Kernel or service | Open |
| B-THR-1 | No thread-specific keys (`tss_create`, `tss_set`, `tss_get` refused; `tss_delete` a no-op), no recursive mutex, no `mtx_timedlock` | Runtime | Open; `_Thread_local` is the supported alternative |
| M9 | `tools/startup_abi.py:17` has `CAP_MAX = 4`, enforced at line 194. The runtime (`experiments/c-runtime/include/arena/abi.h:60`, `ARENA_ARST_CAPABILITY_MAX 7u`) and the Rust platform crate (`userspace/arena-platform/src/startup.rs:16`, `CAPABILITY_MAX: usize = 7`) use 7. A record with more than four descriptors is refused by the Python tool. | Tooling | **Verified this session** (re-verified on the current tree; the earlier verification was on `9d622a3`). Not changed (requires authorization). Regression-test proposal: a Python test asserting `tools/startup_abi.py` `CAP_MAX` equals `ARENA_ARST_CAPABILITY_MAX` from `include/arena/abi.h`, plus a 7-descriptor record round-trip |
| ADR-0111 | Stream ring counter wrap (u32 counters indexed by `counter % 768` are discontinuous at 2^32). Documented, unresolved | Architecture | Documented; a wire change needs ADR review with M and C. Not changed |
| ADR-0110 | Native C runtime profile | Architecture | Proposed; not accepted by C2. No production interface change |
| Untested | 5 symbols with no test reference: `getchar` (blocks on an idle stdin; no host stdin fixture), `thrd_exit`, `tss_delete` (no-op), and `clock` and `time` (refused by design, source inspection only) | C2 follow-up | Open |
| Caveat | App A detach and exit-status race not proven (§2.3); uninitialized `tss_t` in App A G6 | C2 follow-up | Open |

## 5. Toolchain validation (C2.6): coverage and gaps

Covered by tests on both compilers (guest, exact PASS): TLS isolation (C1 native T8), variadic
formatting (`printf` family), multi-TU linking (App B links two objects, the driver and the
unmodified `xxhash.c`, plus the archive), aligned allocation, init and refusal error paths
(stream-less exit 58; sync refusals), exit ordering, **static initialization** (G8: a constructor
in `.init_array` runs before `main`; new at C2.8), and the threading refusals in G6.

Covered by host tests: formatter entry points through `va_list`, `_Exit`, `abort`, and failed
assertion exit statuses (forked child), refusal of file operations, and NULL-stream refusals.

Covered by static checks only: symbol agreement between GCC and Clang archives; the weak symbol
behavior of the legacy debug reference (checked with `nm` in the static precheck); no SIMD or
x87 in functions (ISA audit, with its controls).

Not covered by any test: stack alignment (no dedicated check; the compilers enforce the ABI);
calls from constructors into other functions (the G8 constructor sets a flag and nothing else);
limit tests beyond the allocator and atexit bounds; live blocking stream reads (`getchar`,
`fgetc` on a live stdin); `thrd_exit`; `tss_delete`.

## 6. Constraint compliance (checked this session)

- No production kernel change; no new syscall or capability type; no production startup ABI,
  broker, or `tools/startup_abi.py` change. The `arena_startup_count_kind` addition lives in the
  experimental SDK header, not production.
- No Linux ABI, no dynamic linker, no system libc. `-nostdlib -static -no-pie`.
- No FP or SIMD in apps: the ISA audit passes on every guest ELF. The audit is the enforcement
  (B-FP-5). No kernel FPU state is implemented.
- No SDL3 build. Arena 1's branch was read only (`git fetch --refmap=`, no ref created or
  changed). Luna's branch was not touched. `main` was not touched.
- No fake pthreads. No second allocator (C1 VM allocator only).
- No privileged launch shortcut. No hardcoded image authority. Apps are launched through the
  normal APB1 install and All Applications path.
- Filesystem and PID authority: none granted. Standard streams are not filesystem authority.
- RFC 8032 development fixture used for test bundles only.
- Not merged, not released, no PR opened, and the session is not ended.

## 7. Arena 1 handoff

The migration contract is `docs/compat/C2-SDL3-INTEGRATION.md`. In short:

1. Link `build/<cc>/crt0.o`, `build/<cc>/lib/libarena_c.a`, and `link/arena-user.ld` with the
   flags in `run.py` (`common_flags`, `LINK_FLAGS`). Remove SDL3's custom `_start` and heap.
   Use `-I include -I include/libc`.
2. Replace the duplicated allocator, stdio, and exit paths with the C2 runtime.
3. Replace slot-number capability lookups (`SERVICE_ENDPOINT 1`, `SURFACE_SLOT 2`,
   `CLOCK_SLOT 3`, `DIAGNOSTICS_SLOT 4` in `arenaos_syscalls.h`) with role and kind lookups over
   `arena_startup()->caps[]`. Refuse on absence or ambiguity. This does not solve B-GFX-1; Arena 1
   needs option (b) from §3 for the surface and compositor endpoint.
4. Keep at most one `Other`-role RW Notification, or the stream waits may bind to the wrong one
   (B-NTF-1).
5. Build SDL3 with no SIMD, no dynamic loading, and no optional backends. Gate every output with
   `c2_isa_audit.py` (must PASS) and `c2_report.py` (every referenced symbol defined in both
   compilers). Passing the compile flags alone is not enough (B-FP-5).
6. Install through signed APB1 and launch through All Applications. Use exact verdicts.

Arena 1 must not depend on: `tss_*`, timed mutexes, recursive mutexes, floating-point formatting,
file access, wall-clock time, or SDL float code without B-FP-1 being resolved.

## 8. Reproduction (commands)

Run from the repository root unless noted. `ARENA_ZIG` is the Zig binary (Zig 0.17.0 installed
with `pip install --target /home/user/.arena-tools/zig/pkg ziglang==0.17.0`). `tools/dev-env/env.sh`
provides QEMU 11.0.2 and Rust 1.97.0 from `tools/dev-env/bootstrap.sh`.

```
export ARENA_ZIG=/home/user/.arena-tools/zig/pkg/ziglang/zig
source tools/dev-env/env.sh
cd experiments/c-runtime
python3 run.py host                                   # host, ASan/UBSan (LIBC-HOST, HOST-ONLY)
python3 run.py abi                                    # 92 constants
python3 run.py --cc gcc build && python3 run.py --cc clang build
python3 run.py --cc gcc audit && python3 run.py --cc clang audit   # boot image: 0 SIMD, 0 x87
python3 c2_isa_audit.py --self-test                   # SSE, x87, and freestanding controls
python3 c2_isa_audit.py build/guest-gcc/*.elf build/guest-clang/*.elf
python3 c2_report.py                                  # compat/c2-compat-report.json
python3 guest_c2.py --cc gcc --runs 5                 # C2 multi-app (App A nine groups, App B)
python3 guest_c2.py --cc clang --runs 5
python3 guest_native.py --cc gcc --runs 5             # C1 native
python3 guest_native.py --cc clang --runs 5
python3 run.py --cc gcc guest                         # legacy crt-probe, repeat 3x
python3 run.py --cc clang guest
```

## 9. Remaining work (not done in C2)

- Close the 5 symbols with no test reference: a live `getchar`/`fgetc` read (needs a stdin
  fixture that cannot block), `thrd_exit`, `tss_delete`, and `clock`/`time` (which require
  linking `libc_time.c` into the host build, or a guest test).
- Fix the App A caveats: give G6's `tss_t` an initializer; find a join-free way to confirm a
  detached thread's termination before main exits, or document the limit in the runtime contract.
- Resolve B-GFX-1 (architecture decision) before any graphical runtime contract.
- Resolve B-NTF-1 (one `Other`-role RW Notification, or a role change through an ADR).
- Resolve B-FP-1 (a verified soft-float or no-FP path) before any float-using library, and
  require the audit on every Arena 1 build (B-FP-5).
- Authorize and fix M9 (`CAP_MAX`), with the proposed regression test.
- Stack-alignment test; a constructor that calls other functions.
- Optional graphical smoke test, only after B-GFX-1.
