# ADR Proposal: ArenaOS Desktop Session Protocol Version 2 (`ADSK-v2`)

- **Status:** Proposed Architecture Draft (NOT approved for production implementation)
- **Author:** ArenaOS Graphics Infrastructure Engineer
- **Target Subsystem:** Desktop Compositor (`desktop`), Application Runtime (`arena-runtime`), Display Service (`displayd`)
- **Compatibility Baseline:** Qualified Phase 13 (`arena/phase13-native-app-maturity`, commit `74ace4f9e989`)
- **Strict Implementation Constraint:** This document specifies an architectural proposal for future review. Production kernel, desktop, and wire interfaces must NOT be modified. All legacy ADSK-v1 operations and wire frames must be preserved verbatim.

---

## 1. Context and Problem Statement

In ArenaOS Phase 13, applications interact with the desktop compositor (`desktop`) via the canonical 64-byte `ADSK` inline IPC frame protocol (`userspace/desktop/src/wire.rs`). This protocol governs surface creation, input delivery, transient popups, and damage presentation for native applications.

While the existing `ADSK-v1` protocol successfully qualifies single and multi-window desktop applications (ADR-0097, ADR-0107), targeting standard 2D/3D libraries (e.g. SDL3, Mesa software rasterizers, and future hardware acceleration) exposes key protocol limitations:

1. **Synchronous Presentation Lockstep:** In `ADSK-v1`, `Frame::Damage` carries damage rectangles and synchronously triggers compositor consumption. There is no asynchronous completion token, presentation fence, or notification mechanism indicating when the compositor has finished reading the buffer. Consequently, clients cannot reliably pipeline double-buffered rendering without risking tearing or blocking.
2. **Implicit Pixel Format:** `ADSK-v1` implicitly treats all surfaces as 32-bit opaque `XRGB8888`. It provides no negotiation for alpha blending (`ARGB8888`), 16-bit packed modes (`RGB565`), or linear color spaces.
3. **Implicit Row Stride:** `Frame::Create` and `Frame::CreateAdditional` accept only `width` and `height`, enforcing `stride == width`. Vectorized SIMD software rasterizers and hardware memory controllers require row pitch alignment (typically 16-pixel / 64-byte boundaries).
4. **Wire Compatibility Requirement:** Millions of cycles of guest qualification rely on the exact ADSK-v1 wire format. Any revision must preserve all 12 existing opcodes, error handling, and payload semantics without breaking legacy clients.

---

## 2. ADSK-v1 Baseline Audit

The verified `ADSK-v1` wire layout (`userspace/desktop/src/wire.rs`) defines a fixed 64-byte structure:

```
Bytes  0..4:   b"ADSK" (ASCII Magic)
Byte   4:      1 (Wire Version)
Byte   5:      Opcode (1..=12)
Bytes  6..7:   Reserved (strictly checked zero)
Bytes  8..16:  Window Handle (u64 LE, descriptive routing token)
Bytes 16..24:  Opcode payload (damage count, coordinates, or dismissed popup handle)
Bytes 24..28:  Width/Height, or Keycode, or Min Dimensions
Bytes 28..30:  Event sub-opcode / flags
Bytes 30..32:  Reserved (strictly checked zero)
Bytes 32..64:  Title text (32 bytes) or DamageRects payload
```

### Full Legacy Opcode Table (ADSK-v1)

| Opcode | Legacy Operation | ADSK-v1 Field Usage | Status in Proposal |
|---|---|---|---|
| `1` | `Create` | `b[24..26]=width`, `b[26..28]=height` | **Preserved 100%** |
| `2` | `Damage` | `b[16]=n`, `b[24..64]=r[5][4]` | **Preserved 100%** |
| `3` | `Poll` | `b[8..16]=handle` | **Preserved 100%** |
| `4` | `Title` | `b[8..16]=handle`, `b[32..64]=text[32]` | **Preserved 100%** |
| `5` | `Event` | `b[8..16]=handle`, `b[28]=kind`, `b[24..28]=args` | **Preserved 100%** |
| `6` | `CancelClose` | `b[8..16]=handle` | **Preserved 100%** |
| `7` | `Resize` | `b[8..16]=handle`, `b[24..26]=w`, `b[26..28]=h` | **Preserved 100%** |
| `8` | `Resizable` | `b[8..16]=handle`, `b[24..26]=min_w`, `b[26..28]=min_h` | **Preserved 100%** |
| `9` | `Popup` | `b[8..16]=handle`, `b[16..24]=x,y`, `b[24..28]=w,h`, `b[28]=kind` | **Preserved 100%** |
| `10` | `Dismiss` | `b[8..16]=handle` | **Preserved 100%** |
| `11` | `DestroyWindow` | `b[8..16]=handle` | **Preserved 100%** |
| `12` | `CreateAdditional`| `b[24..26]=width`, `b[26..28]=height` | **Preserved 100%** |

---

## 3. Proposed ADSK-v2 Extensions (Additive & Non-Breaking)

To maintain absolute backward compatibility while introducing graphics extensions, `ADSK-v2` introduces:
1. Version Negotiation: `b[4]` can be negotiated as `1` (legacy ADSK-v1) or `2` (`ADSK-v2`).
2. Additive Fields: Using currently reserved bytes in `Create`, `CreateAdditional`, and `Damage` when version is 2.
3. New Additive Opcode: Opcode `13` (`SyncFence`) for non-blocking presentation query.

### 3.1 Extended Wire Framing (`version == 2`)

