# ArenaOS Graphics Architecture Audit

## 1. Executive Summary & Qualification Baseline

This audit establishes the baseline graphics, memory, and desktop capabilities of **ArenaOS at the completion of Phase 13** (commit `74ace4f9e9898fa8f567d2d666c2a84c6b333bc9`).

ArenaOS is an original, capability-based micro-operating system written in pure Rust (`no_std`), running in x86_64 Long Mode with hardware paging and Ring 3 user execution. It is **not** a Unix or Linux derivative:
- All authority is borne strictly by capability tokens held in fixed per-process capability tables (`CAP_SLOTS = 128`).
- Names, paths, process IDs, and descriptive metadata carry zero execution authority.
- The system uses no dynamic linker, no POSIX libc, no ambient device filesystem (no `/dev/fb0` or `/dev/dri/card0`), and no traditional windowing protocol (neither X11 nor Wayland).

The graphics stack currently supports a bounded desktop environment (up to 32 active windows across up to 32 application sessions, capped at a resolution of 1024×768) driven by a software compositor and a display server supporting either UEFI GOP framebuffer output or OASIS Virtio-GPU 2D command transfers.

---

## 2. Comprehensive Architectural Audit

### 2.1 How Pixels Reach the Display

The pixel path from application rasterization to display scanout spans three distinct Ring 3 processes:

```
[ Application Process ]
       │  Renders into mapped SharedRegion (offset 4096: XRGB8888)
       ▼
 [ SYS_IPC_CALL: Frame::Damage { rects } ]
       │  Carries surface backing cap + session badge
       ▼
[ Desktop Compositor (desktop.rs) ]
       │  publish_rects: copies damaged rectangles into private snapshot SharedRegion
       │  Scene composition: draws desktop background, dock, chrome, and blits snapshots
       │  Canvas: composes damage into scanout SharedRegion
       ▼
 [ SYS_IPC_CALL: Frame::PresentRects { rects } ]
       │  Carries display scanout SharedRegion cap
       ▼
[ Display Server (displayd) ]
       ├── If GOP Mode:
       │     Volatile stores / copy_nonoverlapping into mapped UEFI GOP MMIO framebuffer
       └── If Virtio-GPU 2D Mode:
             virtio-gpu TRANSFER_TO_HOST_2D + RESOURCE_FLUSH across PCI control queue
       ▼
[ Hardware / Emulated Display Surface ]
```

1. **Application Staging:** The client process renders 32-bit `XRGB8888` pixels into its own mapped `SharedRegion` at byte offset `PIXEL_OFFSET = 4096` (`userspace/desktop/src/client.rs:18`). The first 4096 bytes are reserved for explicit service I/O.
2. **Damage Notification:** When the client finishes drawing, it issues an IPC call to the Desktop compositor (`Frame::Damage` with up to 5 discrete `[x, y, width, height]` rectangles or full window, `userspace/desktop/src/wire.rs:16-43`).
3. **Private Snapshot Buffering:** In the Desktop compositor (`userspace/desktop/src/bin/desktop.rs:650-675`), `publish_rects` copies the modified pixel rows from the client's shared staging memory into Desktop's **private snapshot** `SharedRegion`. The client's staging memory is never read directly during scene rendering, preventing tearing or race conditions from malformed or concurrent client writes.
4. **Scene Composition:** During the Desktop render phase (`userspace/desktop/src/bin/desktop.rs:5250-5320`), the compositor calculates screen-space damage across all active windows, menus, and shell elements (`userspace/desktop/src/compose.rs:18-80`). Damaged rectangles are composited into the `scanout` `SharedRegion` (shared exclusively between `desktop` and `displayd`).
5. **Presentation Dispatch:** Desktop sends an IPC request `Frame::PresentRects` (`userspace/desktop/src/bin/desktop.rs:5290-5305`) to `displayd` over its held `DISPLAY` endpoint capability, transferring the `scanout` `SharedRegion` capability.
6. **Hardware Scanout (`userspace/displayd/src/main.rs:80-135`):**
   - **GOP Mode:** `displayd` maps the physical GOP framebuffer via `SYS_MAP_MEMORY` using its boot-granted `Mmio` capability (`SLOT_GOP = 0`). It copies the presented rows directly to physical video RAM using volatile stores or non-overlapping row copies.
   - **Virtio-GPU 2D Mode (`userspace/displayd/src/gpu.rs:60-140`):** `displayd` issues `TRANSFER_TO_HOST_2D` to copy updated pixels from the attached guest physical pages to host resource 1, followed by `RESOURCE_FLUSH` to trigger host scanout updates.

