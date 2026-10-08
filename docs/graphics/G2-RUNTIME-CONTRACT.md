# ArenaOS Graphics Foundation G2: C Runtime & Platform Integration Contract

- **Subsystem:** SDL3 Platform Backend & Application Compatibility Groundwork
- **Milestone:** G2 (Native SDL3 Compatibility Foundation)
- **Author:** ArenaOS Graphics Infrastructure Engineer (Track: Arena 1)
- **Cooperating Tracks:**
  - **Luna:** Phase 14 Static PIE, ASLR, and Executable Loading
  - **Arena 2:** C Runtime (libc), C Toolchain Support, Application Compatibility
  - **Arena 1 (Self):** Graphics Compatibility, SDL3 Platform Backend, Rendering
- **Date:** October 8, 2026

---

## 1. Track Boundaries & Ownership Matrix

To maintain architectural integrity and prevent duplication or divergence across ArenaOS development tracks, responsibilities are partitioned as follows:

| Subsystem / Facility | Responsible Track | Current Implementation Status | G2 Milestone Approach |
|---|---|---|---|
| **General C Runtime (libc)** | **Arena 2** | In active development (not yet upstreamed to master). | **Do NOT implement competing libc.** Use freestanding subset with narrow, graphics-specific glue. |
| **C Standard I/O (`stdio.h`, `printf`, `FILE*`)** | **Arena 2** | Deferred / in development. | Use `SYS_DEBUG_WRITE` for debug logging; disable standard file I/O in SDL3. |
| **POSIX Threads (`pthreads`)** | **Arena 2** | Deferred. | Build SDL3 with single-threaded event loop (`SDL_THREADS_DISABLED=1`). |
| **POSIX Filesystem & Sockets** | **Arena 2** | Deferred. | Disable SDL storage, filesystem, and networking. |
| **Executable Placement (Static PIE, ASLR)** | **Luna** | Phase 14 design (ADR-0108). | Comply with current strict static `ET_EXEC` image rules. |
| **Dynamic Linker (`ld-linux`, `PT_INTERP`)** | **Luna** | Not supported by kernel loader. | Fully static linking (`-static -nostdlib`). |
| **Image Size & Page Bounds** | **Luna / Core OS** | 256 KiB ELF, 128 PT_LOAD pages. | Strictly enforce via binary optimization (`-Os`, `--gc-sections`). |
| **SDL3 Platform Backend (Video, Windows)** | **Arena 1 (Self)** | **G2 Deliverable.** | Implement `SDL_VideoDevice` over `ADSK-v1` and `Client::connect_v2`. |
| **SDL3 Software Framebuffer Lifecycle** | **Arena 1 (Self)** | **G2 Deliverable.** | Direct zero-copy / bounded-copy to mapped `SharedRegion` backing. |
| **SDL3 Input Event Translation** | **Arena 1 (Self)** | **G2 Deliverable.** | Map `DesktopEvent` (Key, Pointer, Wheel, Close) to SDL event queues. |
| **Monotonic Timing & Event Backoff** | **Arena 1 (Self)** | **G2 Deliverable.** | Wire `SDL_GetTicksNS` to `SYS_CLOCK_NOW` and event sleep to `SYS_WAIT`. |

---

## 2. ABI & Executable Loading Expectations

### 2.1 ELF64 Binary Format
- **Type (`e_type`):** `ET_EXEC` (2). Static non-relocatable executable.
- **Machine (`e_machine`):** `EM_X86_64` (62).
- **Entry Point (`e_entry`):** Points to native entry symbol `_start` or `enter_user` at base `0x200000`.
- **Segments:** All `PT_LOAD` segments must be strictly page-aligned (4 KiB) and enforce W^X:
  - Text & read-only data: `PF_R | PF_X`
  - Mutable data & BSS: `PF_R | PF_W`
  - No `PF_W | PF_X` segments are permitted.
- **Size Bounds:**
  - File size: $\le 262,144$ bytes (256 KiB).
  - Memory footprint: $\le 128$ PT_LOAD pages (512 KiB).

### 2.2 Startup ABI v2 Register & Capability Contract
When the kernel spawns a native process, it initializes ring-3 registers and stack:
- **Stack:** 1 initial 4 KiB RW+NX stack page, 16-byte aligned.
- **Slot Table (128 slots total):**
  - **Slot 0:** Destroyable `SharedRegion` containing the `ARST` v2 Startup Record.
  - **Slot 1 (`SERVICE_ENDPOINT`):** Badged endpoint connected to `desktop` session broker.
  - **Slot 2 (`SURFACE`):** `SharedRegion` backing memory for primary window surface.
  - **Slot 3 (`CLOCK`):** Notification capability for timers and wakeups.
  - **Slot 4 (`DIAGNOSTICS`):** Diagnostic memory pool.
