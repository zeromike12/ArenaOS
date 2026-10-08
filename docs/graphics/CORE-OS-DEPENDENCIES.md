# Core OS Graphics Dependencies Matrix

- **Subsystem:** Core Microkernel, Memory Manager, Capability Routing, Display Driver
- **Author:** ArenaOS Graphics Infrastructure Engineer
- **Baseline:** Phase 13 Qualified (`74ace4f9e989`)
- **Review Intent:** Formal classification of graphics infrastructure requirements into **Hard Blockers** vs. **Non-Blocking Performance Enhancements**.

---

## 1. Executive Summary

ArenaOS graphics capabilities must develop without uncoordinated changes to the core microkernel, memory manager, or capability security invariants. This document provides the authoritative contract between the **Graphics Subsystem** and the **Core OS Engineering Team**.

Crucially, **3D rendering does NOT immediately require kernel hardware acceleration, kernel memory restructuring, or PCI modifications**:
1. **Immediate Milestones (G1–G2):** 3D software rendering and basic window presentation operate 100% within existing Phase-13 capability interfaces (`SharedRegion`, `SYS_SHARED_MAP`, `ADSK-v1`, `SYS_VM_RESERVE/COMMIT`).
2. **Intermediate Milestones (G2–G3):** C runtime compatibility for SDL3 and software rasterizers requires only user-level static PIE support and userspace memory translation.
3. **Hardware Acceleration (G4 - Virtio-GPU VirGL 3D):**
   - **Baseline VirGL 3D:** Operates across the existing Virtio modern control queue using standard guest scatter-gather page lists (`ATTACH_BACKING`) and polled fences. It requires **zero kernel PCI BAR or interrupt changes**.
   - **Optional VirGL Optimizations:** Host-visible shared memory blobs (`VIRTIO_GPU_F_RESOURCE_BLOB` on PCI BAR 2/4) and interrupt-driven fences (`SYS_IRQ_RELAY`) are non-blocking performance upgrades, not functional prerequisites.

---

## 2. Hard Prerequisites (Functional Blockers for Roadmap Milestones)

Only two items represent hard prerequisites from the Core OS team, both required for C library compilation and dynamic execution:

### 2.1 Toolchain & Static PIE Support (Milestone G2 Blocker)
- **Target Milestone:** Milestone G2 (SDL3 + glxgears).
- **Core OS Owner:** Core OS Engineer (Phase 14 milestone).
- **Justification:** Standard C libraries (SDL3, Mesa software rasterizer, zlib) and Clang/GCC toolchains emit Position-Independent Executables (PIE). The current Phase-13 loader strictly rejects relocations and requires fixed `ET_EXEC` base addresses (`docs/adr/0108`).
- **Required Contract:**
  1. Kernel and userspace loader must accept `ET_DYN` static PIE binaries with base-relative relocations (`R_X86_64_RELATIVE`) resolved at load time.
  2. The application binary format must remain self-contained (no dynamic `.so` loading).
  3. Memory permissions must strictly preserve $W \oplus X$ (`docs/adr/0095`).

### 2.2 Userspace C Runtime Dynamic Memory Translation (Milestone G2 Blocker)
- **Target Milestone:** Milestone G2 (Standard C Application Porting).
- **Core OS Owner:** Core OS Engineer / Userspace Runtime.
- **Justification:** C graphics software expects `malloc`, `realloc`, and `free` backed by anonymous `mmap`.
- **Required Contract:**
  1. Leverage existing Phase-13 native VM syscalls (`SYS_VM_RESERVE`, `SYS_VM_COMMIT`, `SYS_VM_RELEASE`, `docs/adr/0095`).
  2. A minimalist C runtime shim in `arena-runtime` must translate POSIX `mmap(PROT_READ | PROT_WRITE, MAP_ANONYMOUS)` into `SYS_VM_RESERVE` and `SYS_VM_COMMIT`.
  3. No kernel change is required; this is purely userspace C runtime packaging.

---

## 3. Virtio-GPU 3D (VirGL) Requirements: Baseline vs. Enhancements

