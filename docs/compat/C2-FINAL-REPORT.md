# C2 final report: reusable native C runtime and library compatibility

Branch: `arena/f89ea987-arenaos`. Status: **EXPERIMENTAL**. Nothing here is a production
ABI, a production broker change, or an SDL3 compatibility claim.

## 0. Read this first: history and re-clone

The sandbox was re-cloned after the C2 foundation work. The local commit objects of the
earlier C2 commits (`e3e63c0`, `215ae17`, and the uncommitted work that was later
committed as `76548cd` and `d4d665c`) were not on origin, and they were lost with the
clone. The branch on origin was `0350f33` (C1 final).

Recovery: the working files were restored from a tar snapshot taken before the re-clone
and verified in this sandbox before commit. The branch was reset to `0350f33`
(`origin/arena/f89ea987-arenaos`), and the work was committed as:

- `2d0b1cd` C2.0 plan and integration audit (`C2-PLAN.md`, `C2-INTEGRATION-PLAN.md`).
- `9d622a3` C2.1 through C2.7 code: runtime, libc, threads, graphical contract,
  demos, harness, compatibility report, ISA audit.
- The commit containing this report and the receipts (the branch head after it is the
  final SHA reported in chat).

The original SHAs are **not** reused and must not be cited. All evidence below was
re-collected on the reconstructed commits in this sandbox. The guest receipts record
`source_commit: 9d622a3` with `worktree_dirty_paths: []`.

## 1. Status summary

| Area | Status | Evidence class |
|---|---|---|
| C2.0 audit and integration plan | Done | document |
| C2.1 reusable static runtime (`libarena_c.a`, `crt0.o`, headers) | Implemented, static-built, guest-tested | static + guest |
| C2.2 minimal libc subset (110 declared, 110 defined) | Implemented; 53 host-referenced, 30 guest-referenced, 42 untested, 7 refused | host + static + guest (partial) |
| C2.3 threads and sync (C11 subset) | Implemented; create, join, mutex, condvar timed wait, call_once guest-tested. Refusals documented | guest (partial), host |
| C2.4 graphical runtime contract | **Blocked for identification** (B-GFX-1). Count-only query implemented and host-tested. No graphical guest run | host + doc |
| C2.5 two native demo apps | Done: App A (console) and App B (upstream xxHash v0.8.4, unmodified) | guest |
| C2.6 toolchain validation and compatibility report | Done for GCC and Clang symbol agreement, ABI constants, ISA audit. Several ABI items untested (see §5) | static + host + guest |
| C2.7 gates | Host ASan/UBSan green; guest suites green on both compilers | host + guest |
| Graphical smoke test (optional) | **Not attempted.** Not permitted by the current graphical contract (B-GFX-1) | not run |

## 2. Evidence (separated)

### 2.1 Host-only (ASan + UBSan, glibc reference where meaningful)

Receipt: `experiments/c-runtime/receipts/host-run.txt`.

- `run.py host`: **LIBC-HOST checks=971 failures=0**, **HOST-ONLY RESULT PASS (1569 checks)**.
- Includes: formatter differential against glibc, `strtol` family differential, string
  and memory tests, `qsort`, aligned allocation and invalid-alignment refusal, `calloc`
  overflow, invalid free, realloc failure, atexit order, `div` family, `srand`/`rand`,
  startup count semantics.
- xxHash reference (glibc, host): 319 inputs, digest `1df15d5d2925ddb0`, 7 streaming checks.

Host evidence is **not** kernel-side memory-safety proof.

### 2.2 Static (build, ABI, symbols, ISA)

Receipts: `receipts/c2-isa-audit.txt`, `receipts/audit-gcc.txt`, `receipts/audit-clang.txt`,
`experiments/c-runtime/compat/c2-compat-report.json`.

- `run.py abi`: 92 constants checked, 0 mismatches.
- `run.py --cc gcc build` and `--cc clang build`: rc 0. Both archives built.
- Compatibility report: 110 declared, 110 defined in both archives, 0 missing,
  **GCC and Clang symbol sets identical**.
- ISA audit (function-scoped, `c2_isa_audit.py`): PASS on all 8 guest ELFs (C2 and C1, both
  compilers). Instructions scanned: 8,141 to 16,450 per ELF. Zero vector-register operands,
  zero x87 instructions inside functions.
- Scanner controls (`python3 c2_isa_audit.py --self-test`, PASS): a negative control built
  with SSE double math FAILs (11,358 vector hits, mostly glibc startup code); a positive
  control built with the C2 no-FP flags and a freestanding link PASSes (8 instructions).
  An earlier negative check on Arena 1's G2 `SDL3App` ELF (79 vector-operand instructions)
  was run before the re-clone; that scratch ELF is no longer present, so it is reported
  here as a prior observation, not as current evidence.
