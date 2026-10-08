# 05 — Staged C-compatibility roadmap

Status: proposal. Stage numbers here are local to this document and are **not**
phase numbers. Phase assignment is a project decision. Phase 14 (Luna) owns
static PIE and ASLR (ADR-0108, `docs/phase13/PHASE14-HANDOFF.md`). Nothing
below changes that scope. Graphics and SDL3 video/audio belong to Arena 1.

## Stage 0 — Feasibility (done, this branch)

- **Implemented, guest-executed:** freestanding static C probe, both GCC and
  clang/LLD, `CRT-PROBE RESULT PASS (6/6)` on ArenaOS under QEMU, with the
  milestone suites unchanged (report 03).
- **Host-only:** 42-check runtime test under ASan+UBSan (report 03 §3).
- **Static:** ELF audit, ABI cross-check, SIMD/x87 check (report 02).
- **Documented:** architecture audit (report 01), strategy (report 04),
  Proposed ADR-0110.
- Exit criteria met: a C program ran on the guest with explicit exit and
  observed output. Exit status is **not** independently observed (F8).

## Stage 1 — Native C runtime v0 (proposed)

Goal: a C runtime that a native app can depend on without the shortcuts.

| Item | Existing | Missing | Evidence needed |
|---|---|---|---|
| Startup contract | `_start`, stack switch, TLS install, init_array | argv/env and a documented startup block for C (ARST v2 is Rust-only) | guest proof with argv |
| Stdio | serial via `SYS_DEBUG_WRITE` (prototype) | capability stream binding (ADR-0110 proposal) | guest proof, stream-only output, backdoor not used |
| Allocator | 4 MiB reservation, size classes, reuse, no coalescing | coalescing free runs; growth; C ABI for `realloc` growth in place | host stress; guest T4 with churn |
| Threads | none for C | C thread create/join/detach wrappers over ABI v1 thread calls; mutex and condvar over sync domains | guest two-thread proof |
| TLS | runtime-managed | decision: runtime or loader owns PT_TLS (ADR-0110 Q3) | ADR decision |
| Time | monotonic clock, sleep by yield | a non-busy sleep over timers (`SYS_TIMER_ARM`) | guest sleep proof |
| Exit | `SYS_THREAD_EXIT` | exit-status observation in the harness | serial or harness proof of the status value |
| Toolchain | GCC, clang via zig, flags frozen in `run.py` | a documented LLVM toolchain profile for workstations | build reproducibility check |

**Security risks in this stage:** `SYS_DEBUG_WRITE` must be removed from the
C contract or capability-gated (report 01, F1). Heap exhaustion is process-local
but must return typed NULL, never fault. Bad frees must be refused, not
trusted. Clang must keep the `-mno-sse` family.

**Core-OS dependencies:** ABI v1 thread and sync calls (ADR-0106, ADR-0107),
the VM limits (8 regions per process, 4,096 pages per region, 8,192 committed
pages per process), and the stream rings (ADR-0104).

**Graphics dependencies:** none.

**Phase boundary:** no ABI change lands in this stage. Each ABI proposal needs
an accepted ADR first.

## Stage 2 — Capability-brokered POSIX-subset library (option b)

Goal: source-level compatibility for programs that use a bounded set of
libc-style calls, with authority kept in brokers.

| Item | Existing | Missing |
|---|---|---|
| Files | filesd AFS2 over IPC with file caps (ADR-0076, ADR-0077) | a C file API over filesd caps; no path authority in the library |
| Time | `SYS_CLOCK_NOW`, `SYS_RTC_READ` | `clock_gettime`-style wrapper; time zones are a separate question |
| Sockets | netd Phase-13 service | a C socket API over netd endpoints; DNS and TLS brokering |
| Process | spawn with explicit grants | `posix_spawn`-style wrapper (no fork) |
| Pipes | none (byte streams are not POSIX pipes) | pipe-like wrapper over stream rings |

**Security risks:** a broker that accepts a path and grants authority by name
is an ambient-authority hole. Every broker must hold and check caps. The
library must be tested to show that a path with no granted cap fails.

**Core-OS dependencies:** filesd, netd, timers, spawn grants.

**Graphics dependencies:** none directly.

**Phase boundary:** starts only after Stage 1's threads and stdio are proven.

## Stage 3 — First large port: SDL3 (depends on Arena 1)

Goal: SDL3 built statically for ArenaOS, with a C runtime and an ArenaOS video
and audio backend.

| Item | Status | Owner |
|---|---|---|
| Static SDL3 build with no runtime loading | missing (no dynamic loader) | this program (build config) |
| Threads for SDL | Stage 1 | this program |
| File I/O backend | Stage 2 | this program |
| Video backend (display, input, presentation) | missing | **Arena 1** |
| Audio backend | missing; no C audio path identified | audio owner to confirm |
| Input backend | missing for C | Arena 1 and input owner |

**Security risks:** a large C codebase in one process; SDL's dependency on
threads and file I/O means Stage 1 and 2 bugs surface here first.

**Graphics dependencies:** the whole stage. Arena 1 owns the direction.

**Phase boundary:** SDL3 is not started in this assignment. The C runtime work
is the prerequisite.

## Stage 4 — Pure-C libraries and small native apps

Goal: validate the port path on libraries with no graphics dependency:
zlib, libpng, FreeType, libjpeg, libogg and libvorbis (dependencies listed in
STK's guide, report 04 §4.2). Each library becomes a regression test for the
runtime.

**Core-OS dependencies:** Stage 1 and 2.
**Graphics dependencies:** none for the libraries themselves.

## Stage 5 — Large applications (not scheduled)

- **SuperTuxKart:** needs a C++ runtime profile, OpenAL, sockets and TLS,
  GL or GLES (Arena 1), and threads. Long-term target.
- **Blender:** needs glibc-compatible or fully static builds, a GPU API
  (OpenGL 4.3 and/or Vulkan 1.3, per the official requirements page), an SSE4.2
  CPU baseline (conflicts with the current FP/SIMD contract, report 04 §4.3), a
  Python interpreter, and much larger memory limits. Not in any foreseeable phase.

## Cross-cutting dependencies and boundaries

- **Phase 14 (Luna):** PIE and ASLR are required before any relocated C image,
  and any loader change for PT_TLS must be coordinated with that work.
  No C-runtime work duplicates Luna's ELF loader.
- **Arena 1:** owns graphics, SDL3 video, and the GPU direction. The C runtime
  supplies no graphics API.
- **Production:** no stage here modifies production kernel or userspace code
  without an accepted ADR and a qualified guest proof.
- **Linux binary compatibility:** not in this roadmap at any stage.
- **Merge and release:** the feature branch is not merged or released by this
  work. The `main` branch is not touched.

## Open questions for the architecture review

1. Remove or capability-gate `SYS_DEBUG_WRITE` (ADR-0110 Q1)?
2. Stream capabilities through Startup ABI v2 slots, or a C-specific descriptor (ADR-0110 Q2)?
3. Runtime or loader owns PT_TLS (ADR-0110 Q3)?
4. Is a C++ profile a goal, and if so, when? (Affects Stage 5 only.)
5. Who owns the audio backend for C, given that no C audio path exists?
