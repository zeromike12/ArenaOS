# ArenaOS Graphics Foundation Prototype: Milestone G1.1 Verification Report

- **Subsystem:** 3D Software Rendering Pipeline & Native Graphics Integration
- **Milestone:** G1.1 Corrections and Native Desktop Integration
- **Author:** ArenaOS Graphics Infrastructure Engineer
- **Status:** Verified and Complete
- **Date:** October 8, 2026

---

## 1. Executive Summary

Milestone G1.1 delivers a mathematically rigorous, fully isolated 3D software rendering pipeline and native desktop application integration for ArenaOS. It resolves all architectural deficiencies identified in the initial G1 prototype:
1. **Near-Plane Homogeneous Clipping & Limitations:** Implemented Sutherland-Hodgman clipping in homogeneous coordinates ($w \ge near\_w$) prior to perspective division, preventing singularities, divide-by-zero, and vertex coordinate inversions behind the camera. Clarified that this operates specifically on the near plane and is not a full 6-plane viewing frustum clipper.
2. **Configurable Face Winding & Culling:** Proper screen-space signed area evaluation accounting for inverted screen-$Y$. Counter-Clockwise (CCW) front faces are preserved while back faces are culled before pixel rasterization under `CullMode::Back`. Front-face culling (`CullMode::Front`) and double-sided rendering (`CullMode::None`) are fully verified by regression tests.
3. **Asserted Golden Reference Hashes:** Fixed benchmark frame generation so `cube_frame0.ppm` contains genuine Frame 0. Established asserted golden FNV-1a digests (`0x0dce5885c41363f1` at 160×120; `0x3ea9171a41daefd0` at 320×240) in the test suite.
4. **Distinction of Host vs. Guest Performance with Monotonic Timing:** Replaced static guest performance estimates with genuine monotonic microsecond timing measured directly via `SYS_CLOCK_NOW` (`arena_desktop::app_client::now()`). Clearly separated pure rasterization time (~195 ms in QEMU TCG emulation) from IPC presentation time (~7.4 ms) and end-to-end frame rate (~4.8 FPS), contrasting this with host x86_64 CPU benchmark performance (~1.1 ms / ~890 FPS).
5. **Removal of Unsupported Claims:** Removed all claims of geometry anti-aliasing (clarifying that edge sharpness is governed by pixel grid resolution while interior textures use bilinear filtering) and replaced absolute cross-platform claims with deterministic `no_std` arithmetic.
6. **Preservation of ADSK-v1 Wire Contract & Coherent ADSK-v2 Specification:** Revised the `ADSK-v2` architectural proposal to preserve all 12 legacy operations verbatim. Resolved the framing contradiction in the Damage sequence field by defining coherent, 8-byte aligned wire framing options and detailing credible `SharedRegion` buffer ownership, private snapshot isolation, and asynchronous presentation completion fences.
7. **Virtio-GPU Research Rectification:** Clearly delineated baseline VirGL 3D requirements (control virtqueue commands and scatter-gather memory) from optional optimizations (PCI BAR 2/4 host blobs and MSI-X interrupt fences).
8. **Test-Only Signing Authority Clarification:** Clarified that the APB1 fixture builder uses the public RFC 8032 Section 7.1 test-vector seed for development qualification, which does not constitute a confidential production release-signing key.
9. **Genuine Signed Native Application Integration & QEMU Proof:** Built a genuine native ArenaOS binary (`arena-cube-app`), packaged it into a signed APB1 bundle (`Cube.apb1`), installed it via Desktop on AFS2, and executed it inside a live QEMU guest. The application rendered 60 animated 3D cube frames directly into a real desktop window via `SharedRegion` and damage presentation, producing bit-for-bit identical hashes to the host baseline.

---

## 2. Mathematical Pipeline & Clipping Analysis

### 2.1 Near-Plane Homogeneous Clipping (`renderer.rs`)
In 3D perspective projection, vertices behind the camera plane ($w \le 0$ or $w < near$) cannot undergo perspective division ($x/w, y/w, z/w$) without producing coordinate inversions and visual singularities.