- Boot-image audit (`run.py --cc X audit`): rc 0 for GCC and Clang, 0 SIMD and 0 x87.

Note: a whole-section scan gave false positives (pointer tables in `.text` decoded as
x87 opcodes). The function-scoped scan replaces it.

### 2.3 Guest (QEMU, signed APB1 install, launch from All Applications)

Receipts: `receipts/guest-suite-verdicts.txt`, `receipts/c2-guest-summary-{gcc,clang}.json`,
`receipts/c1-native-guest-summary-{gcc,clang}.json`, `receipts/c2-bundle-identity-{gcc,clang}.json`.

| Suite | GCC | Clang | Verdict |
|---|---|---|---|
| C2 multi-app (`guest_c2.py --runs 5`) | **5/5** | **5/5** | exact PASS; exit sequence `[58, 58, 61, 63]`; boot rc 0 |
| C1 native app (`guest_native.py --runs 5`) | **5/5** | **5/5** | exact PASS; 93 checks; exit `[58, 57]` |
| Legacy crt-probe (`run.py guest`, 3 runs) | **3/3** | **3/3** | `CRT-PROBE RESULT PASS (6/6)` |

Per-run details: every C2 run passed with `failures: []`, `boot_rc: 0`, exit lines
`[58, 58, 61, 63]`. The C1 runs passed with 93 checks each.

What the C2 guest suite proves, each with an exact verdict:

- Four bundles installed through protected APB1 (two positive, two stream-less negative),
  using the normal install and launch path (All Applications search, Enter, wait for
  stream output or stream-less exit).
- App A (console): stdio `puts`, `fputs`, `printf`, `snprintf`, and the float refusal;
  granted stdin read via `arena_stream_read_some` (would-block class observed, not
  blocked); allocator, including aligned and invalid-argument `errno`; monotonic time,
  REALTIME refusal, nanosleep; three threads with 500 mutex increments each (1500 total);
  `call_once`; timed condvar timeout; atexit handler output after the RESULT line, then
  exit status 61.
- App B (xxHash, upstream, unmodified): 319-input corpus digest `1df15d5d2925ddb0`, the
  same value as the glibc host reference; exit status 63.
- Negative builds: a stream-less build of each app exits 58 (`ARENA_E_NO_STREAMS` path),
  before any output.
- Two apps on one runtime archive: each positive app links the same `libarena_c.a`
  (no per-app copy), and both run in the same boot.
- No debug syscall: static precheck confirms ordinary app ELFs do not link the legacy
  `SYS_DEBUG_WRITE` path.

Test-only signing: every guest bundle is signed with the **RFC 8032 development fixture,
test-only, not a production key**. This is recorded in the bundle identity receipts.

Failures during this work, all fixed and re-run (none is hidden):

1. Static precheck failed: the legacy debug path was linked into ordinary apps. Fixed by
   moving it to `legacy_debug.c` behind a weak reference.
2. Install row mapping: rows were assigned in display-name order, not file-name order.
   Fixed.
3. `All Applications` launch failed for the stream-less app (a stray Terminal window took
   focus). Fixed by dismissing open windows, opening the Apps menu, and waiting before typing.
4. Guest run found real defects, now fixed:
   - `calloc` overflow and `aligned_alloc` refusal did not set `errno`. Fixed.
   - `atexit` output was lost because the app closed the streams before the handlers ran.
     Exit order is now handlers, then flush, then close, then end (C11 order).
   - A test expectation (a snprintf length) was wrong. Replaced with `strlen`.
5. Host run found `div`, `ldiv`, `lldiv` declared but not defined. Implemented.
6. A full guest suite was re-run after every runtime change that touched the archive.

## 3. C2.4 graphical runtime contract: findings and blocker

**Validated (against real descriptors, read-only):**

- The production desktop launches graphical apps with ARST v2 descriptors. Slot 1 is a
  BadgedEndpoint, slot 2 a SharedRegion (surface), slot 3 a Notification (clock). Slots
  4 to 7 are placeholders or optional. Streams add StreamSet and StreamWake. NATIVE_SYNC
  adds SyncDomain. Maximum 7 descriptors. **Every graphical descriptor has role `Other`**
  (`/tmp/desktop74.rs`, `startup_cap_descriptor`).
- The runtime validates each descriptor's kind, rights, and role (`startup.c`
  `role_ok`). It exposes stream, sync, and notification indices by role. It does not
  expose a surface or endpoint index.
- Arena 1's G2 `SDL3App` (read-only, `9311dd0`): own `_start` with no TLS and no ARST
  validation; slot constants 1 SERVICE_ENDPOINT, 2 SURFACE, 3 CLOCK, 4 DIAGNOSTICS, not
  ABI-guaranteed.

