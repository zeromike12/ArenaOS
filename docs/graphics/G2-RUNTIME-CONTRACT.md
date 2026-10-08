# ArenaOS Graphics Foundation G2: C Runtime & Platform Integration Contract

- **Subsystem:** SDL3 Platform Backend & Application Compatibility Groundwork
- **Milestone:** G2 (Native SDL3 Compatibility Foundation)
- **Author:** ArenaOS Graphics Infrastructure Engineer (Track: Arena 1)
- **Cooperating Tracks:**
  - **Luna:** Phase 14 Static PIE, ASLR, and Executable Loading
  - **Arena 2:** C Runtime (libc), C Toolchain Support, Application Compatibility
  - **Arena 1 (Self):** Graphics Compatibility, SDL3 Platform Backend, Rendering
- **Date:** October 8, 2026
- **Status:** Checkpoint Report & Blocker Inventory

---

## 1. Track Boundaries & Ownership Matrix

To maintain architectural integrity and prevent duplication or divergence across ArenaOS development tracks, responsibilities are partitioned as follows:

| Subsystem / Facility | Responsible Track | Current Implementation Status | G2 Milestone Approach |
|---|---|---|---|
| **General C Runtime (libc)** | **Arena 2** | In active development (not yet upstreamed to master). | **Do NOT implement competing libc.** Use freestanding subset with narrow, explicitly labeled scaffolding. |
| **C Standard I/O (`stdio.h`, `printf`, `FILE*`)** | **Arena 2** | Deferred / in development. | Use `SYS_DEBUG_WRITE` for debug logging; disable standard file I/O in SDL3. |
| **POSIX Threads (`pthreads`)** | **Arena 2** | Deferred. | Build SDL3 with single-threaded event loop (`SDL_THREADS_DISABLED=1`). |
| **POSIX Filesystem & Sockets** | **Arena 2** | Deferred. | Disable SDL storage, filesystem, and networking. |
| **Memory Allocator (`malloc`/`mmap`)** | **Arena 2** | Required by upstream `dlmalloc`. | Temporary bounded 128 KiB heap scaffolding strictly for G2 graphics experimentation. |
| **Executable Placement (Static PIE, ASLR)** | **Luna** | Phase 14 design (ADR-0108). | Comply with current strict static `ET_EXEC` image rules. |
| **CPU State Management (SSE/x87/FPU)** | **Luna / Kernel** | Integer-only kernel (ADR-0012). | Document ring-3 SSE constraints; compile C code with `-mno-sse` where possible. |
| **Dynamic Linker (`ld-linux`, `PT_INTERP`)** | **Luna** | Not supported by kernel loader. | Fully static linking (`-static -nostdlib`). |
| **Image Size & Page Bounds** | **Luna / Core OS** | 256 KiB ELF, 128 PT_LOAD pages. | Document upstream SDL3 static size overflow against `MAX_LOAD_PAGES=128`. |
| **SDL3 Platform Backend (Video, Windows)** | **Arena 1 (Self)** | **G2 Deliverable.** | Implement `SDL_VideoDevice` over `ADSK-v1` and `Client::connect_v2`. |
| **SDL3 Software Framebuffer Lifecycle** | **Arena 1 (Self)** | **G2 Deliverable.** | Direct zero-copy / bounded-copy to mapped `SharedRegion` backing. |
| **SDL3 Input Event Translation** | **Arena 1 (Self)** | **G2 Deliverable.** | Map `DesktopEvent` (Key, Pointer, Wheel, Close) to SDL event queues. |
| **Monotonic Timing & Event Backoff** | **Arena 1 (Self)** | **G2 Deliverable.** | Wire `SDL_GetTicksNS` to `SYS_CLOCK_NOW` and event sleep to `SYS_WAIT`. |

---

## 2. ABI, Memory Bounds, and Startup Validation

