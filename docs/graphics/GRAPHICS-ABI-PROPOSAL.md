# ADR Proposal: ArenaOS Native Graphics ABI v2 (`ADSK-v2`)

- **Status:** Proposed (Under Review — Do not merge into production until Phase 14 qualification)
- **Author:** ArenaOS Graphics Infrastructure Engineer
- **Target Subsystem:** Desktop Compositor (`desktop`), Application Runtime (`arena-runtime`), Display Service (`displayd`)
- **Compatibility Baseline:** Qualified Phase 13 (`arena/phase13-native-app-maturity`, tip `74ace4f9e989`)

---

## 1. Context and Problem Statement

In ArenaOS Phase 13, applications interact with the desktop via the neutral 64-byte `ADSK` inline IPC frame protocol (`userspace/desktop/src/wire.rs`). While this protocol successfully supports built-in 2D applications and basic multi-window lifecycle (ADR-0097), it presents several friction points when targeting standard graphics libraries (SDL3, software OpenGL, future 3D drivers):

1. **Uncertain Frame Synchronization:** The existing `Frame::Damage` call synchronously blocks the caller while the compositor executes `publish_rects`. There is no asynchronous completion token or fence mechanism, limiting render pipelines to synchronous single-buffered lockstep.
2. **Fixed Pixel Format:** The wire format implicitly assumes 32-bit opaque `XRGB8888`. It provides no negotiation for alpha blending (`ARGB8888`), 16-bit color (`RGB565`), or explicit color-space metadata.
3. **Implicit Buffer Stride:** Surface dimensions in `Frame::Create` supply only `width` and `height`, implicitly assuming `stride == width`. Standard rendering engines require explicit row pitch/stride to satisfy cacheline alignment and SIMD requirements.
4. **Resizing State Transitions:** While ADR-0075 introduces session surface reservations, transitioning between window resize drag and active rendering lacks an atomic buffer swap and fence acknowledgment.

---

## 2. Decision & Technical Specification

We propose `ADSK-v2` (ArenaOS Desktop Session Protocol Version 2), a capability-guarded, versioned graphics interface preserving ArenaOS's security invariants while providing standard primitives for rendering engines.

### 2.1 Capability-Based Resource Ownership & Security Model

```
┌─────────────────────────────────────────────────────────────┐
│                    KERNEL CAPABILITY SPACE                  │
├──────────────────────────────┬──────────────────────────────┤
│ Desktop Process Holds:       │ Client Process Holds:        │
│ - Desktop Session Endpoint   │ - Badged Client Endpoint     │
│   (Server side: READ)        │   (Caller side: WRITE)       │
│ - SharedRegion Cap (Backing) │ - SharedRegion Cap (Backing) │
│ - Private Snapshot Map (Pin) │   (RW / NX Mapping)          │
│ - Client Process Cap         │ - Pacing Notification Cap    │
│ - Client Pacing Notification │                              │
└──────────────────────────────┴──────────────────────────────┘
```

1. **Authority by Possession Only:**
   - Window creation, damage presentation, and destruction require possessing an exact `BadgedEndpoint` capability minted by Desktop.
   - Numeric window handles (`u64`) are purely descriptive routing identifiers. Passing a numeric handle without the corresponding badged endpoint capability returns `STATUS_BAD_ARG` (-2).
2. **Buffer Protection & Isolation:**
   - The application receives its surface backing as a `SharedRegion` capability with attenuated rights `RIGHTS_READ | RIGHTS_WRITE | RIGHTS_COPY | RIGHTS_DESTROY`.
   - The application has zero access to the compositor's scanout buffer, other application surfaces, or device MMIO.
   - The compositor maintains a **private snapshot region** for each window. The compositor never renders directly from the application's shared buffer, isolating the display from application memory corruption.
3. **Fail-Closed Teardown:**
   - If an application terminates or crashes, the kernel automatically unmaps its shared memory and releases capability references (`docs/adr/0056`).
   - Desktop observes client termination via its held `Process` capability, reaps the window record, destroys its private snapshot mapping, and marks the window slot vacant.

---

### 2.2 Wire Protocol Specification (`ADSK-v2`)

All client-compositor requests cross the existing 64-byte inline IPC payload structure. The magic header is updated to `ADSK`, version `2`.

```
Bytes 0..4:   ASCII Magic 'ADSK'
Byte  4:      Version = 2
Byte  5:      Opcode
Byte  6..7:   Flags / Format (0 = XRGB8888, 1 = ARGB8888, 2 = RGB565)
Bytes 8..15:  Window Handle (u64 LE, descriptive)
Bytes 16..23: Sequence / Fence Token (u64 LE)
Bytes 24..27: Width (u16 LE), Height (u16 LE)
Bytes 28..31: Stride in Pixels (u16 LE), Reserved (u16 zero)
Bytes 32..63: Opcode-specific payload / Damage Rectangles / Reserved zero
```

