# Milestone G2 Remediation Plan & Dependency Analysis Report

**Document Version:** 1.0.0  
**Date:** October 8, 2026  
**Status:** In Progress / Corrective Execution  
**Author:** ArenaOS Graphics Subsystem Team  

---

## 1. Executive Summary & Architectural Review Findings

An architectural review of commit `9311dd0` identified that while the desktop integration, event pump, ADSK-v1 wire client, and demonstration application succeed on the live ArenaOS desktop, the core library linked (`libSDL3.a`) was assembled from a clean, custom API-compatible subset rather than compiling the genuine upstream SDL3 source tree (`libsdl-org/SDL` release 3.2.0).

### Key Deficiencies Identified:
1. **API Subset vs. Authentic Port:** Core modules (`SDL.c`, `SDL_video.c`, `SDL_surface.c`, `SDL_events.c`, `SDL_keyboard.c`, etc.) were authored as simplified custom implementations matching SDL3 function signatures rather than integrating ArenaOS platform hooks into the genuine upstream SDL3 source tree.
2. **Premature Qualification Claim:** Documentation in `docs/graphics/G2-FINAL-REPORT.md` claimed full qualification of upstream SDL3 without noting the distinction between the API-compatible subset and the compiled upstream codebase.
3. **Startup Record (ARST) Validation Assumptions:** `arenaos_entry.c` assumed capability slots 1, 2, and 3 held the correct endpoints without validating capability types and permissions via `SYS_CAP_DESCRIBE` or failing closed on corrupted ARST records.
4. **Scaffolding Boundaries:** Freestanding runtime glue (`arenaos_memory.c`, `arenaos_syscalls.h`) was not sufficiently isolated or labeled as temporary graphics scaffolding pending Arena 2's general C runtime.
5. **Machine Code & CPU State Audit:** Generated code was not audited for SSE2/x87 instructions or documented with respect to kernel task-switch FPU/SSE save/restore behavior.

This document establishes the precise remediation plan and comprehensive dependency analysis to port genuine upstream SDL3 to ArenaOS.

---

## 2. Genuine Upstream SDL3 Dependency Analysis

The complete, unmodified upstream source archive of SDL 3.2.0 (release tag `release-3.2.0`, commit `535d80badefc83c5c527ec5748f2a20d6a9310fe`, 16.0 MB) has been acquired into `experimental/upstream-sdl3/SDL-release-3.2.0`.

### 2.1 Upstream Built-in Private Platform Hooks
Upstream SDL3 provides official, designed extension points for in-house/private OS platforms:
- **`SDL_PLATFORM_PRIVATE 1`:** Defined in `src/SDL_internal.h` and `src/dynapi/SDL_dynapi.h`. Automatically disables DynAPI redirection (`SDL_DYNAMIC_API = 0`), ensuring all public functions are emitted with standard external linkage (e.g., `SDL_Init` instead of `SDL_Init_REAL`). Requires defining `SDL_PLATFORM_PRIVATE_NAME "ArenaOS"`.
- **`SDL_VIDEO_DRIVER_PRIVATE 1`:** Defined in `src/video/SDL_video.c:80`. Officially registers an external `VideoBootStrap PRIVATE_bootstrap` without modifying upstream source code.
- **`SDL_build_config_minimal.h`:** Provided in `include/build_config/` as the baseline configuration for embedded/freestanding targets.

### 2.2 Compilation & Linkage Dependency Inventory
Compiling genuine upstream SDL3 files under `-ffreestanding -nostdlib -Os` reveals the exact external symbol requirements across three categories:

#### Table 1: Upstream SDL3 Symbol Dependencies & Status

