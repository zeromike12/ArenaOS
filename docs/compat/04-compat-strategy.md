# 04 — Compatibility strategy comparison and initial dependency analysis

Status: analysis. No Linux binary compatibility is implemented or proposed
here. No SDL3, SuperTuxKart or Blender port is started. Graphics direction
(SDL3 and the video backend) belongs to Arena 1.

Source labels: **[PRIMARY]** official project documentation read in this
session; **[SECONDARY]** third-party summary, verify before relying on it;
**[CODE]** ArenaOS repository.

## 1. The three options

| Criterion | (a) Native source ports on an ArenaOS C runtime | (b) POSIX-subset userspace library over capability-brokered services | (c) Unmodified Linux ELF compatibility |
|---|---|---|---|
| What it means | Port source to a native C API (`arena/*.h`, streams, caps) | Libc-style `open/read/socket` calls in userspace; each call is translated to a broker IPC using caps the process holds | Load Linux ELF binaries and emulate the Linux syscall ABI, glibc and ld.so |
| Authority model | Explicit: a process holds exactly the caps it was granted | Preserved only if the broker, not the library, decides what a path means; path must never be authority | Ambient by design (uid, paths, `/proc`); must be wrapped to fit caps |
| Kernel change | None to start (ABI v1 suffices for the probe); stream binding is a proposal (ADR-0110) | None if all brokering is userspace; any new IPC is a proposal | Large: hundreds of syscalls, `futex`, signals, `clone`, `mmap` semantics, dynamic loader |
| Loader needs | Static ET_EXEC today; PIE from Luna (Phase 14) | Same as (a) | Dynamic linking, PIE, `PT_INTERP`, vDSO |
| Compatibility gained | Only ported code | Source code that uses the supported subset | Unmodified binaries |
| Fork and process model | Native spawn (no fork) | No `fork()` (spawn-only model); `exec`-style use must map to spawn with explicit grants | `fork`, `exec`, signals and pipes must all be emulated |
| Graphics path | Native video/audio backends (Arena 1) | Same, via brokers | GL/Vulkan drivers through an emulation layer; very large |
| Security surface added | C code in the app (contained by process isolation and caps) | Broker logic: a new trusted component that must not create ambient authority | The emulation layer is itself a large privileged surface |
| Effort (order of magnitude) | Medium per runtime; high per large app | High once; medium per library | Very high; out of scope |
| Recommendation | **Adopt first** (prototype already proves the base) | **Adopt selectively later**, for source compatibility, with the rules in §3 | **Reject** for this program |

Decision: **(a) first, then (b) as a bounded, broker-only library. (c) is not
pursued.** The reasons are the capability model (report 01), the measured
prototype (report 03), and the dependency weight of the target applications (§4).

## 2. Option (a) detail

Requirements already met by the prototype (guest-executed, report 03):
entry and stack switch, TLS, bounded formatted output (to serial only, see
F1), VM-backed allocation with reuse, string and memory operations, monotonic
time, sleep by yield, and exit.

Missing for real ports (report 01, F1–F10): a capability-based stdio binding,
a C thread and mutex API over the sync domains, a coalescing allocator, a
startup argv/env contract, and file/time/network brokers.

## 3. Option (b) rules (if pursued later)

1. The library never grants authority. It can only use caps the process was
   spawned with, plus the caps brokers return for resources it asked for.
2. A path string is a name for a broker to resolve. The broker decides what the
   name means, using the caller's caps. The library holds no ambient root.
3. No `fork()`. Programs that need it must be restructured to spawn with
   explicit grants (the ArenaOS model).
4. Every broker call is audited by the broker's own tests. The library gets no
   separate authority.
5. The library is opt-in per port and must not change ABI v1 without an ADR.

## 4. Initial dependency analysis

### 4.1 SDL3

What SDL3 needs at runtime (by subsystem):