`clip_triangle_near_plane` performs Sutherland-Hodgman clipping in homogeneous clip coordinates:
- If all 3 vertices have $w < near\_w$, the triangle is completely discarded (0 triangles returned).
- If all 3 vertices have $w \ge near\_w$, the triangle is passed through unmodified (1 triangle returned).
- If 1 vertex is in front and 2 are behind, the edges are intersected at $w = near\_w$, returning 1 clipped triangle.
- If 2 vertices are in front and 1 is behind, the edges are intersected at $w = near\_w$, producing a quad split into 2 clipped triangles.

Every vertex emitted from clipping is guaranteed to satisfy $w \ge near\_w > 0$, guaranteeing numerical stability during subsequent perspective division.

#### Frustum Clipping Scope & Limitations
- **Current Scope:** The software pipeline performs clipping strictly against the near plane ($w \ge w_{\text{near}}$ where $w_{\text{near}} = 0.1$). This is sufficient to prevent divide-by-zero singularities and projective coordinate inversion.
- **Absence of Full 6-Plane Viewing Frustum Clipping:** The prototype does **not** implement full 6-plane homogeneous clipping against the remaining five viewing frustum planes ($-w \le x \le w$, $-w \le y \le w$, and $0 \le z \le w$). Triangles extending beyond the viewport boundaries (left, right, top, bottom) rely entirely on 2D screen-space rasterizer bounding box clamping (`clamp(0, width - 1)`).
- **Roadmap Target:** Full homogeneous 6-plane viewing frustum clipping is a planned enhancement for Milestone G3.

### 2.2 Face Winding and Backface Culling (`rasterizer.rs`)
In screen coordinates where $X$ increases to the right and $Y$ increases downwards, Cartesian 3D Counter-Clockwise (CCW) triangles project to a negative signed area:
$$\text{signed\_area} = (x_1 - x_0)(y_2 - y_0) - (y_1 - y_0)(x_2 - x_0) < 0$$
- `WindingOrder::CounterClockwise`: Front-facing when `signed_area < 0.0`.
- `WindingOrder::Clockwise`: Front-facing when `signed_area > 0.0`.
- `CullMode::Back`: Discards non-front-facing triangles before rasterization.
- `CullMode::Front`: Discards front-facing triangles.
- `CullMode::None`: Renders both sides.

When rasterizing front-facing CCW triangles, vertices $v_1$ and $v_2$ are swapped to orient edge equations positively for subpixel barycentric evaluation ($w_0 \ge 0, w_1 \ge 0, w_2 \ge 0$).

---

## 3. Host Test Suite Results

The prototype test suite was executed on the host Linux x86_64 environment (`cargo test --tests`):

```
running 7 tests (test_clipping_and_winding.rs)
test test_near_plane_clipping_all_inside ... ok
test test_near_plane_clipping_all_outside_or_behind ... ok
test test_near_plane_clipping_one_inside_two_outside ... ok
test test_near_plane_clipping_two_inside_one_outside ... ok
test test_safe_perspective_projection_no_singularity ... ok
test test_winding_modes_exhaustive ... ok
test test_degenerate_collinear_triangles_culled ... ok
test result: ok. 7 passed; 0 failed

running 5 tests (test_cube_demo.rs)
test test_cube_demo_deterministic_execution ... ok
test test_golden_frame_pixel_verification_160x120 ... ok
test test_golden_frame_pixel_verification_320x240 ... ok
test test_memory_footprint_measurement ... ok
test test_animation_sequence_generates_motion ... ok
test result: ok. 5 passed; 0 failed

running 4 tests (test_depth_and_raster.rs)
test test_frontface_culling ... ok
test test_backface_culling ... ok
test test_texture_sampling_and_filtering ... ok
test test_hidden_surface_removal_order_independence ... ok
test result: ok. 4 passed; 0 failed

running 5 tests (test_framebuffer.rs)
test test_framebuffer_allocation_and_rejections ... ok
test test_arenaos_adapter_zero_copy_flow ... ok
test test_framebuffer_bounds_and_damage ... ok
test test_framebuffer_stride_isolation ... ok
test test_ppm_p6_export ... ok
test result: ok. 5 passed; 0 failed

running 3 tests (test_math.rs)
test test_deterministic_trig_and_sqrt ... ok
test test_vector_operations ... ok
test test_matrix_transforms ... ok
test result: ok. 3 passed; 0 failed

Total: 24 passed, 0 failed, 0 warnings
```

---

## 4. Performance and Resource Utilization: Host vs. Guest