---

### 2.2 How Applications Create Windows

Window creation is strictly brokered by capability authority and the Startup ABI v2 (`docs/adr/0080`, `0083`, `0097`):

1. **Launch Brokering:**
   - The trusted Desktop manager (`userspace/desktop/src/bin/desktop.rs:4300-4450`) acts as launch broker.
   - To launch an application, Desktop creates an `AppInstance` and allocates a bounded `SharedRegion` surface reservation sized to the screen work area (`work_w * work_h * 4 + 4096 + TRANSIENT_PAGES * 4096 + FILE_PAGES * 4096`, `docs/adr/0075`).
   - Desktop spawns the child process via `SYS_SPAWN`, delegating exact startup capabilities:
     - Slot 0: Startup transport / manifest page (`arena_startup_abi`).
     - Slot 1: Badged session endpoint (`BadgedEndpoint` minted with unique session badge).
     - Slot 2: Surface reservation `SharedRegion` capability.
2. **Primary Window Establishment (`Client::connect_v2`, `userspace/desktop/src/client.rs:130-195`):**
   - The child inspects the startup capabilities, maps the inherited `SharedRegion` via `SYS_SHARED_MAP(backing, 1)`, and queries its bounds via `SYS_SHARED_INFO`.
   - The child transmits `Frame::Create { width, height }` over its badged session endpoint to Desktop.
   - The child transmits `Frame::Title { handle, text }` to define its descriptive window label.
   - Desktop validates the session badge, associates the primary window with the caller's held `Process` capability, and records the initial window bounds in its session table.
3. **Additional Windows (`Client::create_window`, `userspace/desktop/src/client.rs:215-285`):**
   - Under Phase-13 ADR-0097, a process may own multiple windows (up to 32 system-wide).
   - The client sends `Frame::CreateAdditional { width, height }` over the badged session endpoint.
   - Desktop allocates a fresh `SharedRegion` and a matching private snapshot, registers an extra window record, and transfers the new `SharedRegion` capability in the IPC reply.
   - The client maps the newly received capability and renders independently.

---

### 2.3 How Application Surfaces Reach the Compositor

Surface sharing is grounded entirely in kernel-managed `SharedRegion` capability primitives (`docs/adr/0056`, `kernel/kernel/src/shared.rs`):

- **No Shared Virtual Addresses:** Physical frames are allocated as contiguous runs in the kernel frame allocator upon `SYS_SHARED_CREATE(pool, pages, out)`.
- **Capability-Gated Mapping:** A process can only map a surface by possessing the exact `SharedRegion` capability (`CapObj::SharedRegion { id }`) and calling `SYS_SHARED_MAP(slot, write_flag)`.
- **Zero-Page Guarantee:** The kernel clears all allocated pages to zero before publishing the capability, preventing cross-client memory leakage (`docs/adr/0056`).
- **Independent Address Spaces:** The application maps the region into its own page table at a kernel-selected virtual address; the Desktop compositor maps the identical physical backing into its own address space.
- **Strict Separation of Staging vs Scene:** The compositor never exposes its own scanout buffer to applications. Applications hold authority only over their allocated surface regions.

---

### 2.4 How Input Reaches Applications

Input delivery is mediated by `inputd`, an unforgeable kernel token, and Desktop hit-testing:

1. **Hardware Capture (`userspace/inputd/`):**
   - `inputd` manages the QEMU virtio-input PCI device (keyboard and absolute pointer / tablet).
   - Scancodes and pointer coordinates are decoded into typed events.