- **Startup Protocol:**
  1. Process maps Slot 0 via `SYS_SHARED_MAP(0, 0)`.
  2. Verifies `ARST` v2 magic (`0x54535241`) and version.
  3. Copies startup record to private static BSS memory.
  4. Unmaps and destroys Slot 0 (`SYS_SHARED_UNMAP`, `SYS_CAP_DESTROY`).
  5. Verifies all live capabilities match the startup manifest.

---

## 3. Required C Symbols & Narrow Graphics Glue

To compile upstream SDL3 without a full libc, the platform glue must supply the following C runtime symbols:

### 3.1 Memory Allocation
Upstream SDL3 supports custom allocators via `SDL_SetMemoryFunctions` or built-in `dlmalloc`. For Milestone G2, Arena 1 provides a bounded heap allocator over a dedicated 16-page (`64 KiB`) static BSS pool:
- `void *malloc(size_t size)`: 8-byte aligned allocation with size header.
- `void free(void *ptr)`: Returns block to free list.
- `void *calloc(size_t nmemb, size_t size)`: Zero-initialized allocation.
- `void *realloc(void *ptr, size_t size)`: Resized allocation with data preservation.

*Handoff to Arena 2:* In Milestone G3/Phase 14, these stubs will be superseded by Arena 2's `ScalableHeap`-backed libc allocator.

### 3.2 Memory Manipulation & String Intrinsics
Freestanding implementations compiled with `-fno-builtin`:
- `void *memcpy(void *dest, const void *src, size_t n)`
- `void *memset(void *s, int c, size_t n)`
- `void *memmove(void *dest, const void *src, size_t n)`
- `int memcmp(const void *s1, const void *s2, size_t n)`
- `size_t strlen(const char *s)`
- `int strcmp(const char *s1, const char *s2)`
- `int strncmp(const char *s1, const char *s2, size_t n)`

### 3.3 Math Routines
Upstream SDL3 includes its own portable math library (`src/stdlib/SDL_stdlib.c` with uClibc algorithms) when `HAVE_LIBC` is not defined. Only compiler floating-point helper intrinsics are required:
- `sinf`, `cosf`, `sqrtf`, `floorf`, `ceilf`, `fabsf` (provided by GCC builtins or SDL3 internal uClibc math).

---

## 4. SDL3 Subsystem Feasibility Matrix for ArenaOS

| SDL3 Subsystem | Supported in G2 | Implementation Path / Dependency |
|---|---|---|
| **Video (`SDL_video.h`)** | **YES** | Native ArenaOS video backend (`SDL_arenaosvideo.c`). |
| **Window Surface (`SDL_surface.h`)** | **YES** | Zero-copy / mapped surface in `SharedRegion` backing. |
| **Events (`SDL_events.h`)** | **YES** | Desktop IPC polling (`window.poll()`) and translation. |
| **Keyboard Input (`SDL_keyboard.h`)** | **YES** | Desktop `Event::Key` to `SDL_SendKeyboardKey`. |
| **Mouse Input (`SDL_mouse.h`)** | **YES** | Desktop `Event::Pointer` and `Event::Wheel` translation. |
| **Timers & Clocks (`SDL_timer.h`)** | **YES** | Microsecond time from `SYS_CLOCK_NOW`, delay via `SYS_WAIT`. |
| **Software Renderer (`SDL_render.h`)** | **YES** | Upstream SDL software 2D renderer over window surface. |
| **Audio (`SDL_audio.h`)** | **NO** | Disabled (`-DSDL_AUDIO=OFF`). ArenaOS has no audio service. |
| **GPU / Vulkan (`SDL_gpu.h`)** | **NO** | Disabled (`-DSDL_GPU=OFF`). Requires Milestone G4 Virtio-GPU. |
| **OpenGL / EGL** | **NO** | Disabled (`-DSDL_OPENGL=OFF`). Requires Mesa port (Milestone G5). |
| **Joystick / Gamepad** | **NO** | Disabled (`-DSDL_JOYSTICK=OFF`). No game controller service. |
| **Filesystem / Storage** | **NO** | Disabled (`-DSDL_FILESYSTEM=OFF`). Blocked on Arena 2 libc VFS. |
| **Dialogs (File Choosers)** | **NO** | Disabled. ArenaOS uses capability-based file grants. |
| **Process Control** | **NO** | Disabled (`-DSDL_PROCESS=OFF`). Blocked on Arena 2 process ABI. |
| **Multi-threading** | **NO** | Disabled (`-DSDL_THREADS=OFF`). Blocked on Arena 2 pthreads. |