```
Bytes  0..4:   b"ADSK" (Magic)
Byte   4:      2 (Version)
Byte   5:      Opcode (1..=13)
Bytes  6..7:   Pixel Format:
                 0 = XRGB8888 (32-bit opaque default)
                 1 = ARGB8888 (32-bit premultiplied alpha)
                 2 = RGB565   (16-bit packed direct color)
Bytes  8..16:  Window Handle (u64 LE, descriptive routing token)
Bytes 16..24:  Sequence / Presentation Token (u64 LE for Damage/SyncFence)
Bytes 24..26:  Width (u16 LE)
Bytes 26..28:  Height (u16 LE)
Bytes 28..30:  Row Pitch / Stride in Pixels (u16 LE; 0 defaults to width)
Bytes 30..32:  Reserved (zero)
Bytes 32..64:  Opcode payload (DamageRects, Title, Popup, or Fence metadata)
```

### 3.2 Operation Extensions

#### Opcode 1 (`Create`) & Opcode 12 (`CreateAdditional`) in v2
- `b[6..7]`: Requested `PixelFormat`. If the requested format is unsupported, Desktop responds with `XRGB8888` or returns `STATUS_BAD_ARG`.
- `b[28..30]`: Requested `stride`. Must be $\ge \text{width}$ and a multiple of 16 pixels. If 0, compositor defaults `stride = width`.

#### Opcode 2 (`Damage`) in v2
- `b[8..16]`: Window handle.
- `b[16]`: `rects.n` (0..=5).
- `b[17..24]`: Monotonic frame submission token `seq: u64`.
- `b[24..64]`: Up to 5 damage bounding rectangles `[x, y, w, h]`.

#### Opcode 13 (`SyncFence`) — Additive New Opcode
- Used by the client to query presentation status or register a notification callback for a submitted sequence token.
- `b[8..16]`: Window handle.
- `b[16..24]`: Sequence token `seq: u64`.
- `b[24..26]`: Action (`0 = QueryStatus`, `1 = RequestAsyncNotification`).

---

## 4. Credible Buffer Ownership & Memory Protection

```
┌─────────────────────────────────────────────────────────────┐
│                    KERNEL CAPABILITY SPACE                  │
├──────────────────────────────┬──────────────────────────────┤
│ Desktop Process:             │ Client Application Process:  │
│ - Desktop Session Server Port│ - Badged Client Endpoint     │
│   (Receives ADSK requests)   │   (Sends ADSK requests)      │
│ - SharedRegion Cap (Backing) │ - SharedRegion Cap (Backing) │
│   (Attenuated transfer)      │   (Mapped RW, NX)            │
│ - Private Snapshot Map (Pin) │ - Client Pacing Notification │
│   (Double-buffer protection) │   (Receives FRAME_RETIRED)   │
│ - Client Process Cap (Reap)  │                              │
└──────────────────────────────┴──────────────────────────────┘
```

1. **Allocation & Attenuation:**
   - The surface backing is created by `desktop` via `SYS_SHARED_CREATE(pages)`.
   - `desktop` retains the master capability and passes an attenuated capability (`RIGHTS_READ | RIGHTS_WRITE | RIGHTS_COPY | RIGHTS_DESTROY`) to the client during session handshake.
   - The application maps the region into its user address space via `SYS_SHARED_MAP(backing, VM_PROT_READ | VM_PROT_WRITE)`.
2. **Double-Buffering & Snapshot Isolation:**
   - In ArenaOS, the compositor never composites directly from live application shared memory during screen scanout to avoid race conditions, torn frames, and malicious memory modification.
   - Upon receiving `Frame::Damage`, `desktop` blits only the damaged rectangles into its private compositor snapshot buffer.
   - The display controller (`displayd`) reads exclusively from the desktop compositor's scanout buffer via unmapped physical DMA / MMIO. Application processes have zero capability access to display hardware.
3. **Clean Resource Reclamation:**
   - If an application exits or faults, the microkernel reclaims all mapped pages and decrements capability reference counts (`docs/adr/0056`).
   - Desktop detects process termination via its held `Process` handle, reaps the window slot, and frees the `SharedRegion`.

---

## 5. Presentation Completion & Synchronization Model

To avoid unbounded synchronous blocking and eliminate frame tearing:

1. **Asynchronous Notification Badge:**
   - On startup, the application receives a badged `Notification` capability minted from the system notification broker.
   - In `ADSK-v2`, when the desktop compositor finishes blitting the damaged regions of sequence `seq` into its private snapshot, it signals the client's pacing notification:
     ```
     SYS_NOTIFY(client_pacing_slot, BADGE_FRAME_RETIRED | (seq & 0xFFFF))
     ```
2. **Pipelined Double Buffering:**
   - The client application renders frame $N$ into back-buffer partition 0, issues `Damage(seq=N)`.
   - The client immediately begins rendering frame $N+1$ into back-buffer partition 1 without waiting.
   - Before publishing frame $N+1$, the client checks its notification badge or calls `SYS_WAIT` on the pacing notification to ensure frame $N$ has been retired by the compositor.
   - Result: Full 60 FPS throughput with zero tearing and minimal IPC latency.

---

## 6. Implementation Rule & Roadmap Alignment

- **Scope:** Architectural specification only.
- **Production Status:** `ADSK-v1` remains the sole production desktop protocol in ArenaOS Phase 13.
- **Review Requirement:** Any changes to `userspace/desktop/src/wire.rs` or compositor message processing require formal architectural review and qualification alongside Phase 14 core OS milestones.