2. **Compositor Injection (`docs/adr/0057`, `0060`):**
   - `inputd` delivers events to Desktop over the `SERVER` endpoint (`userspace/desktop/src/input_wire.rs`).
   - Every input IPC transfer must carry an unforgeable, root-minted inert `ProofToken` capability (`CapObj::ProofToken { id }`). Requests lacking this token are rejected.
3. **Compositor Routing & Focus (`userspace/desktop/src/bin/desktop.rs:5750-5840`):**
   - Desktop processes absolute pointer coordinates, updates the hardware cursor sprite, and performs z-ordered hit-testing against window frames and client rectangles.
   - Keyboard events are dispatched strictly to the currently focused window session.
   - Each session maintains a bounded FIFO queue of at most 16 events. Overflows drop new events and increment an error counter rather than overwriting.
4. **Client Wake and Polling (`userspace/desktop/src/client.rs:430-465`):**
   - Desktop signals the application's private pacing Notification capability (`SYS_NOTIFY`).
   - The application polls for input by sending `Frame::Poll { handle }` over its badged session endpoint.
   - Desktop dequeues the event and replies with `Frame::Event { handle, event }`.

---

### 2.5 How Buffers and Mappings are Protected

Memory isolation is strictly enforced at both kernel and compositor boundaries:

1. **Kernel Object Capabilities:**
   - A `SharedRegion` capability (`CapObj::SharedRegion { id }`) is a kernel object. Possession of the slot in `proc.caps` with `RIGHTS_READ` or `RIGHTS_WRITE` is the sole authority to map memory.
   - `SYS_SHARED_MAP` validates that the caller holds the capability, enforces self-mapping only (a process cannot map into another address space), sets `PAGE_NX` (No-Execute), and enforces Read-Only vs Read-Write based on held capability rights (`kernel/kernel/src/arch/x86_64/syscall.rs:2810-2860`). W^X is strictly enforced.
2. **Physical Address Concealment:**
   - Physical frame addresses are never revealed to client applications.
   - Only `displayd` holds the special `SharedDma/READ` capability required to invoke `SYS_SHARED_PHYS` for virtio-gpu DMA setup (`docs/adr/0056`). Ordinary clients calling `SYS_SHARED_PHYS` or `SYS_CAP_PHYS` on a `SharedRegion` receive `-2` (`STATUS_BAD_ARG`).
3. **Reference Counting and Mapping Pins:**
   - The kernel maintains `refs` (capability count) and `pins` (active page-table mappings) for each region (`kernel/kernel/src/shared.rs:60-120`).
   - Dropping or transferring a capability does not invalidate an active mapping.
   - Physical frames are reclaimed if and only if `refs == 0`, `pins == 0`, and all in-flight IPC escrows are retired.
   - On process death, the kernel sweeps all mappings before releasing capability references, preventing double-free or dangling physical frame allocations.
4. **Compositor Private Snapshots:**
   - The Desktop compositor creates a private snapshot `SharedRegion` for every window and immediately destroys its own capability after mapping it (`docs/adr/0075`). The mapping remains pinned by the kernel until Desktop unmaps it or terminates.
   - Applications have zero access to the private snapshot. Staging memory corrupted by a rogue or crashed application cannot affect the compositor's scanout integrity.

---

### 2.6 How Processes Synchronize with Rendering

ArenaOS uses synchronous IPC, explicit pacing notifications, and native sync primitives:

1. **Compositor-to-Display Synchronization:**
   - Presentation via `SYS_IPC_CALL(DISPLAY, ...)` is synchronous. `desktop` blocks until `displayd` completes the MMIO row copy or virtio-gpu queue submission, guaranteeing single-writer scanout access without tearing (`userspace/desktop/src/bin/desktop.rs:2820-2840`).
2. **Client-to-Compositor Damage Synchronization:**
   - `Client::damage()` and `Client::damage_rects()` execute as synchronous IPC (`SYS_IPC_CALL`). The client thread blocks while Desktop executes `publish_rects` to copy pixels into the private snapshot.
