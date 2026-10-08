# C2 Plan — Reusable Native C Runtime and Library Compatibility

Status: EXPERIMENTAL. Branch `arena/f89ea987-arenaos`. Base checkpoint `0350f33` (C1).
Owner role: independent C Runtime and Application Compatibility Engineer.

This plan does not implement a POSIX environment, a Linux ABI, a dynamic linker, or
SDL3. It does not change the production kernel, the startup ABI, the application
broker, or `tools/startup_abi.py`. Any proposed change to those is recorded as a
blocker or a proposal, never landed.

## Mission

Turn the C1 runtime into a reusable, independently testable, statically linkable
native C foundation. Several independent native applications, including future
graphical ones, must link one runtime without copying its low-level code.

## Checkpoints

| Id | Deliverable | Exit condition |
|----|-------------|----------------|
| C2.0 | `C2-INTEGRATION-PLAN.md`: component classification; audit of headless and graphical manifests and startup records | Document committed; every claim cites a file or a measured result |
| C2.1 | Reusable static runtime: `libarena_c.a`, `crt0.o`, public headers, GCC and Clang builds, packaging | Host and cross-compiler link of an app against the installed SDK tree |
| C2.2 | Minimal libc subset backed by existing ArenaOS services | Host semantic tests and differential checks; coverage list generated from symbols |
| C2.3 | Threads and sync bindings (C11 `threads.h`, `call_once`, TLS) | Guest test of create/join/contention/TLS isolation; unsupported calls refuse |
| C2.4 | Graphical runtime contract (Startup ABI v2, graphical profile) | Real graphical descriptor layout validated; ambiguity documented as blocker |
| C2.5 | Two independent native apps on the same SDK | App A console (guest); App B upstream library (guest, pinned and hashed) |
| C2.6 | Toolchain validation (GCC and Clang ABI agreement) | Machine-readable compatibility report generated from symbols and tests |
| C2.7 | Gates: C1 regressions, host sanitizers, guest runs | Exact PASS/FAIL receipts; repeated QEMU runs per compiler |

## Acceptance criteria (from the assignment)

1. Multiple independent native C apps link the same reusable runtime.
2. Standard library interfaces behave correctly within documented scope.
3. Apps launch through the normal signed, capability-secure lifecycle.
4. Streams, allocation, TLS, and threads remain functional.
5. Arena 1 has a concrete, tested migration path (documented; not applied to their branch).
6. All C1 host and guest regressions still pass.

## Evidence labels

- **HOST-ONLY**: host test binary (ASan + UBSan). Not kernel memory-safety proof.
- **STATIC**: checked from the ELF or disassembly without running it.
- **GUEST**: executed inside ArenaOS under QEMU with serial receipts.
- **PROPOSED**: documented, not implemented.
- **BLOCKED**: needs architectural approval.

No C1 result is reused as C2 evidence. Each C2 claim is backed by a receipt from
the C2 commit it names.

## Constraints in force

- No production kernel change, no new syscall, no new capability type, no unreviewed public ABI change.
- No implicit filesystem authority. Standard streams are not ambient filesystem authority.
- No dynamic linker, Linux ABI, POSIX namespace, or Linux system libc dependency.
- No FP/SIMD in guest code (no SSE, x87, MMX, AVX). No kernel FPU state.
- No second allocator. No fake pthread. No privileged or test-only launch path.
- The RFC 8032 development fixture is test-only and is never presented as a production key.
- Do not modify `main`, Luna's branch `arena/phase14-native-pie-aslr`, or Arena 1's branch `arena/13f9221b-arenaos`.
- Do not build SDL3. Do not modify `tools/startup_abi.py` without authorization.
- ADR-0110 remains proposed. ADR-0111 (ring counter wrap) remains an unresolved defect.

## Status at closeout (C2.8)

Source of truth for the evidence is `docs/compat/C2-FINAL-REPORT.md`. Guest results
cite source commit `7b27c67`; the receipts are in `experiments/c-runtime/receipts/`.

| Checkpoint | Status | Evidence class |
|---|---|---|
| C2.0 plan and integration audit | Done (`2d0b1cd`) | document |
| C2.1 reusable static runtime | Implemented. Headers and archive are used in place from `include/` and `build/`. No separate installed SDK tree was produced, so that part of the C2.1 exit condition is not met | static + guest |
| C2.2 libc subset | Implemented: 110 declared, 110 defined in both archives; 69 host-referenced, 48 guest-referenced, 10 refused, 5 without a test reference | host + static + guest |
| C2.3 threads and sync | Implemented as a C11 subset. Create, join, detach, mutex, trylock, condvar wait and broadcast, timed wait, `call_once`, TLS isolation (C1 T8) guest-tested. `thrd_exit`, `tss_delete` untested. Refusals documented | guest + host |
| C2.4 graphical runtime contract | **Blocked** for service identification (B-GFX-1). Count-only query host-tested. No graphical guest run | host + doc |
| C2.5 two native apps | Done: App A (nine groups) and App B (xxHash v0.8.4, unmodified). Both guest-tested on both compilers | guest |
| C2.6 toolchain validation | Done for symbol agreement, ABI constants (92, 0 mismatches), ISA audit with x87 and SSE controls. Finding: Zig clang emits x87 for double math under the no-FP flags; the audit is the gate | static + host |
| C2.7 gates | Host ASan/UBSan green (`LIBC-HOST 988/0`, `HOST-ONLY PASS 1586`). Guest suites 5/5 per compiler for C2 and C1, 3/3 crt-probe per compiler | host + guest |

Acceptance criteria:

1. Multiple independent native apps link one reusable runtime: **guest** (App A and App B
   link the same `libarena_c.a` and both pass).
2. Standard library interfaces behave correctly within documented scope: **partial**
   (host differential tests and guest tests for the listed groups; not full libc).
3. Apps launch through the normal signed lifecycle: **guest** (APB1 install and All
   Applications launch; RFC 8032 test-only signing).
4. Streams, allocation, TLS, and threads remain functional: **guest** for streams,
   allocation, and threads; TLS through the C1 native suite (T8).
5. Arena 1 has a concrete migration path: **documented** in `C2-SDL3-INTEGRATION.md`.
   It has not been applied to Arena 1's branch, and no Arena 1 build has been run
   against it.
6. C1 regressions still pass: **guest** (C1 native 5/5 per compiler; crt-probe 3/3 per
   compiler, at `7b27c67`).

## Toolchain (sandbox)

C1 used `ARENA_ZIG=/opt/zig-clang/pkg/ziglang/zig`. That path is not present in this
sandbox. C2 uses Zig 0.17.0 (clang 22) installed from PyPI to a user-owned path,
`/home/user/.arena-tools/zig/pkg/ziglang/zig`, selected with `ARENA_ZIG`. The QEMU
and Rust bootstrap follows `tools/dev-env/bootstrap.sh` (PyPI, npm, GitHub sources
only). Each C2 receipt records the toolchain versions it used.