#### Supported Operations

| Opcode | Name | Description |
|---|---|---|
| `1` | `Create` | Create the primary window surface for this authenticated session. |
| `2` | `CreateAdditional` | Request an additional independently backed ordinary window (ADR-0097). |
| `3` | `Damage` | Publish modified rectangles and commit rendering. |
| `4` | `Poll` | Retrieve queued input and window events. |
| `5` | `Resize` | Adopt newly configured window dimensions. |
| `6` | `Title` | Set descriptive window title string. |
| `7` | `DestroyWindow` | Explicitly close an individual window while keeping the process alive. |
| `8` | `SyncFence` | Query or wait for completion of a previously submitted damage sequence. |

---

### 2.3 Buffer Geometry, Pitch, and Formats

1. **Dimensions:**
   - Strictly bounded: `MIN_WIDTH = 80`, `MIN_HEIGHT = 60`, `MAX_WIDTH = 1024`, `MAX_HEIGHT = 768`.
   - Oversized or zero dimensions are rejected before resource mutation.
2. **Explicit Stride:**
   - `stride` must be $\ge \text{width}$ and aligned to a 16-pixel (64-byte) cacheline boundary.
   - Allows graphics rasterizers to employ SIMD vector stores without corrupting adjacent rows.
3. **Pixel Formats:**
   - `Format 0 (XRGB8888)`: 32 bits per pixel, opaque, byte order B, G, R, X in memory. (Default standard format).
   - `Format 1 (ARGB8888)`: 32 bits per pixel, straight or premultiplied alpha for transparent overlays.
   - `Format 2 (RGB565)`: 16 bits per pixel, packed direct color for memory-constrained clients.

---

### 2.4 Presentation, Damage, and Completion Fences

To enable high-performance double-buffering without tearing or unbounded blocking:

1. **Damage Rectangles:**
   - `Damage` carries up to 4 discrete bounding rectangles `[x, y, width, height]` plus an accumulation flag (`DamageRects::MAX = 4`).
   - If `n == 0` or overflow occurs, it degrades safely to full-window damage.
2. **Monotonic Frame Sequence Numbers:**
   - The application increments a 64-bit monotonic sequence counter with each `Damage` request.
   - The compositor acknowledges receipt with the sequence number.
3. **Asynchronous Presentation Fence:**
   - When the compositor finishes copying pixels into its private snapshot, it signals the application's private pacing Notification capability with badge `BADGE_FRAME_RETIRED | (seq & 0xFFFF)`.
   - The application can immediately reuse its back-buffer memory without polling or tearing.

---

### 2.5 Window Resizing Protocol

Resizing leverages the Phase 11.3 / ADR-0075 pre-reserved surface capacity:

1. **Event Delivery:** When the user resizes a window frame, Desktop queues an `Event::Configure { width, height }` to the client.
2. **Adoption:** The client invokes `Client::adopt(new_w, new_h)` to reconfigure its internal rasterizer and viewport. Because the initial reservation covers the full screen work area, no memory re-allocation or capability re-negotiation occurs.
3. **Commit:** The client repaints the new dimensions and sends `Frame::Resize { width, height }`, instantly re-synchronizing compositor bounds.

---

### 2.6 Input Event Delivery

Input routing preserves the verified Phase-10 model (`docs/adr/0062`):

- **Event Queue:** Each window session maintains an isolated 16-slot FIFO ring.
- **Typed Delivery:** `Event::Key(code, pressed, mods)`, `Event::Pointer(x, y, buttons)`, `Event::Wheel(delta)`, `Event::Configure(w, h)`, and `Event::Close`.
- **Zero Raw Device Access:** Applications never receive raw `inputd` tokens, hardware scancodes, or global pointer coordinates.

---

## 3. Security Analysis

| Threat Model Vector | Mitigation in Proposed ABI |
|---|---|
| **Buffer Overrun / Out-of-Bounds Write** | Kernel validates page counts on `SYS_SHARED_CREATE`. Framebuffer wrappers enforce bounds checks on slice dimensions and strides. |
| **Cross-Client Information Leak** | Kernel zeroes all physical frames upon allocation before minting capabilities. |
| **Compositor Hijacking / Forgery** | Operations require possession of exact unforgeable `BadgedEndpoint` capabilities. Descriptive IDs cannot authenticate actions. |
| **Denial of Service via Unbounded Allocation** | Bounded reservations: max 32 windows system-wide, max 1024 pages per surface, max 96 total system SharedRegions. |
| **Display Memory Corruption** | Framebuffer MMIO is held exclusively by `displayd`. Client applications possess zero physical DMA or MMIO capabilities. |

---

## 4. Implementation Rule

**This proposal is an engineering design deliverable. Production kernel and userspace interfaces must NOT be modified until formal review alongside Phase-14 qualification.**