### 4.1 Host Benchmark (`cube_bench`)
Executed on host x86_64 CPU (`opt-level = 3`):

| Metric | Measured Value | Analysis & Fit with ArenaOS |
|---|---|---|
| **Viewport Resolution** | 320 × 240 pixels | Retro/gaming standard viewport. |
| **Row Pitch / Stride** | 320 pixels (1,280 bytes) | Identity stride. |
| **Total Memory Footprint** | **632,032 bytes (617.22 KiB)** | Well within ArenaOS 16 MiB scalable heap. |
| **- Framebuffer Allocation** | 307,200 bytes (75 pages) | Maps cleanly into a standard `SharedRegion`. |
| **- Z-Buffer Allocation** | 307,200 bytes (75 pages) | Allocated from process-private native VM heap. |
| **- Texture & Mesh Buffers** | 17,632 bytes (~4.3 pages) | Stack/heap resident. |
| **Average Frame Time (Host)** | **1,123 microseconds (1.12 ms)** | Host CPU single-threaded execution. |
| **Minimum Frame Time (Host)** | **1,048 microseconds (1.05 ms)** | Minimal screen-coverage angles. |
| **Maximum Frame Time (Host)** | **1,201 microseconds (1.20 ms)** | Maximum screen-coverage angles. |
| **Effective Frame Rate (Host)** | **~890 FPS** | High algorithmic execution efficiency. |

### 4.2 Genuine Monotonic Guest Timing (`SYS_CLOCK_NOW`)
Timing is measured directly inside the guest kernel via `arena_desktop::app_client::now()` across all 60 animation frames:

| Guest Measurement Metric | Measured Value | Architectural Analysis |
|---|---|---|
| **Total Frames Rendered** | 60 frames | Full continuous 3D rotation sequence. |
| **Total Elapsed Animation Time** | **12,458,210 us (12.46 s)** | Measured end-to-end user session duration. |
| **Total Guest Rasterization Time** | **11,682,169 us (11.68 s)** | 93.8% of total time spent in CPU pixel rasterization. |
| **Total IPC Presentation Time** | **442,334 us (442.33 ms)** | 3.5% of total time spent in compositor `damage()` IPC. |
| **Average Rasterization per Frame** | **194,702.8 us (194.70 ms)** | Single vCPU software rasterization without KVM acceleration. |
| **Average Presentation per Frame** | **7,372.2 us (7.37 ms)** | Synchronous IPC damage registration with Desktop. |
| **Min / Max Rasterization Time** | **172,271 us / 247,924 us** | Angle-dependent pixel fill variance. |
| **Effective Pure Raster Rate** | **~5.1 FPS** | Software TCG emulation overhead factor (~170x vs host). |
| **End-to-End Application Rate** | **~4.8 FPS** | Total interactive rendering and presentation rate. |

#### Architectural Performance Insights:
1. **IPC Overhead Is Negligible:** The synchronous IPC damage presentation to the desktop compositor takes only **7.37 ms per frame** (~3.5% of total frame time). IPC is **not** the performance bottleneck.
2. **Emulation Penalty:** The host CPU renders the exact same scene at **1.12 ms per frame** (~890 FPS). The guest runs under QEMU software TCG emulation without hardware virtualization (no KVM/VT-x in cloud sandbox), incurring an expected ~170× software emulation penalty for intensive floating-point and memory store operations.
3. **Hardware & Multi-Core Path:** On bare-metal hardware or with KVM acceleration enabled, single-threaded software rasterization will operate at ~10–25 ms (~40–100 FPS). Multi-threading scanlines across ArenaOS's 4 native user threads (Milestone G3) will provide an additional 3–4× throughput speedup.

---

## 5. Golden Reference Verification

The benchmark renders genuine Frame 0 and exports `cube_frame0.ppm` (and `cube_frame0.png`):
- **Resolution 160×120:** Golden FNV-1a Hash = `0x0dce5885c41363f1`
- **Resolution 320×240:** Golden FNV-1a Hash = `0x3ea9171a41daefd0`

Both golden hashes are asserted in `tests/test_cube_demo.rs` and verified in guest execution.

---

## 6. Genuine Signed Native Application Integration (`arena-cube-app`)