Virtio-GPU 3D acceleration (Milestone G4) is often assumed to require massive kernel changes. We delineate the functional baseline from optional optimizations:

### 3.1 Baseline VirGL 3D (Functional Minimum — No Kernel Changes Required)
- **Mechanism:**
  - Control virtqueue command submission: `VIRTIO_GPU_CMD_CTX_CREATE`, `RESOURCE_CREATE_3D`, `ATTACH_BACKING`, `SUBMIT_3D`, `TRANSFER_TO_HOST_3D`, `RESOURCE_FLUSH`.
  - Memory: Attaches existing guest physical frames allocated via `SYS_SHARED_CREATE` or page reservations to host resources using scatter-gather descriptors (`ATTACH_BACKING`). This mirrors ArenaOS's existing Virtio-GPU 2D implementation in `userspace/gpu2d`.
  - Synchronization: Fences are polled synchronously on the control virtqueue used-ring via `VIRTIO_GPU_FLAG_FENCE`.
- **Kernel Impact:** **ZERO.** Operates entirely within the userspace display server (`displayd`) using existing modern PCI capability MMIO.

### 3.2 Optional Optimization 1: Host-Visible Blobs (PCI BAR 2/4 Mapping)
- **Target Milestone:** Post-G4 Performance Upgrade.
- **Benefit:** Eliminates copy overhead between guest RAM and host GPU memory by directly mapping host-allocated staging buffers into guest userspace (`VIRTIO_GPU_F_RESOURCE_BLOB`).
- **Core OS Contract if Implemented:**
  1. Extend kernel PCI capability scanning to discover and mint an explicit `Mmio/READ|WRITE` capability for Virtio-GPU BAR 2 when present.
  2. Grant this capability exclusively to `displayd`. Ordinary applications remain barred from direct MMIO.

### 3.3 Optional Optimization 2: MSI-X Interrupt Relay for GPU Fences
- **Target Milestone:** Post-G4 Performance Upgrade.
- **Benefit:** Wakes waiting user threads via `SYS_WAIT` on a Notification capability instead of busy-polling the virtqueue used ring during long GPU draw passes.
- **Core OS Contract if Implemented:**
  1. Kernel binds the Virtio-GPU MSI-X fence vector to `SYS_IRQ_RELAY`.
  2. Signals a dedicated Notification capability delivered to `displayd`.

---

## 4. Resource Capacity Envelopes

These tuning enhancements improve rendering scale, but baseline software rendering functions within current limits:

| Subsystem | Existing Phase-13 Limit | Proposed Graphics Envelope | Benefit | Fallback Behavior if Omitted |
|---|---|---|---|---|
| **Per-Process Heap** | 16 MiB scalable heap (`docs/adr/0096`) | 64–128 MiB scalable heap | Allows high-res textures and large geometry models (SuperTuxKart). | Textures are kept low-res (64×64 to 256×256); low-poly meshes. |
| **SharedRegion Capacity** | 96 global regions; 1024 pages per region (`kernel/shared.rs`) | 128 global regions; 2048 pages per region | Enables 1080p surfaces and multi-window 3D viewports. | Display surfaces bounded at 1024×768 maximum. |
| **User Threads** | Max 4 user threads per process (`docs/adr/0106`) | 8–16 threads per process | Enables multi-core scanline parallel software rendering. | Software rasterizer uses up to 4 threads or single thread. |
| **Audio Subsystem** | None | Virtio-Sound (`virtio-snd`) | Game sound effects and music. | Audio operates with silent dummy output. |

---

## 5. Architectural Invariants (Non-Negotiable)

1. **Capability Isolation:** Applications never receive raw physical MMIO or DMA capabilities.
2. **Deterministic Teardown:** Crashed 3D applications must release all `SharedRegion` allocations without leaking kernel heap or compositor surface slots.
3. **$W \oplus X$ Enforcement:** JIT shader compilers (e.g. Mesa LLVMpipe) are prohibited; all shaders in ArenaOS must be ahead-of-time compiled or interpreted within non-executable memory.