| Need | Source | ArenaOS status | Gap |
|---|---|---|---|
| Glibc-only link; other features loaded at runtime | [PRIMARY] SDL Linux docs: "By default SDL will only link against glibc, the rest of the features will be enabled dynamically at runtime" ([README-linux.md](https://github.com/libsdl-org/SDL/blob/main/docs/README-linux.md)) | No glibc, no dynamic loader | A static SDL build with optional backends disabled is required; runtime `dlopen`-style loading is not available. This is the first blocker. |
| Threads | [PRIMARY] SDL_Init: file I/O and threading are initialised by default ([SDL_Init](https://wiki.libsdl.org/SDL3/SDL_Init)) | Prototype: single-thread; 4 extra threads per process (report 01, F6) | C thread API missing; sync domains exist in Rust only |
| Main-thread video init | [PRIMARY] SDL_Init (same page) | Possible: the main thread is the first thread | none |
| Video | [PRIMARY] Linux build dependencies include X11, Wayland, and libdrm/gbm (KMS) development packages ([README-linux.md](https://github.com/libsdl-org/SDL/blob/main/docs/README-linux.md)); the video backend set is therefore X11, Wayland and KMS/DRM [SECONDARY summary: deepwiki SDL3 overview] | No X11 or Wayland on ArenaOS | **Requires a new ArenaOS SDL video backend on Arena 1's graphics ABI. Arena 1 owns this.** |
| Audio | [PRIMARY] ALSA, PipeWire, PulseAudio, JACK, sndio (same doc) | No C-facing audio path identified in this audit | Audio backend and its service do not exist for C. Owner to confirm. |
| File I/O | [PRIMARY] SDL_Init (file I/O on by default) | Only via filesd IPC (report 01 §8) | Needs a broker-backed SDL IO backend (option b). |
| Input | [PRIMARY] joystick/udev notes in README-linux.md (udev-based detection on Linux) | No C input binding identified in this audit | C input binding and an SDL input backend |
| Build-time optional deps | [PRIMARY] README-linux.md (Vulkan, Wayland, PipeWire and others as dev packages) | none present | Disable in the ArenaOS SDL build config |

Initial verdict: **SDL3 is not portable without (1) a static configuration
with no runtime loading, (2) a C thread API, (3) an ArenaOS video backend, and
(4) an audio backend.** Items (3) and (4) are Arena 1 and audio-owner work.
Items (1) and (2) are inside this program's C-runtime scope.

### 4.2 SuperTuxKart (STK)

Dependencies listed in the project's build guide [PRIMARY:
[stk-code INSTALL.md](https://github.com/supertuxkart/stk-code)]: SDL2,
OpenAL, libcurl, enet, FreeType, HarfBuzz, libjpeg, libogg, libpng, OpenSSL or
mbedTLS, libvorbis, zlib and bluez. libopenglrecorder is optional
(`BUILD_RECORDER=off`). The guide lists SDL2, not SDL3.

Rendering: a GitHub-hosted mirror lists "OpenGL >= 3.1" and "512 MB VRAM"
[SECONDARY, verify against the INSTALL text]. A 2018 ODROID-XU4 post says
STK builds with `-DUSE_GLES2=1` for GLES 3.x [SECONDARY, old].

| Need | ArenaOS status | Gap |
|---|---|---|
| C++ standard library and exceptions | none (freestanding C only) | a C++ runtime profile; not in any current phase |
| Threads, mutexes, condition variables | prototype single-thread; Rust sync domains only | C thread and sync API |
| Sockets and TLS (libcurl, enet, OpenSSL) | netd exists as a Phase-13 service | No C socket binding; broker design needed (option b) |
| OpenGL 3.1+ or GLES 3.x | no GPU API for C; graphics belongs to Arena 1 | depends entirely on Arena 1 graphics |
| OpenAL audio | no C audio path (§4.1) | audio backend |
| Font and image libraries (FreeType, HarfBuzz, libpng) | nothing in the image | these are pure userspace C; portable once a C runtime exists. Build-only work. |
| Filesystem | filesd IPC only | broker-backed I/O |

Initial verdict: **STK is not a near-term target.** The C++ runtime, the
graphics API, the audio path, and sockets are each a separate phase. Pure-C
dependencies (FreeType, libpng, zlib, and others) can be tested as a
library-porting exercise earlier, without STK.

### 4.3 Blender

Official requirements [PRIMARY: [Blender system requirements](https://www.blender.org/download/requirements/),
fetched 2026-10-08]:

- Linux: "Distribution with glibc 2.28 or newer (64-bit)".
- CPU: "4 cores with SSE4.2 support" (minimum), 8 recommended.
- RAM: 8 GB minimum, 32 GB recommended.
- GPU: 2 GB VRAM with **OpenGL 4.3 and Vulkan 1.3** (listed in the table),
  plus the mandatory extensions GL_ARB_shader_draw_parameters and
  GL_ARB_clip_control for OpenGL.

Build dependencies from the Linux developer handbook [PRIMARY: developer.blender.org
Linux build page, read earlier in this session]: X11 libraries, EGL, Wayland,
wayland-protocols, xkbcommon, libdecor and dbus. Portable builds need an
older glibc and precompiled libraries.

Forward-looking GPU statements [SECONDARY, unverified]: a 2025 Blender 4.5 LTS
article says Vulkan reached feature parity with OpenGL, and a 2026-10-06 article
says Blender 5.3 plans Vulkan as default with a Vulkan 1.1 minimum. Neither is
verified against Blender release notes, and they conflict with the official
requirements page above, which still lists Vulkan 1.3. Do not rely on them.

| Need | ArenaOS status | Gap |
|---|---|---|
| glibc 2.28+ and dynamic linking | none | dynamic linker and glibc-compatible C library: out of this program |
| **SSE4.2 CPU baseline** | **Conflict.** The kernel never sets `CR4.OSFXSR` and has no FXSAVE/XSAVE path (grep of `kernel/kernel/src` finds none). Per Intel SDM Vol. 3 §2.6, with OSFXSR clear SSE instructions raise #UD. | A CPU-feature and FP/SIMD-state design is needed before any SSE-compiled application can run. Recorded as a blocking item. |
| OpenGL 4.3 / Vulkan 1.3 GPU | No GPU driver or GPU API on ArenaOS | Arena 1 graphics; far beyond this program |
| X11/Wayland/EGL/dbus | none | new backend; see SDL3 |
| 8 GB RAM, large heaps | ArenaOS VM caps: 8,192 committed pages per process (32 MiB), 4 MiB C heap in the prototype | address-space and commit limits must be raised by an ADR; not in scope |
| Python embedding | none | interpreter port; not in scope |

Initial verdict: **Blender is not portable in the foreseeable roadmap.** The
SSE4.2 baseline conflicts with the current execution contract, which is the
most concrete finding in this section. It needs the GPU and loader work above
first. Do not port Blender in this assignment (per the assignment's
constraints).

## 5. Conclusions

- Option (a) is the right first step. It needs a capability stdio binding and
  a C thread API, and both are proposed or in-scope (ADR-0110, report 05).
- Option (b) is worth designing once (a) has real users, under the rules in §3.
- Option (c) is rejected for this program.
- SDL3 is the realistic first large target, and it depends on Arena 1 video
  and audio backends. The C runtime must provide a static, non-loading build
  and threads.
- SuperTuxKart and Blender are long-term targets. Their blockers are C++,
  GPU APIs, audio, sockets, CPU-feature support (SSE4.2) and memory limits.
  Neither is near-term.