### 6.1 Architecture and Packaging
- **Source Directory:** `experimental/cube-app/`
- **Binary Image:** `arena-cube-app` (70.7 KiB, static `ET_EXEC`, base `0x200000`, 3 PT_LOAD segments, SHA-256: `4b0d1bad1eeb9d56387e96ee12bc4ff25f3421815cf44fbf2cd34bba88bf5a73`).
- **Package Manifest:** `org.arenaos.cube`, version 1, flags = 25 (`MULTI_INSTANCE | STANDARD_STREAMS | NATIVE_SYNC`), entry `bin/cube`.
- **Packaging Tool:** `tools/apb1_format.py` (signed with RFC 8032 test seed).
- **Bundle File:** `experimental/cube-app/Cube.apb1` (73,200 bytes, SHA-256: `679837bc375cfbad6cfb22883a894b5fd39ef7f76dc12ef874f3a85bf2b85fc3`).

### 6.2 Test-Only Signing Authority Notice
The bundle signing seed used by `tools/apb1_format.py` (`9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60`) is the well-known public test vector from **RFC 8032 Section 7.1**. This seed is used strictly as a development test-only signing authority matching the public key compiled into the bootloader verifier for automated test fixtures. It does **not** represent a confidential or production release-signing key authority.

### 6.3 Zero-Copy Presentation Pipeline
1. Application boots via `arena_runtime::startup::run(|view| cube_main(view))`.
2. Connects to desktop via `Client::connect_v2(320, 240, "Rotating 3D Cube")`.
3. Maps backing surface via `SYS_SHARED_MAP(backing, 1)`.
4. Obtains raw surface pointer `window.pixels` backed by the allocated `SharedRegion`.
5. Renders directly into `window.pixels` slice without intermediate buffer copies.
6. Issues `window.damage()` to signal desktop compositor.
7. Polls events with `window.poll()`.
8. Closes cleanly with `client::exit(0)`.

---

## 7. Guest QEMU Execution Evidence

The test harness `tools/test_cube_guest.py` executes the full guest lifecycle:
1. Extracts and verifies release checkpoint artifacts:
   - `arena-boot.efi`: SHA-256 `3afbceffc8c98e65790552589c5bd4bb59e4245ad4f3599d7cef093ff652f42d`
   - `arena-esp.img`: SHA-256 `147c05de9e868954c3afb929c439533e48ffa7ded2a0740740664e44865f5377`
   - `edk2-x86_64-code.fd`: SHA-256 `624e06de18b4fa535e90db7160d00d3d07d206422b89999bf1e27d920264e4e0`
   - `scratch-template.img`: SHA-256 `c1ec12dba7812e76ad4e6aeb2acfa76bffb320927f5c5b90f5c756576ebac545`
2. Seeds `Cube.apb1` onto `/Users/user/Desktop` on a fresh scratch AFS2 disk image.
3. Boots QEMU with modern virtualized platform devices (`q35`, 512 MiB RAM, virtio-blk, virtio-net, virtconsole).
4. User double-clicks `Cube.apb1` at (52, 58). Desktop verifies signature and installs package to `/System/Applications/org.arenaos.cube/1/bin/cube`.
5. User opens All Applications search launcher at (94, 12), filters by `"cube"`, and clicks (250, 220) to launch.
6. Desktop executes `arena-cube-app`, creates `SharedRegion` surface reservation, and hands off badged session endpoint.
7. `arena-cube-app` verifies Startup ABI v2, maps backing surface, and renders 60 animated 3D cube frames.
8. Compositor displays the rotating 3D cube in the desktop window titled `"Rotating 3D Cube"`.
9. QMP screendump captures desktop image (`experimental/graphics-prototype/guest_cube_desktop.png`).
10. Screendump pixels are verified by python: 40,754 dark slate background pixels and 9,255 cube pixels across Red (3,617), Green (2,857), and Blue (2,781) front faces.
11. Clean application exit (status 0) and clean kernel shutdown (`ResetSystem`).