3. **Frame Pacing & VSync Emulation:**
   - Applications do not spin-wait. `app_client::idle` blocks on a private pacing Notification capability or arms a timer via `SYS_TIMER_ARM` (`userspace/desktop/src/app_client.rs:60-120`).
   - Desktop paces applications at ~50 Hz (20 ms period) and issues `SYS_NOTIFY` hints when damage composition or input is ready.
4. **Multithreaded User Synchronization (Phase 13, ADR-0107):**
   - Within an application process, native multithreaded rendering uses `SyncDomain` capabilities (`CAP_KIND_SYNC_DOMAIN`).
   - Native `Mutex`, `Condvar`, and `Once` primitives in `arena-runtime` operate via atomic sequence-check-and-park (`SYS_SYNC_WAIT`, `SYS_SYNC_WAKE`) rather than Linux futexes (`userspace/arena-runtime/src/sync.rs:30-150`).

---

### 2.7 What Graphics Hardware Interfaces are Currently Available

ArenaOS Phase 13 exposes exactly two graphics hardware interfaces, both accessed exclusively by `displayd`:

1. **UEFI Graphics Output Protocol (GOP):**
   - Early boot captures the EFI framebuffer base address, dimensions, stride, and 32-bit pixel format (BGR or RGB) before `ExitBootServices` (`boot/boot/src/main.rs`).
   - The kernel mints a single `Mmio/READ|WRITE` capability covering the physical framebuffer to `displayd` (`docs/adr/0056`).
   - `displayd` queries mode parameters via `SYS_DISPLAY_INFO` and maps the framebuffer uncached (`PTE_PWT | PTE_PCD`), non-executable (`PTE_NX`).
2. **Virtio-GPU 2D Transport (PCI Device 0x1AF4:0x1050):**
   - Discovered during kernel PCI scan and granted to `displayd` as a bounded BAR MMIO capability (`docs/adr/0058`).
   - Implements OASIS Virtio v1.3 §5.7 2D features using a single polling queue (MSI-X disabled):
     - `VIRTIO_GPU_CMD_GET_DISPLAY_INFO`
     - `VIRTIO_GPU_CMD_RESOURCE_CREATE_2D`
     - `VIRTIO_GPU_CMD_RESOURCE_ATTACH_BACKING`
     - `VIRTIO_GPU_CMD_SET_SCANOUT`
     - `VIRTIO_GPU_CMD_TRANSFER_TO_HOST_2D`
     - `VIRTIO_GPU_CMD_RESOURCE_FLUSH`
   - Maximum scanout size is bounded at 512 pages (2 MiB, e.g. 800×600 or 1024×768). Larger modes fail closed.
3. **Hardware Acceleration Status:**
   - **Zero 3D Hardware Acceleration:** No virgl 3D, no Venus Vulkan, no host pass-through, no proprietary GPU drivers (Intel/AMD/NVIDIA).
   - **Zero Client Hardware Access:** Applications cannot map GPU registers or issue hardware command packets.

---

### 2.8 Specific Limitations Preventing Existing Graphics Libraries from Running

The following architectural constraints currently prevent standard libraries (SDL2/SDL3, Mesa, GLFW, cairo) and complex applications (Blender, SuperTuxKart) from executing on ArenaOS:

| Domain | ArenaOS Phase-13 State | Standard Graphics Library Assumption |
|---|---|---|
| **Binary Format** | Static `ET_EXEC` ELF only (`docs/adr/0108`). Rejects `ET_DYN` and `PT_INTERP`. | Requires dynamic shared libraries (`.so`), dynamic linking (`dlopen`, `dlsym`), or static PIE. |
| **C Runtime** | Pure Rust `no_std`. Custom syscall numbers (`SYS_IPC_CALL`, `SYS_SHARED_MAP`). No libc. | Assumes POSIX libc (glibc/musl): `malloc`, `free`, `errno`, `open`, `mmap`, `ioctl`, `pthreads`. |
| **Window System** | Proprietary 64-byte IPC frames (`ADSK` wire protocol) with transferred capability objects. | Expects Wayland protocol (`wl_surface`, `wl_compositor`), X11 (`libX11`), or Windows Win32 API. |
| **Graphics API** | 2D pixel copy to 32-bit `XRGB8888` `SharedRegion` memory. | Expects OpenGL (`libGL.so`), OpenGL ES (`libGLESv2.so`), EGL (`libEGL.so`), or Vulkan (`libvulkan.so`). |
| **Buffer Management** | Fixed 4 KiB aligned `SharedRegion`, max 1024 pages (4 MiB) per region, max 96 system-wide regions. | Assumes DRM/KMS GEM/TTM ioctls, `GBM` (Generic Buffer Management), or arbitrary `mmap`. |
| **Pixel Formats** | Only 32-bit opaque `XRGB8888` (`fmt = 0` or `1`). | Requires RGBA8888, RGB565, floating-point surfaces, depth buffers (D24S8, D32F), texture formats. |
| **Threading Limits** | Max 4 user threads per process, 64 global threads. | Modern 3D engines spawn worker pools, asset streaming threads, and parallel shader compilers. |
| **Memory Capacity** | 512 MiB total guest RAM, 16 MiB scalable heap default (`docs/adr/0096`). | Blender 4.x requires >2 GiB RAM; SuperTuxKart assets require 200–500 MiB RAM. |

---

## 3. Confirmed Implementation Details vs Architectural Assumptions

To ensure rigorous engineering, the table below separates verified facts from future assumptions:

| Topic | Confirmed Implementation Details (Verified in Source) | Assumptions / Future Extrapolations |
|---|---|---|
| **Display Device** | `displayd` supports GOP MMIO and Virtio-GPU 2D polling queue (`userspace/displayd/src/main.rs`). | Virtio-GPU 3D (VirGL) can be added simply as a userspace protocol extension without kernel interrupt changes. |
| **Window Protocol** | Desktop speaks `ADSK` v1 over 64-byte IPC messages with capability transfer (`userspace/desktop/src/wire.rs`). | An SDL3 video backend can map 1:1 onto `ADSK` without altering desktop wire structures. |
| **Buffer Access** | Clients access surfaces via Ring-3 memory pointers into mapped `SharedRegion`s (`userspace/desktop/src/client.rs`). | Zero-copy client rendering is fast enough for 30–60 FPS 3D software rasterization. |
| **Executable Loading** | Kernel strictly enforces static `ET_EXEC` ELF, rejecting relocations and dynamic linkers (`docs/adr/0108`). | Phase 14 static PIE will suffice for C runtime porting without requiring full dynamic `ld.so`. |
| **Concurrency** | Threads share process PML4, TLS in FS.base, max 4 threads (`userspace/arena-runtime/src/threads.rs`). | Software 3D renderers can effectively distribute scanlines across 4 native user threads. |

---

## 4. Key Source Locations

- **System Call ABI:** `userspace/abi.rs`, `kernel/kernel/src/arch/x86_64/syscall.rs`
- **Capability Object Definitions:** `kernel/kernel/src/cap.rs:55-155`
- **Generic Shared Memory Substrate:** `kernel/kernel/src/shared.rs`
- **Display Service (GOP & Virtio-GPU 2D):** `userspace/displayd/src/main.rs`, `gpu.rs`
- **Virtio-GPU 2D Command Codec:** `userspace/gpu2d/src/lib.rs`, `queue.rs`, `device.rs`
- **Desktop Window Manager & Compositor:** `userspace/desktop/src/bin/desktop.rs`, `compose.rs`, `desk.rs`
- **Desktop Client Interface:** `userspace/desktop/src/client.rs`, `wire.rs`, `model.rs`
- **2D Drawing Toolkit & Font Engine:** `userspace/gfxkit/src/lib.rs`
- **Native Memory & Multithreading Runtime:** `userspace/arena-runtime/src/heap.rs`, `threads.rs`, `sync.rs`