| Symbol | Category | Upstream Consumer | ArenaOS Resolution / Blocker |
| :--- | :--- | :--- | :--- |
| `_exit` | POSIX Process | `src/SDL.c` (`SDL_ExitProcess`) | Map directly to `SYS_THREAD_EXIT` (syscall 2) |
| `access` | POSIX VFS | `src/SDL.c` (`SDL_GetSandbox`) | Sandboxing check; stub to return `-1` (no sandbox) |
| `SDL_GetCurrentThreadID` | Threading | `src/SDL.c`, `src/events/SDL_events.c` | Provided by `src/thread/generic/SDL_systhread.c` (single-thread return 1) |
| `SDL_CreateKeymap`, `SDL_DestroyKeymap` | Event Engine | `src/events/SDL_keyboard.c` | Provided by `src/events/SDL_keymap.c` |
| `SDL_InitAudio`, `SDL_QuitAudio` | Audio Subsystem | `src/SDL.c` | Disabled via `#define SDL_AUDIO_DISABLED 1` |
| `SDL_CameraInit`, `SDL_QuitCamera` | Camera Subsystem | `src/SDL.c` | Disabled via `#define SDL_CAMERA_DISABLED 1` |
| `SDL_CleanupTrays`, `SDL_UpdateTrays` | Tray Subsystem | `src/SDL.c` | Provided by `src/tray/SDL_tray_utils.c` |
| `SDL_InitFilesystem`, `SDL_QuitFilesystem` | Filesystem | `src/SDL.c` | Provided by `src/filesystem/dummy/SDL_sysfilesystem.c` |
| `SDL_QuitCPUInfo` | CPU Info | `src/SDL.c` | Provided by `src/cpuinfo/SDL_cpuinfo.c` |
| `SDL_QuitAsyncIO` | Async I/O | `src/SDL.c` | Disabled via `#define SDL_ASYNCIO_DISABLED 1` |
| `sbrk` / `mmap` | Memory Allocator | `src/stdlib/SDL_malloc.c` (dlmalloc) | Upstream bundles dlmalloc 2.8.6; requires `MORECORE` backed by static heap or `SYS_SHARED_MAP` |
| `memcpy`, `memset`, `memmove` | Compiler Intrinsics | Upstream stdlib / GCC | Provided in `src/stdlib/` or compiler builtins |

### 2.3 The Hard Architectural Constraint: Kernel Image Budget
In `kernel/kernel/src/image_registry.rs`, the ArenaOS kernel enforces a strict hard limit on dynamic ELF images:
```rust
pub const MAX_LOAD_PAGES: usize = 128; // 512 KiB
```
Every executable loaded into user space must have its entire mapped memory image (`.text`, `.rodata`, `.data`, `.bss`, stack, and static heap) fit inside **128 pages (524,288 bytes)**.

#### Size Budget Breakdown:
- **Stack:** 64 KiB (16 pages) — minimum safe stack for complex SDL surface blitters.
- **Heap:** 128 KiB (32 pages) — minimum safe heap for SDL hashtables, properties, and window states.
- **Executable Load Budget Remaining:** `128 - 48 = 80 pages (320 KiB)`.
- **Full Upstream SDL3 Library Size:** Full upstream static library with full blit tables and properties measures ~667 KiB unstripped. When stripped and dead-code eliminated with `-Wl,--gc-sections -Os`, the upstream core (`SDL.c`, `video.c`, `surface.c`, `events.c`, `properties.c`, `hashtable.c`, `stdlib.c`) must be carefully pruned to stay within the 320 KiB load budget.

---

## 3. Remediation Action Plan

### Phase R1: Documentation Correction & Baseline Labeling
- Update `docs/graphics/G2-FINAL-REPORT.md` immediately to:
  - Clearly label the commit `9311dd0` implementation as the **"ArenaOS SDL3 Subsystem Baseline" (an API-compatible experimental prototype)**.
  - Revoke claims of a completed genuine upstream port.
  - Document the exact architectural gap between the API facade and the upstream source.
  - Formally record this document (`G2-REMEDIATION-PLAN.md`) in the graphics documentation index.