### 2.1 ELF64 Binary Format & Kernel Page Limits
- **Type (`e_type`):** `ET_EXEC` (2). Static non-relocatable executable.
- **Machine (`e_machine`):** `EM_X86_64` (62).
- **Entry Point (`e_entry`):** Points to native entry symbol `_start` at base `0x200000`.
- **Segments:** All `PT_LOAD` segments must be strictly page-aligned (4 KiB) and enforce W^X:
  - Text & read-only data: `PF_R | PF_X`
  - Mutable data & BSS: `PF_R | PF_W`
  - No `PF_W | PF_X` segments are permitted.
- **Kernel Load Budget (`image_registry.rs`):**
  ```rust
  pub const MAX_LOAD_PAGES: usize = 128; // 512 KiB total mapped image budget
  ```
  Every executable image loaded into user space must have its entire memory footprint (`.text`, `.rodata`, `.data`, `.bss`, stack, and heap) fit strictly inside 128 pages.

### 2.2 Fail-Closed Startup (ARST v2) Validation Contract
When the kernel spawns a native process, it provisions initial capabilities:
- **Slot 0:** Destroyable `SharedRegion` containing the `ARST` v2 Startup Record.
- **Slot 1 (`SERVICE_ENDPOINT`):** Badged endpoint connected to `desktop` session broker.
- **Slot 2 (`SURFACE`):** `SharedRegion` backing memory for primary window surface.
- **Slot 3 (`CLOCK`):** Notification capability for timers and wakeups.

#### Defensive Verification Rules (Implemented in `arenaos_entry.c`):
1. **Slot 0 Capability Inspection:** Invoke `SYS_CAP_DESCRIBE(0)`. Fail closed if `status != 0`, `kind != 7` (`SharedRegion`), or `rights & RIGHTS_READ == 0`.
2. **Slot 0 Mapping:** Map via `SYS_SHARED_MAP(0, 0)`. Fail closed if mapping returns non-positive address.
3. **ARST Header Verification:** Read header. Fail closed if `magic != 0x54535241` (`ARST`) or `version != 2`.
4. **Transport Capability Release:** Copy 4096-byte record into private BSS, unmap via `SYS_SHARED_UNMAP`, and destroy capability via `SYS_CAP_DESTROY(0)`.
5. **Slot 1 Validation:** Describe Slot 1. Fail closed if `status != 0`, `kind != 12` (`BadgedEndpoint`), or `(rights & RIGHTS_WRITE) == 0`.
6. **Slot 2 Validation:** Describe Slot 2. Fail closed if `status != 0`, `kind != 7` (`SharedRegion`), or `(rights & (RIGHTS_READ | RIGHTS_WRITE)) != 3`.
7. **Slot 3 Validation:** Describe Slot 3. Fail closed if `status != 0` or `kind != 3` (`Clock`).

---

## 3. CPU State & Hardware Register Audit (SSE / x87 / FPU)

An audit of machine code generation and kernel task-switching architecture reveals a critical dependency:

### 3.1 Kernel State Management (ADR-0012)
`kernel/kernel/src/arch/x86_64/context.rs` explicitly documents the kernel thread switch frame:
```
The switch frame is exactly the Win64 callee-saved set plus RFLAGS:
`rbx, rbp, rdi, rsi, r12–r15` (8 × 8 B) + `pushfq` (8 B) + return address (8 B) = 80 bytes.
No FPU/SSE state is saved — the kernel image contains zero FPU/SSE/MMX instructions
(soft-float target, integer-only code) and tools/build.sh fails the build if that
invariant ever breaks (ADR-0012).
```