**Added (experimental, host-tested):** `arena_startup_count_kind(record, kind, rights_mask)`
in `src/startup_query.c` and `include/arena/rt.h`. It counts verified descriptors of one kind
with the given rights. It returns -1 when no verified record is present. It does **not**
identify a service. This is an addition to the experimental SDK header, not a change to
the production startup ABI, the broker, or `tools/startup_abi.py`.

**Blocker B-GFX-1: service identity is not in the Startup ABI v2 contract.**

- Problem: a graphical app receives BadgedEndpoint, SharedRegion, and Notification
  descriptors, all with role `Other`. Nothing in the record says which SharedRegion is the
  surface, or which endpoint is the compositor. Two SharedRegions of the same kind and
  rights are indistinguishable. Choosing by slot number is an ABI assumption, which the
  task forbids.
- Consequence: a native runtime cannot discover a graphical service through the contract
  alone. It must either guess (not permitted), or rely on a per-protocol handshake that
  Arena 1 owns, or wait for an ABI-level tag.
- Options for architecture review (not implemented; wire change requires ADR and review):
  (a) add a service-tag field or a new role to the descriptor (ABI change, coordinated
  Rust and C); (b) a protocol-level handshake on the endpoint that names the surface,
  defined by Arena 1 under its own spec (no ABI change); (c) keep the positional
  convention as a documented, versioned contract (requires architecture approval).
- Recommendation: option (b) for Arena 1 now, and option (a) only with an ADR, because
  it touches the production wire.

**Not attempted:** the optional graphical smoke test. It would need either a privileged
path (forbidden) or an identification that B-GFX-1 does not allow.

**Unsupported profiles:** if a record is absent, the runtime reports `present == 0` and
the app must refuse. A present but invalid record exits with status 122 before `main`.
A graphical profile without its descriptors is therefore refused by the app, not guessed.

## 4. Blocker list (updated)

| ID | Blocker | Owner | Status |
|---|---|---|---|
| B-GFX-1 | No service identity for graphical descriptors (role `Other`). Surface and endpoint cannot be found by contract | Architecture, Arena 1 | Open; options in §3 |
| B-FP-1 | No soft-float runtime. `-msoft-float` code that uses `float` or `double` needs `libgcc` helpers that C2 does not link (`-nostdlib`). SDL3 float code fails to link (fail-closed) | Runtime | Open; a verified helper library needs approval |
| B-FP-2 | Kernel does not save FP or SIMD state (`switch_context` saves none; no OSFXSR). Any SSE or x87 is a hazard. This is the reason for the no-FP profile | Kernel | Open (ADR-level). C2 does not implement FPU state |
| B-FP-3 | Arena 1's `SDL3App` used SIMD in a prior static observation (79 vector-operand instructions, `SDL_snprintf` among them; scratch ELF no longer present). Must be re-confirmed and rebuilt with no SIMD | Arena 1 | Open |
| B-FP-4 | Blender requires SSE4.2 (documented baseline) and is outside the no-FP profile | Product | Open; not in scope |
| B-FS-1 | No file authority. `fopen`, `freopen`, `remove`, `rename` refused. Files need a broker interface that does not exist | Broker | Open; C2 stops here by design |
| B-TIME-1 | No wall-clock grant (`time`, `CLOCK_REALTIME` refused). No CPU time (`clock`) | Kernel or service | Open |
| B-THR-1 | No thread-specific keys (`tss_*` refused), no recursive mutex, no `mtx_timedlock` | Runtime | Open; `_Thread_local` is the supported alternative |
| M9 | `tools/startup_abi.py:17` has `CAP_MAX = 4`, enforced at line 194. The runtime and `include/abi.h` use 7. A record with more than four descriptors is refused by the Python tool | Tooling | **Verified this session.** Not changed (requires authorization). Regression-test proposal: a Python test asserting `tools/startup_abi.py` `CAP_MAX` equals `ARENA_ARST_CAPABILITY_MAX` from `include/arena/abi.h`, and a 7-descriptor record round-trip |
| ADR-0111 | Stream ring counter wrap (u32 counters indexed by `counter % 768` are discontinuous at 2^32). Documented, unresolved | Architecture | Documented; wire change needs ADR review with M and C. Not changed |
| ADR-0110 | Native C runtime profile | Architecture | Proposed; not accepted by C2. No production interface change |
| Untested | 42 symbols defined but untested (see `C2-LIBC-COVERAGE.md`) | C2 follow-up | Open |

## 5. Toolchain validation (C2.6): coverage and gaps

