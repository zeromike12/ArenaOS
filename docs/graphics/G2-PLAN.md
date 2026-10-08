# ArenaOS Graphics Foundation G2: Native SDL3 Compatibility Plan

- **Subsystem:** Graphics Compatibility, SDL3 Integration & Rendering Infrastructure
- **Milestone:** G2 (Native SDL3 Compatibility Foundation)
- **Author:** ArenaOS Graphics Infrastructure Engineer (Track: Arena 1)
- **Cooperating Tracks:**
  - **Luna:** Phase 14 Static PIE, ASLR, and Executable Loading
  - **Arena 2:** C Runtime (libc), C Toolchain Support, Application Compatibility
  - **Arena 1 (Self):** Graphics Compatibility, SDL3 Platform Backend, Rendering
- **Branch:** `arena/13f9221b-arenaos`
- **Base Commit:** `1b48882cfce4027c252ea0f1f8850f1a3a0202ab` (Verified G1.1 Checkpoint)
- **Date:** October 8, 2026

---

## 1. Milestone Goals & Scope

The principal objective of Milestone G2 is to establish the first authentic SDL3 compatibility foundation for ArenaOS. This is a functional software compatibility milestone enabling genuine C applications written against standard upstream SDL3 APIs to compile, link, and execute within the capability-secure ArenaOS desktop environment.

### 1.1 Non-Goals & Invariant Constraints
- **No Competing General C Runtime:** Arena 2 owns general libc/CRT development. Arena 1 will implement only the minimal graphics-specific runtime glue necessary for SDL3 platform backend operation, fully documented in `docs/graphics/G2-RUNTIME-CONTRACT.md`.
- **No Production Code Modifications:** All work is confined to `experimental/sdl3/`, `docs/graphics/`, and `tools/`. Zero changes to `kernel/`, `userspace/desktop/src/bin/desktop.rs`, `displayd`, `compositord`, or existing public wire ABIs.
- **No ADSK-v2 Implementation:** Desktop presentation uses the qualified Phase-13 `ADSK-v1` protocol and `Client::connect_v2` SharedRegion facilities. ADSK-v2 remains a proposal.
- **No Facade or Imitation Library:** We will integrate genuine upstream SDL3 source code and headers (pinned release 3.2.0 / 3.2.24).
- **Enforcement of System Invariants:** W^X, non-executable stack, static `ET_EXEC` ELF rules, APB1 signature verification, and capability security will not be bypassed or weakened.

---

## 2. Work Breakdown & Sequencing

### Phase G2.0: Feasibility Assessment & Runtime Dependency Contract
1. Audit existing Phase-13/Phase-14 desktop IPC, capability slots, and memory bounds:
   - Dynamic Image executable bound: 256 KiB (`262,144` bytes).
   - PT_LOAD segment bound: 128 pages (`524,288` bytes).
   - Startup contract: Native Startup ABI v2 (`ARST` v2) in slot 0.
   - Window presentation: `SharedRegion` backing mapped via `SYS_SHARED_MAP`, 4096-byte control offset, synchronous `damage()` IPC.
2. Establish pinned upstream SDL3 source baseline:
   - Upstream Release: SDL 3.2.0 / 3.2.24 (Git revision `release-3.2.24-0-ga8589a842`, zlib license).
   - Source headers extracted from official upstream release archives (`kivy-deps.sdl3-dev` PyPI distribution).
3. Produce `docs/graphics/G2-RUNTIME-CONTRACT.md`:
   - Categorize every SDL3 subsystem as: Available Today, Requires Minimal Graphics Glue, or Blocked on Arena 2 / Luna.
   - Define exact C symbol requirements (`malloc`, `free`, `memcpy`, `memset`, math intrinsics).

### Phase G2.1: Experimental ArenaOS SDL3 Platform Backend
Implement the smallest viable set of actual SDL3 video and input driver interfaces:
1. **Video Driver (`SDL_arenaosvideo.c` / `SDL_arenaosvideo.h`):**
   - Video bootstrap entry (`ARENAOS_bootstrap`) and device creation (`ARENAOS_CreateDevice`).
   - Video initialization (`VideoInit`) and shutdown (`VideoQuit`).
   - Window creation (`CreateSDLWindow`), destruction (`DestroyWindow`), and titles (`SetWindowTitle`).
