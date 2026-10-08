# ArenaOS Graphics Milestone G2: Native SDL3 Compatibility Foundation Report

**Date:** October 8, 2026  
**Milestone:** G2 (Native SDL3 Compatibility Foundation)  
**Status:** CHECKPOINT / REMEDIATION IN PROGRESS  
**Baseline Status:** Experimental API-Compatible Subset Verified in QEMU Guest  
**Upstream Port Status:** Genuine Upstream SDL 3.2.0 Port Underway (See `docs/graphics/G2-REMEDIATION-PLAN.md`)  
**Target Environment:** ArenaOS x86_64 UEFI Guest (Q35, OVMF, VirtIO, Desktop Broker, ADSK-v1 Protocol)  

---

## 1. Executive Summary & Status Classification

Milestone G2 aims to introduce native Simple DirectMedia Layer 3 (SDL3) application capabilities into ArenaOS ring-3 userspace.

### Critical Status Classification:
1. **Experimental API-Compatible Demonstration (Verified Baseline):**  
   Commit `9311dd0` established a functional ring-3 demonstration application (`experimental/sdl3-app`) running inside the ArenaOS desktop. This demonstration validated the `ADSK-v1` window protocol, software framebuffer mapping via Slot 2 shared memory, input event dispatching, and APB1 package signing. However, the supporting library was constructed from an **API-compatible subset** (`SDL.c`, `SDL_video.c`, `SDL_surface.c`, `SDL_events.c`) rather than linking against the genuine compiled upstream SDL3 source tree.
2. **Authentic Upstream SDL3 Port (Active Remediation):**  
   A comprehensive remediation plan (`docs/graphics/G2-REMEDIATION-PLAN.md`) is now actively being executed to port the genuine upstream SDL3 source tree (`libsdl-org/SDL` release tag `release-3.2.0`, commit `535d80badefc83c5c527ec5748f2a20d6a9310fe`). This involves integrating the native ArenaOS video backend into upstream's `PRIVATE_bootstrap` interface, providing minimal freestanding runtime scaffolding, and validating binary constraints.

Until the genuine upstream SDL3 library compiles and executes in the ArenaOS guest without substitute implementations, G2 is formally classified as **in remediation**.

---

## 2. Track Ownership & Architectural Boundaries

Milestone G2 strictly preserves the architectural ownership established across parallel tracks:
- **Phase 14 Ownership (Luna):** Luna owns Static PIE, ASLR, and general dynamic ELF loading contracts. Milestone G2 produces static ELF executables (`-fno-pie -no-pie -static`) compatible with the existing Phase-13 image loader, introducing zero kernel changes.
- **C Runtime Track Ownership (Arena 2):** Arena 2 owns the general userspace C library (libc, pthreads, toolchains). Milestone G2 does not ship an independent competing libc; runtime glue is isolated strictly within `experimental/sdl3/src/platform/arenaos/` and explicitly labeled as temporary scaffolding.
- **Graphics Track Ownership (Arena 1):** Focuses exclusively on the graphics subsystem, framebuffer presentation, event translation, timing, and upstream SDL3 platform backend interfaces.
- **Core Isolation:** Zero changes made to production kernel (`kernel/`), compositor (`userspace/compositord`), or desktop (`userspace/desktop`). All code is isolated under `experimental/`, `docs/graphics/`, and `tools/`.

---

## 3. Architecture of the Verified Baseline Demonstration

The experimental baseline demonstrates the end-to-end viability of the SDL3 programming model on ArenaOS:

```
+-------------------------------------------------------------------------+
|                       Native SDL3 Application                           |
|      (experimental/sdl3-app/src/main.c - Standard Public SDL3 APIs)     |
+-------------------------------------------------------------------------+
                                    |
                                    v
+-------------------------------------------------------------------------+
|                    libSDL3.a (API-Compatible Subset)                    |
|  +------------------+  +------------------+  +------------------------+ |
|  | SDL Core / Video |  |   SDL Events     |  |   SDL Surface/Pixels   | |
|  | (SDL.c, video.c) |  | (events, mouse)  |  | (surface.c, pixels.c)  | |
|  +------------------+  +------------------+  +------------------------+ |
|  +--------------------------------------------------------------------+ |
|  |                    ArenaOS SDL3 Video Driver                       | |
|  | (SDL_arenaosvideo.c, SDL_arenaosframebuffer.c, SDL_arenaosevents.c)| |
|  +--------------------------------------------------------------------+ |
|  +--------------------------------------------------------------------+ |
|  |               Freestanding Temporary Scaffolding                   | |
|  | (arenaos_entry.c, arenaos_memory.c, arenaos_client.c, syscalls.h) | |
|  +--------------------------------------------------------------------+ |
+-------------------------------------------------------------------------+
         | (SYS_SHARED_MAP)                 | (SYS_IPC_CALL)
         v                                  v
+------------------+             +----------------------+
| SharedRegion Shm |             | Desktop Broker Server|
| (Slot 2 Surface) | <========== | (Slot 1 IPC Endpoint)|
+------------------+  ADSK-v1    +----------------------+
```

### 3.1 Verification & Performance Metrics of the Baseline
- **Application Binary:** `experimental/sdl3-app/build/sdl3_app` (36,704 bytes, SHA-256 `b17ed95995cea6cbea9adadbd7b1322c211285aafb27c4535226654878ef93dc`)
- **Signed Package:** `experimental/sdl3-app/SDL3App.apb1` (37,488 bytes, SHA-256 `7402ffc06dcf824a3535fad51435211c9315f12f57e10ae45e1ab0a788bfa18c`)
- **Execution Summary:** 60 frames rendered in 1153 ms (~52.0 FPS) on QEMU guest desktop.
- **Timing Breakdown:** Average rasterization 767.4 us/frame; average IPC damage presentation 7355.2 us/frame.
- **Visual Artifact:** Verified screenshot `experimental/sdl3/guest_sdl3_desktop.png` confirming dark slate window (`#1A1A2E`), subheader banner, bouncing box, cursor tracker, and progress bar.
- **Teardown:** Verified zero leaks (`child Process-cap exit status=0`, final process group members=0).

---

## 4. Remediation Progress & Upstream Port Status

Work is progressing according to `docs/graphics/G2-REMEDIATION-PLAN.md`:

1. **Upstream Source Acquired:** Official release `release-3.2.0` (commit `535d80badefc83c5c527ec5748f2a20d6a9310fe`) downloaded into `experimental/upstream-sdl3/SDL-release-3.2.0`.
2. **Platform Configuration:** Configured using upstream private platform flags (`SDL_PLATFORM_PRIVATE 1`, `SDL_VIDEO_DRIVER_PRIVATE 1`).
3. **Core Compilation:** Successfully compiled upstream `SDL.c`, `src/video/SDL_video.c`, `src/video/SDL_surface.c`, `src/events/SDL_events.c`, `src/stdlib/SDL_malloc.c`, and supporting modules under freestanding GCC flags.
4. **Current Focus:** Resolving minimal libc symbol dependencies (`_exit`, `access`, keymap utilities) and validating image size against the kernel's `MAX_LOAD_PAGES = 128` (512 KiB) limit.
