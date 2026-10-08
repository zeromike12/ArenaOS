# Core OS Dependencies & Handoff Contract

## 1. Overview & Parallel Development Principles

To maintain strict parallel velocity alongside the Core OS Engineer:
- **No Production Kernel Changes:** The Graphics Infrastructure Engineer will not modify kernel sources, capability semantics, the public process ABI, or memory management in current releases.
- **Clear Interface Contracts:** This document establishes the exact, versioned system contracts the graphics stack requires from the core operating system across upcoming phases.
- **Prerequisites vs Enhancements:** Dependencies are strictly categorized into **Essential Prerequisites** (blocking milestones) and **Optional Enhancements** (performance optimizations).

---

## 2. Essential Prerequisites (Blocking Compatibility Milestones)

The following core OS capabilities are hard prerequisites for achieving the milestones in `docs/graphics/COMPATIBILITY-ROADMAP.md`:

### 2.1 Static PIE and ASLR (`ET_DYN`) — Owned by Core Engineer in Phase 14
- **Target Milestone:** Milestone G2 (`glxgears` / `testgl` with SDL3).
- **Justification:** Existing C toolchains (GCC / Clang) producing position-independent static binaries generate `ET_DYN` ELF objects with `R_X86_64_RELATIVE` dynamic relocations. ArenaOS currently rejects `ET_DYN` (`docs/adr/0108`).
- **Required Contract:**
  1. The kernel ELF loader must admit static `ET_DYN` without `PT_INTERP`.
  2. The loader must process base-relative relocations (`R_X86_64_RELATIVE`) applying the randomized load bias.
  3. The loader must report the final base address in Startup ABI v2 metadata so C runtimes can initialize TLS and data segments.
  4. Enforce W^X and user range checks on all loaded segments.

### 2.2 Standard C Memory Expansion Stub (`sbrk` / `mmap` Emulation)
- **Target Milestone:** Milestone G2 (SDL3 and C library integration).
- **Justification:** Standard C graphics libraries expect a heap allocator (`malloc`/`free`) capable of growing virtual address space.
- **Required Contract:**
  1. Leverage existing Phase-13 native VM syscalls (`SYS_VM_RESERVE`, `SYS_VM_COMMIT`, `SYS_VM_RELEASE`, `docs/adr/0095`).
  2. A minimalist C runtime shim in `arena-runtime` must translate POSIX `mmap(PROT_READ | PROT_WRITE, MAP_ANONYMOUS)` into `SYS_VM_RESERVE` and `SYS_VM_COMMIT`.
  3. No kernel change is required; this is purely userspace C runtime packaging.

### 2.3 Kernel PCI BAR 2 Mapping & Virtio-GPU Host Shared Memory
- **Target Milestone:** Milestone G4 (Virtio-GPU 3D VirGL Acceleration).
- **Justification:** Virtio-GPU 3D command submission and host blob resources (`VIRTIO_GPU_CMD_RESOURCE_CREATE_BLOB`) require mapping PCI BAR 2/4 into userspace to access host-visible staging buffers.
- **Required Contract:**
  1. Extend kernel PCI device capability scanning to discover and mint an explicit `Mmio/READ|WRITE` capability for Virtio-GPU BAR 2 when present.
  2. Deliver this capability exclusively to the display service (`displayd`).
  3. Ordinary applications must remain prohibited from directly mapping device BARs.

### 2.4 MSI-X Interrupt Relay for GPU Fences
- **Target Milestone:** Milestone G4 (Hardware-accelerated 3D).
- **Justification:** Virtio-GPU 3D requires asynchronous fence signaling (`VIRTIO_GPU_CMD_SUBMIT_3D` with fence ID). Polling queues introduce excessive latency and CPU spinning for complex draw streams.
- **Required Contract:**
  1. Provide interrupt notification via `SYS_IRQ_RELAY` (`userspace/abi.rs:49`) for the GPU completion vector.
  2. The kernel must route the device interrupt to a dedicated Notification capability held by `displayd`.

---

## 3. Optional Enhancements (Non-Blocking Performance Upgrades)

These enhancements improve rendering performance and capacity envelopes, but their absence will not block milestone progression:

| Subsystem | Proposed Enhancement | Benefit | Fallback Behavior if Omitted |
|---|---|---|---|
| **Memory Limits** | Increase per-process scalable heap ceiling from 16 MiB to 64–128 MiB (`docs/adr/0096`). | Enables loading larger 3D models and uncompressed textures (required for SuperTuxKart). | Textures are compressed or kept small (64×64 to 256×256). |
| **SharedRegion Capacity** | Increase global SharedRegion table from 96 to 128 entries; per-region cap from 1024 pages to 2048 pages (`kernel/kernel/src/shared.rs`). | Allows higher desktop resolutions (1280×1024, 1920×1080) and 4K display scanouts. | Bounded at 1024×768 maximum resolution. |
| **Scheduler Concurrency** | Increase user threads per process from 4 to 8–16 (`docs/adr/0106`). | Allows parallel multi-core software rasterization (e.g. 8 parallel scanline workers). | Software rasterizer uses up to 4 threads or single thread. |
| **Audio Subsystem** | Userspace driver for Virtio-Sound (`virtio-snd`, device type 25). | Enables game audio for SuperTuxKart. | Audio subsystem operates with dummy silent output. |

---

## 4. Responsibility Matrix: Division of Labor

```
┌─────────────────────────────────────────────────────────────┐
│                 ENGINEERING RESPONSIBILITY MATRIX           │
├──────────────────────────────┬──────────────────────────────┤
│ Core OS Engineer Owns:       │ Graphics Engineer Owns:      │
├──────────────────────────────┼──────────────────────────────┤
│ - Phase 14 Static PIE / ASLR │ - SDL3 ArenaOS Video Driver  │
│ - Kernel ELF Loader          │ - SDL3 Input & Event Driver  │
│ - Virtual Memory Substrate   │ - Software 3D Pipeline       │
│ - PCI Device Scanning & BARs │ - Software OpenGL Runtime    │
│ - Capability System Invariants│ - ADSK-v2 Protocol Spec     │
│ - Scheduler & Synchronization│ - Desktop Compositor Bridge  │
│ - Production Qualification   │ - 3D Benchmark Suites        │
└──────────────────────────────┴──────────────────────────────┘
```

By adhering strictly to this contract, the Graphics Infrastructure Engineer can build the entire userspace graphics framework, SDL3 backend, and software OpenGL pipeline without interfering with or depending on the Core OS Engineer's concurrent Phase-14 kernel development.