2. **Framebuffer Lifecycle (`SDL_arenaosframebuffer.c`):**
   - Framebuffer allocation (`CreateWindowFramebuffer`): Map ArenaOS `SharedRegion` backing surface or allocate staging surface.
   - Presentation (`UpdateWindowFramebuffer`): Translate damage rects and publish through ArenaOS desktop IPC.
   - Teardown (`DestroyWindowFramebuffer`): Cleanly unmap surface and release window resources.
3. **Event & Input Routing (`SDL_arenaosevents.c`):**
   - Event pump (`PumpEvents`): Poll ArenaOS desktop event stream (`window.poll()`).
   - Translate `DesktopEvent::Key` to `SDL_SendKeyboardKey`.
   - Translate `DesktopEvent::Pointer` to `SDL_SendMouseMotion` and `SDL_SendMouseButton`.
   - Translate `DesktopEvent::Wheel` to `SDL_SendMouseWheel`.
   - Translate `DesktopEvent::Close` to `SDL_SendWindowEvent(window, SDL_EVENT_WINDOW_CLOSE_REQUESTED)`.
   - Translate `DesktopEvent::Focus` to `SDL_SendWindowEvent(window, SDL_EVENT_WINDOW_FOCUS_GAINED/LOST)`.
4. **Timing & Clocks (`SDL_arenaostime.c`):**
   - Monotonic nanosecond/microsecond time via ArenaOS `SYS_CLOCK_NOW`.
   - Bounded idle waiting via `SYS_WAIT` on the private clock notification capability.

### Phase G2.2: Authentic Upstream SDL3 Library Integration
1. Extract official upstream SDL3 headers and core sources into `experimental/sdl3/`.
2. Configure build system with minimal footprint:
   - Core subsystems enabled: `SDL_VIDEO`, `SDL_EVENTS`, `SDL_RENDER` (software), `SDL_TIMERS`.
   - Heavy/unsupported subsystems disabled: `SDL_AUDIO=0`, `SDL_GPU=0`, `SDL_VULKAN=0`, `SDL_OPENGL=0`, `SDL_JOYSTICK=0`, `SDL_HAPTIC=0`, `SDL_HIDAPI=0`, `SDL_CAMERA=0`, `SDL_DIALOG=0`, `SDL_PROCESS=0`.
3. Provide narrow graphics-specific runtime glue (`arenaos_glue.c` / `arenaos_bridge.rs`) bridging Startup ABI v2 and system calls.
4. Measure and enforce the 256 KiB executable size constraint.

### Phase G2.3: Genuine Third-Party SDL3 C Application Demonstration
1. Write a clean, authentic C application (`experimental/sdl3-app/src/main.c`) using purely standard public SDL3 APIs:
   - `SDL_Init(SDL_INIT_VIDEO)`
   - `SDL_CreateWindow("SDL3 ArenaOS Demo", 320, 240, 0)`
   - `SDL_GetWindowSurface(window)`
   - Software drawing: Moving animated color palette / interactive shape drawing.
   - Event loop: `SDL_PollEvent(&event)`.
   - Respond to keyboard keypress (e.g. spacebar / arrow keys changes color/position) and mouse motion/click.
   - `SDL_UpdateWindowSurface(window)` to present.
   - Clean exit on `SDL_EVENT_WINDOW_CLOSE_REQUESTED` or `SDL_EVENT_QUIT`.
   - `SDL_DestroyWindow(window)` and `SDL_Quit()`.
2. Package as signed APB1 bundle (`SDL3Demo.apb1`) using RFC 8032 test seed.
3. Verify installation and desktop execution.

### Phase G2.4: Verification, Qualification & Reporting
1. Host unit/integration test suite:
   - Upstream header and version integrity verification.
   - Video backend device creation, surface allocation, and destroy tests.
   - Event translation unit tests.
2. QEMU guest end-to-end integration test (`tools/test_sdl3_guest.py`):
   - Boot qualified Phase-13 desktop.
   - Install `SDL3Demo.apb1` via pointer double-click.
   - Launch via All Applications menu.
   - Inject genuine virtual keyboard and mouse input events via QMP.
   - Verify application receives events and mutates state.
   - Capture QMP screendump and assert visual pixel output.
   - Verify clean process exit status 0 and kernel shutdown.
3. Produce `docs/graphics/G2-FINAL-REPORT.md`.
