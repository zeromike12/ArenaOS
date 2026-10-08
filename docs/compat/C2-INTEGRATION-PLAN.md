# C2 Integration Plan (C2.0 audit and design)

Status: EXPERIMENTAL. Written at C2.0 on `arena/f89ea987-arenaos` (base `0350f33`).
Sources audited (read-only):

- C1 runtime: `experiments/c-runtime/` at `0350f33`.
- Arena 1 G2 graphics checkpoint: `arena/13f9221b-arenaos` at `9311dd0c54b66f47d28248e1346ff15940b49439`
  (`experimental/sdl3/`, `experimental/sdl3-app/`, `experimental/cube-app/`, `docs/graphics/G2-*.md`).
- Production inputs at `74ace4f` (the base of this session's branch history): `userspace/desktop` launch
  paths (`userspace/desktop/src/bin/desktop.rs`), `userspace/arena-platform/src/startup.rs`
  (`CapabilityRole`, `CAPABILITY_MAX = 7`), `userspace/abi.rs`, `tools/startup_abi.py`.

Nothing on Arena 1's branch or Luna's branch was modified. Arena 1's SDL3 headers, core, and app are
read by `git show`, not checked out over this branch.

## 1. Classification of components

Each component is classed as one of:

- **SHARED-RUNTIME**: general C runtime. Lives once in `libarena_c.a` and is linked by every app.
- **SDL3-BACKEND**: SDL3 platform glue (window, input, audio, ADSK-v1 client). Owned by Arena 1.
- **EXISTING**: already provided by ArenaOS services. The C layer binds it and does not reimplement it.
- **MISSING**: needs architectural approval before code is written. Listed in section 7.

| Component | Class | Where it stands | Notes |
|---|---|---|---|
| `_start`, ARST v2 gate, stack switch, TLS install, `.init_array`, `main` | SHARED-RUNTIME | C1 `src/crt0.c`, `src/startup.c` | G2 `_start` does NOT validate the record (F-C2-02). |
| Memory allocator (buddy over one VM reservation) | SHARED-RUNTIME over EXISTING (`SYS_VM_*`) | C1 `src/alloc.c`, `src/vm.c` | G2 has its own first-fit heap (second allocator; to be retired). |
| `malloc` family, aligned allocation | SHARED-RUNTIME over the C1 allocator | C2.2 | Aligned allocation for alignment above 16 is new; see C2-LIBC-COVERAGE. |
| String and memory functions | SHARED-RUNTIME | C1 `src/string.c` (subset) | C2.2 completes the set. G2 uses its own `SDL_stdlib.c`. |
| Formatted output (`printf` family) | SHARED-RUNTIME | C1 `src/stdio.c` (integer and string only) | Float conversions to be refused, not faked. |
| stdin, stdout, stderr | SHARED-RUNTIME over EXISTING (ADR-0104 streams) | C1 `src/streams.c` | Granted by the record only. Never ambient filesystem authority. |
| Legacy serial (`SYS_DEBUG_WRITE`) | EXISTING, excluded from the C contract (ADR-0110) | C1 opt-in only | G2's OOM and log path uses it (F-C2-07). |
| Monotonic time | SHARED-RUNTIME over EXISTING (`SYS_CLOCK_NOW`) | C1 `arena_clock_us` | `clock_gettime` narrow form is C2.2. |
| Sleep | SHARED-RUNTIME over EXISTING | C1 busy-yield only | A blocking timer binding over `SYS_TIMER_ARM`/`SYS_WAIT` is existing service; deferred (F-C2-13). |
| errno | SHARED-RUNTIME | Missing in C1 | Per-thread through TLS. C2.2. |
| Threads (create, join, detach) | SHARED-RUNTIME over EXISTING (`SYS_THREAD_*`) | C1 `src/threads.c` | C11 `threads.h` names are C2.3. |
| Mutex, condition variable, once | SHARED-RUNTIME over EXISTING (`SYS_SYNC_*`, ADR-0107) | C1 `src/sync.c` | Timed mutex lock is not provided (no kernel timeout path in the C binding). |
| Thread-specific keys (`tss_*`) | SHARED-RUNTIME (deferred) | Missing | Needs a per-thread table; refused in C2.3. |
| ADSK-v1 window protocol, surface, input | SDL3-BACKEND | G2 `arenaos_client.c` | Protocol stays with Arena 1. The runtime provides only granted-resource lookup (C2.4). |
| Window and surface capability discovery | MISSING (architecture) | Position convention in the desktop | See BLK-C2-01. |
| Upstream SDL3 core | SDL3-BACKEND (Arena 1) | Vendored 3.2.24 headers and custom core | Not built here (task rule). |
| Floating point and SIMD | MISSING (architecture, Luna review) | Not supported by the kernel | See BLK-C2-02 and `C1-FPSIMD-BOUNDARY.md`. |
| General file access | MISSING (broker interface) | Not provided | Stopped by design; see BLK-C2-03. |
| Dynamic loading (`dlopen`) | Not planned | Out of scope | Prohibited by the mission. |
| Blender, SuperTuxKart | Out of scope | Not ported | Task rule. |

## 2. Manifests audited

Manifest flags (`userspace/arena-startup-abi`, `docs/`): MULTI_INSTANCE 1, BACKGROUND 2, HEADLESS 4,
STANDARD_STREAMS 8, NATIVE_SYNC 16. Mask `0x1F`.

| Application | Flags | Profile selected by the desktop | Source |
|---|---|---|---|
| C1 native probe | 28 (HEADLESS, STREAMS, NATIVE_SYNC) | headless (`launch_headless_image_v2`) | `guest_native.py` |
| C1 native probe, stream-less negative | 20 (HEADLESS, NATIVE_SYNC) | headless | `guest_native.py` |
| G2 `SDL3App.apb1` | 25 (MULTI_INSTANCE, STREAMS, NATIVE_SYNC) | **graphical** (no HEADLESS bit) | `9311dd0:experimental/sdl3-app` |
| G2 `Cube.apb1` (Rust) | not audited for the manifest | graphical | `9311dd0:experimental/cube-app` |

The desktop's test at `launch_installed_application` chooses the path. `FLAG_HEADLESS` set sends the
app to `launch_headless_image_v2`. Otherwise it sends the app to `launch_image_v2_for_app_with_document`,
which is the graphical path.

## 3. Startup records compared

Descriptors are `slot / role / kind / rights`. Roles: CWD 1, Stdin 2, Stdout 3, Stderr 4, Other 5,
StreamSet 6, StreamWake 7, SyncDomain 8. Kinds (`arena-platform/src/startup.rs`): Notification 3,
SharedRegion 7, MemoryPool 8, BadgedEndpoint 12, SyncDomain 16. Rights: READ 1, WRITE 2, COPY 4, DESTROY 8.

### 3a. Headless record (`launch_headless_image_v2`, source desktop.rs lines around 2955–3060)

| Slot | Role | Kind | Rights | Meaning |
|---|---|---|---|---|
| 0 | (ARST page) | SharedRegion | READ, DESTROY | Startup record |
| 1 | Other | Notification | READ, WRITE | Private clock/wait notification |
| 2 | StreamSet | SharedRegion | READ, WRITE | Standard stream rings (only with STANDARD_STREAMS) |
| 3 | StreamWake | Notification | WRITE | Wake notification (only with STANDARD_STREAMS) |
| 4 | SyncDomain | SyncDomain | READ, WRITE | Native sync (only with NATIVE_SYNC) |

Maximum: 4 capability descriptors (slots 1–4); slot 0 holds the record itself. This is the record that `CAP_MAX = 4` in
`tools/startup_abi.py` was written for.

### 3b. Graphical record (`launch_image_v2_for_app_with_document`, source desktop.rs lines around 3514–3640)

| Slot | Role | Kind | Rights | Meaning |
|---|---|---|---|---|
| 0 | (ARST page) | SharedRegion | READ, DESTROY | Startup record |
| 1 | Other | BadgedEndpoint | WRITE, COPY, DESTROY | Window-service endpoint, badged per session |
| 2 | Other | SharedRegion | READ, WRITE, COPY | Surface (pixel) region |
| 3 | Other | Notification | READ, WRITE (+COPY for kind 1) | Clock notification |
| 4 | Other | MemoryPool (diagnostics) **or** BadgedEndpoint (home) **or** empty | READ / WRITE, COPY | Optional tail |
| 5–6 | StreamSet, StreamWake | SharedRegion, Notification | RW / W | With STANDARD_STREAMS |
| next | SyncDomain | SyncDomain | READ, WRITE | With NATIVE_SYNC |

Maximum: 7 descriptors, which equals `CAPABILITY_MAX = 7`. The record has no role that marks the window
endpoint, the surface, or the clock. Every graphical service is role `Other`, distinguished only by kind
and by slot position.

### 3c. Differences that matter for the runtime

1. The headless record has one `Other` notification. The graphical record has three `Other`
   capabilities, and one of them is also a BadgedEndpoint that could be a home endpoint at slot 4.
2. A consumer that picks "the BadgedEndpoint" finds two candidates in the graphical record whenever a
   home directory is granted. The ABI provides no tie-breaker.
3. A consumer that picks "the SharedRegion" finds the surface, and the stream set when streams are
   granted. The stream set is distinguishable by role, but the surface is not.
4. C1's `notification` selection rule (first `Other` + Notification with exact RW) selects the clock
   on the graphical record. That is correct for the clock but is not a general service identity.

## 4. Findings

Severity: H high, M medium, L low. "Measured" means checked in this session. "Reported" means taken from
Arena 1's documents and not reproduced.

- **F-C2-01 (H, measured)**: Arena 1's `SDL3App.apb1` ELF contains SSE/XMM instructions. Extracted from
  `9311dd0:experimental/sdl3-app/SDL3App.apb1` (ELF at offset 784, section-header end 36 704 bytes):
  `objdump -d` shows 79 instructions with XMM operands (movaps 34, movss 30, cvtsi2sd 3, cvtsi2ss 3,
  subss 2, movd 2, xorps 1, mulsd 1, movsd 1, divsd 1, addsd 1). They sit in `main`, `SDL_SetError`,
  `SDL_snprintf`, `SDL_Log`, `SDL_SendMouseMotion`, and others. The C1 contract forbids these
  instructions (`C1-FPSIMD-BOUNDARY.md`). The cube app (Rust) has no XMM instructions.
  Consequence: the graphical path runs SSE code. Arena 1 reports a passing guest test, which implies the
  firmware left `CR4.OSFXSR` set and that the kernel never saves XMM state on a context switch. That is
  an inference from Arena 1's report and was not reproduced here. Corrupted XMM state across processes
  is possible and unverified. Route to Luna (F-FP4, F-FP9).
- **F-C2-02 (H)**: G2 `_start` (`arenaos_entry.c`) maps slot 0 and copies 4096 bytes. It does not check
  the ARST magic, the version, the canonical layout, the roles, the capability kinds, or the rights. It
  then unmaps and destroys slot 0 and runs `main(1, {"sdl3-app"})`. It sets no TLS and gives no argv or
  envp. The C1 gate does all of these (`startup.c`, 419 lines). A graphical app must not skip this gate.
- **F-C2-03 (H, measured)**: G2 hardcodes the graphical slots (`arenaos_syscalls.h`): `SERVICE_ENDPOINT 1`,
  `SURFACE_SLOT 2`, `CLOCK_SLOT 3`, `DIAGNOSTICS_SLOT 4`. The ABI guarantees none of these. Slot 4 is only
  a MemoryPool, a BadgedEndpoint, or empty depending on the grant. Today's desktop happens to match, but
  the match is a convention, not an ABI guarantee. The task rule forbids this assumption.
- **F-C2-04 (H)**: no ABI-level identity for the window endpoint or the surface. See BLK-C2-01.
- **F-C2-05 (M, measured)**: M9. `tools/startup_abi.py` sets `CAP_MAX = 4` and rejects `capc > CAP_MAX`
  (line 194). The Rust validator and the C parser accept 7. The graphical record can reach 7 descriptors,
  so the Python tooling cannot build a graphical record. Verify and propose a regression test (C2-SDL3
  doc §8). Do not change the tool without authorization.
- **F-C2-06 (L, measured)**: G2 documents disagree on the heap size. `G2-RUNTIME-CONTRACT.md` §3.4 says a
  16-page (64 KiB) pool. `arenaos_memory.c` defines `HEAP_SIZE (128 * 1024)` with a comment saying
  512 KiB. The OOM message says 512 KiB. `G2-FINAL-REPORT.md` says 128 KiB. Four figures for one
  allocator.
- **F-C2-07 (M, measured)**: G2's OOM path and its log output use `SYS_DEBUG_WRITE` (serial). ADR-0110
  excludes that call from the C contract. The migration target is the granted stderr stream.
- **F-C2-08 (H, reported and partly measured)**: G2 SDL3 math needs float helpers (`sinf`, `cosf`,
  `sqrtf`, and others), and F-C2-01 shows scalar double arithmetic in the app. Both are blocked by the
  no-FP profile. Upstream SDL3 cannot run under the current profile.
- **F-C2-09 (positive, measured)**: G2's 19 syscall constants in `arenaos_syscalls.h` all equal the
  values in `userspace/abi.rs`. No numbering drift. Not a substitute for a full ABI check.
- **F-C2-10 (M, reported)**: G2 disables filesystem support (`-DSDL_FILESYSTEM=OFF`, "blocked on Arena 2
  libc VFS"). C2 does not provide general file access (BLK-C2-03).
- **F-C2-11 (M, measured)**: G2's `arenaos_memory.c` is a first-fit allocator, separate from C1's buddy
  allocator. Two allocators in one process is what C2 forbids. After migration, Arena 1 uses the
  SHARED-RUNTIME allocator.
- **F-C2-12 (L, measured)**: C1's public `abi.h` exposed every raw `ARENA_SYS_*` number as a macro. That
  conflicts with "headers must not depend on private kernel details". C2.1 moves the numbers to an
  internal header and keeps the public names only.
- **F-C2-13 (L, measured)**: no blocking sleep in C1 (`arena_busy_sleep_us` burns CPU). G2 uses
  `SYS_TIMER_ARM` plus `SYS_WAIT` on its clock notification. The C binding for that is existing service
  and is deferred to C2.2 or later.
- **F-C2-14 (M, measured)**: the G2 entry does not set TLS. Any `_Thread_local` or compiler-generated TLS
  in G2 code is undefined behaviour, and G2's SDL code does not use TLS today. The shared runtime sets
  TLS before `main`.

## 5. Shared entry path (design)

One entry path for all native apps, headless or graphical:

1. `_start` (`crt0.o`, shared) receives the process entry.
2. The ARST v2 gate runs (C1 `startup.c`). It maps slot 0 once, copies the page, unmaps and destroys
   slot 0, and parses a private copy. A missing record gives `present == 0`. A bad record gives
   `ARENA_E_STARTUP` and the app exits with the defined status. Nothing is silently degraded.
3. The stack switches to a runtime-owned stack. TLS is installed. `.init_array` runs.
4. `main(argc, argv)`. Stdio attaches only if the record grants streams.
5. The profile layer (`arena/profile.h`, C2.4) reads the verified record and reports what the app was
   granted, by kind and role. It never assumes a slot's service.
6. `exit` runs `atexit` handlers and flushes stdio. The status goes to `arena_exit`.

The graphical profile does not change this path. It adds a discovery function. The window protocol and
its slots remain Arena 1's, and they are used only after the record proves them (BLK-C2-01 limits what
can be proved today).

## 6. Capability behavior under each profile

| Capability | Headless record | Graphical record | Runtime behavior |
|---|---|---|---|
| Streams | granted when STANDARD_STREAMS | granted when STANDARD_STREAMS | stdio works. Otherwise `ARENA_E_NO_STREAMS`. |
| Sync domain | granted when NATIVE_SYNC | granted when NATIVE_SYNC | threads and mutex. Otherwise `ARENA_E_NO_CAP`. |
| Notification (private) | slot 1 | slot 3 (clock) | blocking stream waits. Found by role Other and exact RW. |
| Window endpoint | none | slot 1 (unidentifiable) | Not discovered by the runtime. Reported as ambiguous. |
| Surface | none | slot 2 (unidentifiable) | Not discovered by the runtime. Reported as ambiguous. |
| Home or diagnostics tail | none | slot 4, optional | Reported as an unlabelled Other entry. |
| File access | none | optional home grant | Not provided by C2 (BLK-C2-03). |

## 7. Blockers (architectural approval needed; nothing landed)

- **BLK-C2-01 Graphical capability identity.** ARST v2 has roles CWD, Stdin, Stdout, Stderr, Other,
  StreamSet, StreamWake, SyncDomain. Window endpoint, surface, and clock are all Other. The ABI cannot
  tell a graphical app which descriptor is its window endpoint. Options (for ADR, M and C review):
  (a) new roles `WindowService` and `Surface` in a v3 record (wire change, needs coordinated Rust and C
  work and a version bump); (b) a documented, signed manifest field that names the expected kinds and
  slots, verified by the desktop before launch (no wire change, but still a launch-contract change);
  (c) keep the position convention, and have Arena 1 refuse to run when the record does not match the
  convention exactly. C2 recommends (c) as the interim and (a) or (b) as the target. Until one is approved,
  the C runtime reports the graphical record as AMBIGUOUS and does not bind window services.
- **BLK-C2-02 FP/SIMD profile.** Upstream SDL3 and the G2 app require SSE and float. The kernel does not
  save XMM state (C1 static audit). Enabling SSE needs kernel FPU state, OSFXSR and OSXMMEXCPT policy,
  and the F-FP4/F-FP9 decisions. Owned by Luna's review after Phase 14. C2 does not implement it.
- **BLK-C2-03 General file access.** Files need an explicit broker interface (grant, not path). Not
  designed here. C2 provides no `fopen`. A missing broker is a stop condition, not a workaround.
- **BLK-C2-04 Ring counter wrap (ADR-0111).** Stream counters wrap at 2^32, and indexing by `% 768`
  is discontinuous at the wrap. Fix needs a wire change. Unchanged in C2.
- **BLK-C2-05 M9 `CAP_MAX` mismatch.** Tooling caps at 4, the production validator at 7. Verify,
  recommend, propose a regression test. Do not change `tools/startup_abi.py` without authorization.

## 8. Migration path for Arena 1 (summary; full version in C2-SDL3-INTEGRATION.md)

1. Replace G2 `_start` with the shared `crt0.o`.
2. Replace G2 `arenaos_memory.c` with the shared allocator through `malloc`/`free`.
3. Replace G2 OOM and log output with the granted stderr stream.
4. Replace hardcoded slot constants with a lookup through the profile layer. Keep the position
   convention only as an explicit, documented check until BLK-C2-01 is resolved.
5. Remove the F-C2-01 SSE instructions, or stop claiming the graphical app is FP-clean. Route to Luna.
6. Keep the ADSK-v1 client and all window and input code in Arena 1's backend.

## 9. Decision record

- Autonomous: directory layout, the SDK tree, the libc subset boundary, the header naming, the test
  harness design, the upstream library choice (permissive license, no FS, no FP, pinned and hashed).
- Escalated (not decided here): BLK-C2-01 through BLK-C2-05.