### Phase R2: Robust ARST v2 Record & Capability Validation
- Rework `arenaos_entry.c` with fail-closed defensive verification:
  1. Validate Slot 0 using `SYS_CAP_DESCRIBE` (`type == 7`, `rights & RIGHTS_READ`).
  2. Map Slot 0 and validate header magic (`0x54535241` / `ARST`) and `version == 2`.
  3. Validate Slot 1 using `SYS_CAP_DESCRIBE` (`type == 4` for Endpoint, `rights & (RIGHTS_WRITE | RIGHTS_CALL)`).
  4. Validate Slot 2 using `SYS_CAP_DESCRIBE` (`type == 7` for SharedRegion, `rights & (RIGHTS_READ | RIGHTS_WRITE)`).
  5. Validate Slot 3 using `SYS_CAP_DESCRIBE` (`type == 6` for Device).
  6. If any validation fails, emit an explicit error code and terminate immediately via `SYS_THREAD_EXIT(1)`.

### Phase R3: Authentic Upstream SDL3 Integration
- Configure genuine upstream SDL3 in `experimental/upstream-sdl3/`:
  - Create `experimental/upstream-sdl3/include/build_config/SDL_build_config_arenaos.h` defining:
    - `SDL_PLATFORM_PRIVATE 1`
    - `SDL_PLATFORM_PRIVATE_NAME "ArenaOS"`
    - `SDL_VIDEO_DRIVER_PRIVATE 1`
    - `SDL_STATIC_LIB 1`
    - Subsystem disable switches: `SDL_AUDIO_DISABLED`, `SDL_CAMERA_DISABLED`, `SDL_GPU_DISABLED`, `SDL_RENDER_DISABLED`, `SDL_JOYSTICK_DISABLED`, `SDL_HAPTIC_DISABLED`, `SDL_HIDAPI_DISABLED`, `SDL_POWER_DISABLED`, `SDL_SENSOR_DISABLED`, `SDL_DIALOG_DISABLED`, `SDL_THREADS_DISABLED`.
  - Connect the native ArenaOS video backend into `PRIVATE_bootstrap` in `src/video/arenaos/SDL_arenaosvideo.c`.
  - Compile genuine upstream SDL3 source modules into `experimental/upstream-sdl3/build/libSDL3_upstream.a`.

### Phase R4: Scaffolding Isolation & Arena 2 Coordination
- Label all freestanding runtime glue in `experimental/sdl3/src/platform/arenaos/` as **Milestone G2 Temporary Scaffolding**.
- Coordinate with Arena 2 by providing a formal C runtime requirements document specifying the exact missing symbols (`_exit`, `mmap`/`munmap`, `gettimeofday`, `abort`).
- Do not export or register a general allocator outside the experimental graphics tree.

### Phase R5: CPU State & Instruction Set Audit (SSE/x87)
- Compile all C code with `-mno-sse -mno-sse2` or audit emitted machine code to verify whether SSE/x87 instructions are generated.
- Verify whether the ArenaOS microkernel performs `fxsave`/`xsave` across thread scheduling or whether floating-point register usage in ring 3 requires kernel CPU state management.
- Document findings without introducing kernel modifications.

### Phase R6: Rigorous Automated Test Harness (`tools/test_sdl3_guest.py`)
- Enhance the QEMU test harness:
  - Cryptographically verify artifact SHA-256 digests before execution.
  - Assert that keyboard injection (Space key) triggers an actual state change observable in the guest serial stream.
  - Assert that pointer motion and button clicks produce recorded event coordinates.
  - Validate expected frame outcomes via settled screenshot pixel analysis.
  - Observe and assert clean process termination (`child Process-cap exit status=0` and capability reclamation).
  - Stop at any blocker and report verifiable evidence.

---

## 4. Immediate Execution Checkpoint

We now proceed to execute Phase R1, Phase R2, Phase R3, Phase R5, and Phase R6 autonomously. If upstream SDL3 compilation or executable-size limits encounter a blocker that cannot be resolved without architectural changes to the kernel or C runtime, execution will stop at that verifiable checkpoint and report the exact blocker.
