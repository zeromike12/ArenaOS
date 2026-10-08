# ArenaOS Graphics Compatibility Roadmap

## 1. Overview & Long-Term Benchmarks

Our long-term compatibility targets are:
- **SuperTuxKart:** Modern 3D open-source racing game requiring OpenGL 3.3 / GLES 3.0, SDL2/SDL3 windowing, OpenAL audio, physics engine, and complex asset loading.
- **Blender:** Professional 3D creation suite requiring OpenGL 4.3+ or Vulkan 1.2+, multi-threaded job scheduling, extensive C++ runtime, Python scripting, and hundreds of megabytes of memory.

Neither application can run in the immediate future without substantial operating system infrastructure. This roadmap defines a practical, milestone-driven progression toward that goal.

---

## 2. First Graphics Compatibility Benchmark: `glxgears` / `testgl`

To validate graphics infrastructure early without waiting for complex game engines, we establish **`glxgears`** (or SDL3's reference **`testgl` / `glgears`**) as our **first graphics compatibility benchmark**.

### Benchmark Profile: `glxgears` / `testgl`
- **Origin & Codebase:** Classic OpenGL demonstration (Mesa / X.org / SDL test suite).
- **Source Code Availability:** Open source (MIT / zlib license), ~500 lines of standard C.
- **Rendering Workload:**
  - Real 3D geometric meshes (three interlocking involute gears).
  - Depth buffer testing (Z-buffer) with hidden-surface removal.
  - Directional lighting and Gouraud shading.
  - Continuous animated matrix rotation.
  - Precise frame rate reporting.
- **Visual Behavior:** Three colored gears (red, green, blue) rotating in mesh contact against a dark background. Immediate visual proof of depth testing, matrix transformation, lighting, and animation.
- **Realistic Execution Target:** Requires minimal C runtime shims and software OpenGL; achievable well before SuperTuxKart.

---

## 3. Gap Analysis: What is Missing to Run `glxgears` on ArenaOS

The table below breaks down every technical requirement to run the benchmark, cleanly delineating the responsibilities of the **Graphics Infrastructure Engineer** from the **Core Operating System Engineer**:

| Requirement Subsystem | Exact Current Gap in ArenaOS | Owner: Graphics Engineer | Owner: Core OS Engineer |
|---|---|---|---|
| **Binary Executable Loading** | Kernel ELF loader only admits static `ET_EXEC` (`docs/adr/0108`). No static PIE or base address relocation. | None (consumers of the binary format). | **Core OS Engineer (Phase 14):** Implement validated static `ET_DYN` with `R_X86_64_RELATIVE` relocations. |
| **C Runtime Environment** | No standard C library. ArenaOS uses pure Rust `no_std`. Custom syscall ABI in `abi.rs`. | Implement minimalist C runtime shims for graphics (`malloc`, `free`, `memcpy`, `memset`, `sin`, `cos`, `sqrt`). | **Core OS Engineer:** Provide standard syscall stubs for memory expansion (`sbrk`/`mmap` via `SYS_VM_RESERVE`). |
| **Windowing & Events** | No X11 or Wayland display protocol. ArenaOS uses proprietary `ADSK` IPC frames. | **Graphics Engineer:** Implement ArenaOS SDL3 video/input driver (`src/video/arenaos/`). | None. |
| **Graphics API Implementation** | No `libGL.so` or OpenGL runtime dispatch in ArenaOS. | **Graphics Engineer:** Integrate lightweight software OpenGL (TinyGL / Mesa swrast subset) outputting to `SharedRegion`. | None. |
| **High-Resolution Timing** | Need microsecond timestamps for animation pacing and FPS calculation. | Connect `SDL_GetTicksNS` to ArenaOS `SYS_CLOCK_NOW`. | None. |
| **Filesystem Access** | Benchmark is self-contained (compiled-in gear geometry); requires zero disk access. | None. | None for this benchmark (AFS2 is already qualified for future assets). |
| **Audio Subsystem** | Benchmark produces no sound. | Stub SDL audio with dummy backend (`SDL_AUDIODEV_DUMMY`). | None for this benchmark. |
| **GPU Acceleration** | Benchmark can run on CPU software rasterizer at 60+ FPS at 640×480. | None required for first benchmark. | None. |

---

## 4. Phased Engineering Milestones

### Milestone G1: Native Software Rendering Substrate (Current Assignment)
- **Deliverables:**
  - Complete architecture audit of Phase-13 graphics and capability boundaries.
  - Proposed `ADSK-v2` graphics interface and security model.
  - Isolated working prototype of 3D software rendering pipeline (textured rotating cube, early-Z, lighting, deterministic PPM output).
  - Automated test suite validating buffer bounds, stride isolation, and depth order-independence.
- **OS Dependency:** 100% independent. Uses existing Phase-13 Ring 3 capabilities.

### Milestone G2: ArenaOS SDL3 Video Backend & Lightweight OpenGL
- **Objectives:**
  - Build isolated SDL3 port with `SDL_VideoDevice` targeting ArenaOS `SharedRegion` and `Desktop` IPC.
  - Embed lightweight software OpenGL (TinyGL / standalone software rasterizer).
  - Execute **`glxgears` / `testgl`** inside ArenaOS userspace as the first external graphics compatibility proof.
- **OS Dependency:** Requires Phase 14 static PIE from Core OS Engineer.

### Milestone G3: Intermediate 3D Gaming Benchmark (`Neverball`)
- **Objectives:**
  - Run **Neverball** (open-source 3D physics/ball game using SDL and OpenGL 1.4/2.0).
  - Add basic asset loading via AFS2 filesystem capabilities (`filesd`).
  - Introduce multi-threaded software rasterization distributing scanlines across ArenaOS's 4 native user threads (`userspace/arena-runtime/src/threads.rs`).
- **OS Dependency:** Standard C file I/O shims over `filesd`, thread-local storage (`fs:base`).

### Milestone G4: Virtio-GPU 3D (VirGL) Acceleration Substrate
- **Objectives:**
  - Upgrade `displayd` and introduce kernel VirGL command stream protocol.
  - Implement Gallium virgl driver in userspace for hardware-accelerated draw submission.
  - Enable host GPU pass-through in Linux QEMU environments (`-device virtio-gpu-gl-pci`).
- **OS Dependency:** Core OS Engineer must provide kernel PCI BAR 2 host-visible memory mappings and MSI-X interrupt relay for fence completion.

### Milestone G5: SuperTuxKart Compatibility Milestone
- **Objectives:**
  - SuperTuxKart compiles and runs on ArenaOS.
  - OpenGL 3.3 / GLES 3.0 shader pipeline via VirGL.
  - OpenAL audio output via virtio-snd driver.
  - Memory footprint tuned to 512 MiB guest RAM.
- **OS Dependency:** Virtio-sound audio driver, expanded process memory limits (256–512 MiB heap).

### Milestone G6: Blender Long-Term Benchmark
- **Objectives:**
  - Blender headless / GUI viewport rendering on ArenaOS.
  - Complete modern OpenGL 4.3+ or Vulkan (Venus) support.
  - Full C++ standard library and multithreaded task scheduler.
- **OS Dependency:** Multi-gigabyte virtual memory subsystem, 64-bit address space scalability, dynamic shared library support.