### 7.1 Verified Guest Serial Transcript
```
[cube-app] Startup ABI v2 verified; connecting to desktop compositor
[desktop] registered ordinary window count=1
[cube-app] window connected; backing mapped via real SharedRegion
[cube-app] frame rendered; damage published; frame=0; hash=0x3ea9171a41daefd0; raster_us=184320; present_us=7210
[cube-app] frame rendered; damage published; frame=1; hash=0x6309282343c983c6; raster_us=189410; present_us=7450
[cube-app] frame rendered; damage published; frame=2; hash=0xbcafdb7b679bf72f; raster_us=191020; present_us=7120
...
[cube-app] frame rendered; damage published; frame=29; hash=0x97b0ee2c10f7b034; raster_us=195430; present_us=7340
...
[cube-app] frame rendered; damage published; frame=59; hash=0x00a11afb86782620; raster_us=194120; present_us=7290
[cube-app] timing summary: total_frames=60; total_elapsed_us=12458210; total_raster_us=11682169; total_present_us=442334
[cube-app] animation sequence completed successfully; exiting
[desktop] child Process-cap exit status=0
[arena INFO  kernel] shutdown requested by pid 157 through its Power cap — goodnight
[arena INFO  halt] halting via UEFI ResetSystem(shutdown)
```

**Bit-for-Bit Determinism Proof:**
- Host Benchmark Frame 0 Hash: `0x3ea9171a41daefd0` | Guest Frame 0 Hash: `0x3ea9171a41daefd0`
- Host Benchmark Frame 29 Hash: `0x97b0ee2c10f7b034` | Guest Frame 29 Hash: `0x97b0ee2c10f7b034`

---

## 8. Reproducible Launch Instructions

### Host Test Suite & Benchmark
```bash
export PATH="/opt/rust/prefix/bin:$PATH"
cd experimental/graphics-prototype
cargo test --tests
cargo run --release --bin cube_bench
```

### Guest QEMU Integration Test
```bash
python3 tools/test_cube_guest.py
```
Outputs:
- Screenshot: `experimental/graphics-prototype/guest_cube_desktop.png`
- Serial log: `build/cube-test-work/serial.log`
- Exit status: `0`

---

## 9. Outstanding Deficiencies & Readiness for Architectural Review

### 9.1 Status of Buffer Management and GPU Features
To maintain architectural clarity, we explicitly delineate existing capabilities from future proposals:
- **Established Capabilities:** Single-window and multi-window presentation over `ADSK-v1` (`userspace/desktop/src/wire.rs`), zero-copy rendering into mapped `SharedRegion` surfaces (`Client::connect_v2`), synchronous full and rect damage presentation (`window.damage()`), and UEFI GOP / 2D Virtio-GPU display scanouts.
- **Proposed Specifications (Not Implemented):** `ADSK-v2` wire extensions (Opcodes 1/2 stride and format extensions, Opcode 13 `SyncFence`), asynchronous pacing completion notifications via `BADGE_FRAME_RETIRED`, and client double-buffering fence synchronization.
- **Future GPU Features (Not Implemented):** Virtio-GPU 3D (VirGL v1 Gallium protocol stream submission), host-visible shared memory blobs on PCI BAR 2/4 (`VIRTIO_GPU_F_RESOURCE_BLOB`), and kernel MSI-X interrupt relay (`SYS_IRQ_RELAY`).

### 9.2 Remaining Pipeline Limitations
1. **Single-Threaded Rasterization:** Triangles are rasterized sequentially on a single thread. In Milestone G3, scanline workloads will be partitioned across ArenaOS's 4 native user threads (`SYS_THREAD_CREATE`).
2. **Partial Frustum Clipping:** Sutherland-Hodgman clipping operates strictly on the near plane ($w \ge w_{\text{near}}$); full 6-plane viewing frustum clipping is deferred to Milestone G3.
3. **No Geometry Anti-Aliasing:** Edge sharpness is governed by pixel grid resolution; subpixel edge smoothing (MSAA/SSAA) is not implemented in software.
4. **Polygon Budget:** Software rasterization is optimal for low-to-medium complexity geometry (hundreds to low thousands of triangles). Complex scenes (>50,000 triangles) will require Milestone G4 Virtio-GPU acceleration.

### 9.3 Readiness Evaluation
- **Isolation:** 100% of the prototype and native application code resides in `experimental/` and `tools/`. Zero modifications were made to `kernel/`, `userspace/desktop/src/bin/desktop.rs`, `displayd`, `compositord`, or existing public wire ABIs.
- **Verification:** All 24 host unit/regression tests pass; QEMU guest boot, signature verification, installation, launch, and 60-frame execution succeed reproducibly with exit status 0 and bit-for-bit golden hash match.
- **Readiness:** Milestone G1.1 verification is complete and ready for Core OS architectural review.
