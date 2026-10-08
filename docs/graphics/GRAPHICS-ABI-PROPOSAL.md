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
Bytes 16..24:  Opcode payload (damage count n: u8 at b[16], padding b[17..24], or coordinates/dismissed handle)
Bytes 24..28:  Width/Height, or Keycode, or Min Dimensions
Bytes 28..30:  Event sub-opcode / flags
Bytes 30..32:  Reserved (strictly checked zero)
Bytes 32..64:  Title text (32 bytes) or DamageRects payload (r: [[u16; 4]; 5] spanning bytes 24..64)
```

### Full Legacy Opcode Table (ADSK-v1)

| Opcode | Legacy Operation | ADSK-v1 Field Usage | Status in Proposal |
|---|---|---|---|
| `1` | `Create` | `b[24..26]=width`, `b[26..28]=height` | **Preserved 100%** |
| `2` | `Damage` | `b[16]=n`, `b[17..24]=zero pad (7 bytes)`, `b[24..64]=r[5][4]` | **Preserved 100%** |
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

## 3. Proposed ADSK-v2 Extensions (Coherent Wire Layout)

### 3.1 Resolving the Damage Frame Framing Contradiction

In the initial draft proposal, a framing error attempted to allocate `b[17..24]` to an 8-byte `u64` sequence token while retaining 5 damage rectangles spanning `b[24..64]`. Because $24 - 17 = 7$ bytes, placing an 8-byte integer in a 7-byte window was an arithmetic contradiction.

We specify two fully coherent, mathematically consistent layouts:

#### Option A (Recommended Primary Layout: Aligned 64-bit Sequence Token):
When `version == 2`, the 64-byte `Frame::Damage` structure is defined with natural 8-byte alignment:
```
Bytes  0..4:   b"ADSK" (Magic)
Byte   4:      2 (Wire Version)
Byte   5:      2 (Opcode Damage)
Bytes  6..7:   Flags (b[6]: PresentationFlags, b[7]: Reserved zero)
Bytes  8..16:  Window Handle (u64 LE, 8-byte aligned at offset 8)
Bytes 16..24:  Sequence Token (u64 LE, 8-byte aligned at offset 16)
Byte  24:      Damage Rect Count n (u8, 0..=4)
Bytes 25..32:  Reserved zero (7 bytes padding)
Bytes 32..64:  Damage Rectangles r: [[u16; 4]; 4] (4 rects * 8 bytes = 32 bytes, covering b[32..64])
```
*Benefits:* Both `handle` and `seq` are naturally aligned 64-bit integers. Four discrete damage rectangles ($320\text{ bits}$) provide sufficient granularity for dirty region updates before coalescing into the bounding union rectangle.

#### Option B (Alternative Layout: Aligned 32-bit Sequence Token with 5 Rectangles):
If retaining all 5 legacy damage rectangles ($5 \times 8 = 40$ bytes at `b[24..64]`) is preferred:
```
Bytes  0..4:   b"ADSK" (Magic)
Byte   4:      2 (Wire Version)
Byte   5:      2 (Opcode Damage)
Bytes  6..7:   Flags / Reserved zero
Bytes  8..16:  Window Handle (u64 LE)
Byte  16:      Damage Rect Count n (u8, 0..=5)
Byte  17:      Presentation Flags
Bytes 18..20:  Reserved zero (2 bytes padding)
Bytes 20..24:  Sequence Token (u32 LE, 4-byte aligned at offset 20)
Bytes 24..64:  Damage Rectangles r: [[u16; 4]; 5] (5 rects * 8 bytes = 40 bytes)
```
*Benefits:* Retains exact 5-rectangle capacity of ADSK-v1. A 32-bit sequence token accommodates $4,294,967,296$ unique frames (over 820 days of continuous 60 FPS presentation before monotonic rollover).

---

### 3.2 Extended Creation & Fence Operations (`version == 2`)

#### Opcode 1 (`Create`) & Opcode 12 (`CreateAdditional`) in v2
```
Bytes  0..4:   b"ADSK" (Magic)
Byte   4:      2 (Version)
Byte   5:      1 or 12 (Create / CreateAdditional)
Byte   6:      PixelFormat (0 = XRGB8888, 1 = ARGB8888, 2 = RGB565)
Byte   7:      ColorSpace (0 = sRGB linear, 1 = display gamma)
Bytes  8..16:  Reserved zero
Bytes 16..24:  Reserved zero
Bytes 24..26:  Width in pixels (u16 LE, 80..=1024)
Bytes 26..28:  Height in pixels (u16 LE, 60..=768)
Bytes 28..30:  Row Pitch / Stride in pixels (u16 LE; 0 defaults to width)
Bytes 30..32:  Reserved zero
Bytes 32..64:  Reserved zero
```

#### Opcode 13 (`SyncFence`) — Additive New Opcode
Provides a non-blocking query mechanism for client rendering fences:
```
Bytes  0..4:   b"ADSK" (Magic)
Byte   4:      2 (Version)
Byte   5:      13 (SyncFence)
Bytes  6..7:   Action (0 = QueryStatus, 1 = RegisterNotification)
Bytes  8..16:  Window Handle (u64 LE)
Bytes 16..24:  Sequence Token seq (u64 LE)
Bytes 24..32:  Reserved zero
Bytes 32..64:  Reserved zero
```

---

## 4. Credible Buffer Ownership & Memory Protection

```
┌─────────────────────────────────────────────────────────────┐
│                    KERNEL CAPABILITY SPACE                  │
├──────────────────────────────┬──────────────────────────────┤
│ Desktop Compositor Process:  │ Client Application Process:  │
│ - Desktop Session Server Port│ - Badged Client Endpoint     │
│   (Receives ADSK requests)   │   (Sends ADSK requests)      │
│ - SharedRegion Cap (Backing) │ - SharedRegion Cap (Backing) │
│   (Retains master authority) │   (Mapped RW, NX)            │
│ - Private Snapshot Map (Pin) │ - Client Pacing Notification │
│   (Double-buffer protection) │   (Receives FRAME_RETIRED)   │
│ - Client Process Cap (Reap)  │                              │
└──────────────────────────────┴──────────────────────────────┘
```

1. **Surface Allocation & Attenuation:**
   - The surface backing is allocated by `desktop` via `SYS_SHARED_CREATE(pages)`.
   - `desktop` holds the master capability and delegates an attenuated capability (`RIGHTS_READ | RIGHTS_WRITE | RIGHTS_COPY | RIGHTS_DESTROY`) to the client during session handshake.
   - The application maps the region into its user address space via `SYS_SHARED_MAP(backing, VM_PROT_READ | VM_PROT_WRITE)`.
2. **Snapshot Isolation & Tearing Avoidance:**
   - In ArenaOS, the compositor never composites directly from live application shared memory during scanout to prevent data races and visual artifacts.
   - Upon receiving `Frame::Damage`, `desktop` blits only the declared damaged rectangles from the application's `SharedRegion` into Desktop's private snapshot buffer (`snapshot`).
   - The display controller (`displayd`) reads exclusively from Desktop's scanout buffer via unmapped physical MMIO/DMA. Ordinary applications possess zero capability access to display hardware.
3. **Fail-Closed Teardown:**
   - If an application exits or faults, the microkernel reclaims all mapped pages and decrements capability reference counts (`docs/adr/0056`).
   - Desktop detects process termination via its held `Process` capability, destroys the snapshot mapping, reaps the window slot, and frees the `SharedRegion`.

---

## 5. Presentation Completion & Synchronization Semantics

To eliminate synchronous blocking and prevent buffer tearing:

1. **Asynchronous Pacing Notification:**
   - On startup, the application receives a dedicated `Notification` capability minted from the system notification broker.
   - In `ADSK-v2`, when Desktop finishes copying the damaged regions of sequence `seq` into its private snapshot buffer, Desktop signals the client's pacing notification:
     ```
     SYS_NOTIFY(client_pacing_slot, BADGE_FRAME_RETIRED | (seq & 0xFFFF))
     ```
2. **Pipelined Double-Buffering Flow:**
   - The client application partitions its mapped surface into two sub-buffers (Buffer 0 and Buffer 1).
   - Step 1: Render Frame $N$ into Buffer 0.
   - Step 2: Issue `Frame::Damage { handle, seq: N, rects }`.
   - Step 3: Immediately begin rendering Frame $N+1$ into Buffer 1 without waiting for Desktop.
   - Step 4: Before submitting Frame $N+1$, verify that Frame $N$ has been retired:
     - Non-blocking: Poll the notification badge via `SYS_POLL_NOTIFICATION`.
     - Blocking: Park via `SYS_WAIT(pacing_slot)` if the compositor has not yet finished reading Buffer 0.
   - Result: Full 60 FPS presentation throughput with zero tearing and minimal microkernel IPC latency.

---

## 6. Implementation Rule & Roadmap Alignment

- **Scope:** Architectural specification only.
- **Production Status:** `ADSK-v1` remains the sole production desktop protocol in ArenaOS Phase 13.
- **Review Requirement:** Any changes to `userspace/desktop/src/wire.rs` or compositor message processing require formal architectural review and qualification alongside Phase 14 core OS milestones.