### 3.2 Ring-3 Implications for SDL3
1. **Upstream SDL3 Float APIs:** Genuine upstream SDL3 relies on standard IEEE 754 floating-point types (`float`, `double`) across public APIs (display scale, SDR white point, colorspace math, audio resampling).
2. **AMD64 System V ABI:** Mandates passing and returning floating-point values in `%xmm0..%xmm7`.
3. **Preemption Hazard:** Because the microkernel does not execute `fxsave`/`xsave` across context switches, multiple ring-3 threads or processes using SSE registers risk state corruption across preemptive switches.
4. **Baseline Scaffolding Posture:** To guarantee 100% deterministic execution on the current integer-only microkernel, the Milestone G2 demonstration binary is compiled with `-mno-sse -mno-sse2` and uses integer fixed-point arithmetic for animation and timing.
5. **Requirement for Luna / Core Track:** Multi-threaded or full upstream SDL3 execution will require Luna's architecture to incorporate optional user FPU/SSE context switching (`fxsave`/`fxrstor`) when ring-3 processes execute floating-point operations.

---

## 4. Genuine Upstream SDL 3.2.0 Blocker Inventory

Compiling genuine upstream SDL 3.2.0 (`experimental/upstream-sdl3/SDL-release-3.2.0`) under freestanding GCC identifies the following concrete blockers:

### 4.1 Missing C Runtime / POSIX Symbols

| Missing Symbol | Upstream File | Required Functionality | Arena 2 Resolution Path |
| :--- | :--- | :--- | :--- |
| `mmap` / `munmap` | `src/stdlib/SDL_malloc.c` | Page-level allocation for dlmalloc 2.8.6 | Translate to `SYS_SHARED_PAGES` / `SYS_SHARED_MAP` |
| `mremap` / `sbrk` | `src/stdlib/SDL_malloc.c` | Heap resizing | Emulate in libc over virtual memory regions |
| `sched_yield` | `src/stdlib/SDL_malloc.c` | Spinlock backoff | Map to `SYS_WAIT` (timeout=0) |
| `sysconf` | `src/stdlib/SDL_malloc.c` | Query system page size | Static return `4096` in libc |
| `__errno_location` | `src/stdlib/SDL_malloc.c` | Thread-local error indicator | Provide TLS-backed errno |
| `time` | `src/stdlib/SDL_malloc.c` | Entropy initialization | Wire to `SYS_RTC_READ` (syscall 50) |
| `_exit` | `src/SDL.c` | Process termination | Wire to `SYS_THREAD_EXIT` (syscall 2) |
| `access` | `src/SDL.c` | VFS sandbox detection | Return `-1` (no sandbox) |
| `fopen`/`fscanf`/`fclose` | `src/cpuinfo/SDL_cpuinfo.c` | CPU cache size probe from `/sys` | Provide standard POSIX VFS over `filesd` |

### 4.2 Executable Size Budget Overflow
The genuine upstream SDL3 library archive (`libSDL3_upstream_full.a`) is **713 KiB** unstripped.
Even with `-Os -Wl,--gc-sections`, a minimal application linked against genuine upstream SDL3 exceeds the kernel's `MAX_LOAD_PAGES = 128` (512 KiB) total memory ceiling when combined with the required stack (64 KiB) and heap (128 KiB).

---

## 5. Architectural Recommendations & Track Handoff

1. **Arena 2 (C Runtime):**
   - Provide minimal POSIX libc support supplying `_exit`, `mmap`, `munmap`, and thread-local `errno`.
   - Implement `dlmalloc` backing over ArenaOS capability-based shared memory pages (`SYS_SHARED_PAGES`).
2. **Luna (Phase 14 Loader):**
   - Evaluate expanding `MAX_LOAD_PAGES` beyond 128 pages or providing dynamic on-demand paging for larger C/C++ runtimes.
   - Introduce ring-3 FPU/SSE register save/restore (`fxsave`/`fxrstor`) on thread context switches to support standard floating-point System V AMD64 ABI applications.
3. **Arena 1 (Graphics):**
   - Maintain the native ArenaOS video backend (`PRIVATE_bootstrap`) ready to plug directly into upstream SDL3 once the C runtime and page budget prerequisites land.