Covered by tests on both compilers (guest, exact PASS): TLS isolation (C1 native T8),
variadic formatting (`printf` family), multi-TU linking (App B links two objects, the
driver and the unmodified `xxhash.c`, plus the archive), aligned allocation, init and
refusal error paths (stream-less exit 58; sync refusals), exit ordering. Both compilers
build the same C and pass the same verdicts.

Covered by static checks only: symbol agreement between GCC and Clang archives; weak
symbol behavior of the legacy debug reference (checked with `nm` in the static precheck);
no SIMD or x87 (ISA audit).

Not covered by any test: static initialization (`.init_array` runs before `main` in
`crt0.c`, but no test registers a constructor and checks it ran); stack alignment (no
dedicated check; the compilers enforce the ABI); function-pointer calls from constructors;
limit tests beyond the allocator and atexit bounds.

## 6. Constraint compliance (checked this session)

- No production kernel change; no new syscall or capability type; no production startup ABI,
  broker, or `tools/startup_abi.py` change. (The `arena_startup_count_kind` addition lives
  in the experimental SDK header, not production.)
- No Linux ABI, no dynamic linker, no system libc. `-nostdlib -static -no-pie`.
- No FP or SIMD in apps (ISA audit PASS). No kernel FPU state.
- No SDL3 build. No Arena 1 branch modification (read-only inspection only). Luna's branch
  not touched. `main` not touched.
- No fake pthreads. No second allocator (C1 VM allocator only).
- No privileged launch shortcut. No hardcoded image authority. Apps are launched through
  the normal APB1 install and All Applications path.
- Filesystem and PID authority: none granted. Standard streams are not filesystem authority.
- RFC 8032 development fixture used for test bundles only.
- Not merged, not released, no PR opened.

## 7. Arena 1 handoff

The migration contract is `docs/compat/C2-SDL3-INTEGRATION.md`. In short:

1. Link `build/<cc>/crt0.o`, `build/<cc>/lib/libarena_c.a`, and `link/arena-user.ld` with the
   flags in `run.py` (`common_flags`, `LINK_FLAGS`). Remove SDL3's custom `_start` and heap.
2. Replace the duplicated allocator, stdio, and exit paths with the C2 runtime.
3. Replace slot-number capability lookups with role and kind lookups over
   `arena_startup()->caps[]`. Refuse on absence or ambiguity. This does not solve B-GFX-1;
   Arena 1 needs option (b) from §3 for the surface and compositor endpoint.
4. Build SDL3 with no SIMD, no dynamic loading, no optional backends. Gate with
   `c2_isa_audit.py` (PASS) and `c2_report.py` (every referenced symbol defined in both
   compilers).
5. Install through signed APB1 and launch through All Applications. Use exact verdicts.

Arena 1 must not depend on: `tss_*`, timed mutexes, recursive mutexes, floating-point
formatting, file access, wall-clock time, or SDL float code without B-FP-1 being resolved.

## 8. Reproduction (commands)

```
export ARENA_ZIG=/home/user/.arena-tools/zig/pkg/ziglang/zig     # zig 0.17.0 via pip --target
source tools/dev-env/env.sh                                       # QEMU and Rust toolchains
cd experiments/c-runtime
python3 run.py host                                               # host, ASan/UBSan
python3 run.py abi
python3 run.py --cc gcc build && python3 run.py --cc clang build
python3 run.py --cc gcc audit && python3 run.py --cc clang audit
python3 c2_report.py                                              # compat/c2-compat-report.json
python3 c2_isa_audit.py build/guest-gcc/*.elf build/guest-clang/*.elf
python3 guest_native.py --cc gcc --runs 5                         # C1 native
python3 guest_native.py --cc clang --runs 5
python3 guest_c2.py --cc gcc --runs 5                             # C2 multi-app
python3 guest_c2.py --cc clang --runs 5
python3 run.py --cc gcc guest                                     # legacy crt-probe, repeat 3x
python3 run.py --cc clang guest
```

Environment note: the sandbox toolchain (QEMU 11.0.2, Rust 1.97.0) was reinstalled with
`tools/dev-env/bootstrap.sh` after the re-clone. The Zig binary was installed with
`pip install --target /home/user/.arena-tools/zig/pkg ziglang==0.17.0`.

## 9. Remaining work (not done in C2)

- Close the 42 untested symbols (signal and wait paths, `fgetc`/`fputc` family, `perror`, `setvbuf`, `thrd_detach`, and others).
- Resolve B-GFX-1 (architecture decision) before any graphical runtime contract.
- Resolve B-FP-1 (soft-float helper) before any float-using library.
- Authorize and fix M9 (`CAP_MAX`), with the proposed regression test.
- Stack-alignment and constructor-from-init-array tests.
- Optional graphical smoke test, only after B-GFX-1.
