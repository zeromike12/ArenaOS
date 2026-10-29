# V1 API contract table — from source (userspace/abi.rs, kernel syscall.rs)

**Source of truth:** `userspace/abi.rs` (the single ABI file included by every
service crate binary) and `kernel/kernel/src/arch/x86_64/syscall.rs` (the
dispatcher; numbers mirror the userspace registry). This table is a
descriptive record of actual API semantics — not a grant of new authority.
**Milestone:** V1 — Native Graphical Launch Foundation.

## 1. Syscall registry (ABI v1)

ABI v1: RAX = call number, RDI/RSI/RDX/R10/R8/R9 = up to six arguments,
result in RAX as typed i64 status (0 = OK, positive = call-specific
payload, negative = typed error). RBX/RBP/R12–R15 preserved; RCX/R11
consumed by hardware. Unused trailing registers must be zeroed by the
caller (the kernel validates R9 as an inline-message pointer for IPC v1.1
calls).

| Call | Nr | Arguments | Result / semantics |
|---|---|---|---|
| SYS_DEBUG_WRITE | 1 | buf, len | Copy `buf[..len]` (≤256 B, page-validated, SMAP-aware) to the serial console. Returns accepted byte count; STATUS_BAD_ARG / STATUS_BAD_ADDRESS otherwise. Temporary diagnostics backdoor; not stdio. |
| SYS_THREAD_EXIT | 2 | status | Diverges: records exit, terminates the thread/process through the scheduler. Never returns. |
| SYS_PROVE_RING3 | 4 | frame | Arms a #GP expectation at the user's next instruction (M3 proof call). |
| SYS_PROVE_DONE | 5 | — | Records whether the armed #GP was delivered; returns 1/0. |
| SYS_ABI_ECHO6 | 6 | a0..a5 | Fingerprint of the six received argument registers (marshalling proof). |
| SYS_IPC_CALL | 7 | ep slot, w0, w1, send-cap slot, reply[3] ptr, msg64 ptr | Blocking endpoint rendezvous; WRITE on the endpoint. Reply `[w0, w1, cap-slot-or-CAP_NONE]` plus optional 64-byte inline message (IN/OUT, v1.1). **No client-side deadline; blocking.** Only STATUS_BUSY retries are bounded. |
| SYS_IPC_RECV | 8 | ep slot, buf[3] ptr, msg64 ptr | Blocking receive; READ on the endpoint. |
| SYS_IPC_TRY_RECV | 42 | ep slot, buf[3] ptr, msg64 ptr | Nonblocking receive (a3..a5 must be 0). |
| SYS_IPC_REPLY | 9 | ep slot, w0, w1, send-cap slot, msg64 ptr | Stage a reply; READ on the endpoint. |
| SYS_IPC_REPLY_CHECKED | 43 | … | As REPLY with policy checks. |
| SYS_IPC_RECV_BADGED | 49 | ep slot, out[4] ptr, msg64 ptr, blocking, 0 | RECV/TRYS_RECV returning `[w0, w1, landed-cap-slot, badge]` (word 3). Serve side. (a3 ≤ 1, a4 == 0 required.) |
| SYS_NOTIFY | 10 | notif slot, badge | OR `badge` into the pending word; wake waiter. Nonblocking. WRITE on the notification. |
| SYS_WAIT | 11 | notif slot | Take pending merged badge (positive payload); block while zero. READ on the notification. |
| SYS_TRY_WAIT | 32 | notif slot | Take pending badge or return 0; never parks. |
| SYS_SPAWN | 12 | image slot, spec ptr, spec count, notif slot|CAP_NONE, badge | Spawn a process from an image cap; explicit attenuated inheritance. Returns child pid (positive payload). |
| SYS_CONSOLE_READ | 13 | buf, max | Blocking console line read; count returned (terminator excluded). max ≤ LINE_MAX. |
| SYS_PROC_LIST | 14 | buf, max_pairs | Write `(pid, live_threads)` pairs; returns pair count. |
| SYS_SHUTDOWN | 15 | power slot | Halt via UEFI ResetSystem(shutdown); CapObj::Power + WRITE required. |
| SYS_ALLOC_FRAME | 16 | slot | Mint an owned Untyped cap over one allocated frame in an empty slot; returns phys address. STATUS_BUSY on exhaustion/occupied slot. |
| SYS_MAP_MEMORY | 17 | slot, writable | Self-map Untyped (ownership transfers) or Mmio cap at a kernel-chosen VA (1 GiB base, 2 MiB strides). Writable needs WRITE; read-only needs READ. No executable windows. Returns VA. |
| SYS_IRQ_RELAY | 18 | dev_idx, msix_entry, notif_slot, badge | Arm device-interrupt→notification bridge; returns vector. |
| SYS_CAP_PHYS | 19 | slot | Phys address a memory cap names (READ). Zero-copy DMA seam. |
| SYS_CAP_DESTROY | 20 | slot | Discard a cap reference; DESTROY required; owned Untyped frees its frame. |
| SYS_CAP_COPY | 21 | src, dst, rights | Attenuated delegation inside the caller's space (rights ⊆ RIGHTS_ALL, nonzero); dst empty; source needs COPY. Untyped copies are LENT. |
| SYS_DEV_INFO | 22 | dev_idx, buf | Resolved virtio record (12 words) for a granted device; held Mmio cap gate. |
| SYS_CONSOLE_PUSH | 23 | slot, buf, len | Feed bytes into the console line discipline (CapObj::ConsoleInput + WRITE). len==0 is a capability probe returning 0. |
| SYS_CONSOLE_ATTACH | 24 | cap slot, notif slot, badge | Attach as console output channel (ConsoleOutput + READ; notification + WRITE; nonzero badge). STATUS_BUSY if already attached. |
| SYS_CONSOLE_PULL | 25 | cap slot, buf, len | Drain mirrored console output (owner only); count returned. |
| SYS_CLOCK_NOW | 26 | — | Monotonic microseconds since boot (positive payload; saturates at i64::MAX). |
| SYS_TIMER_ARM | 27 | notif slot, badge, delay_us | Deliver `badge` on the notification after the delay; returns timer id. Notifications gated; WRITE on the notification. |
| SYS_TIMER_CANCEL | 28 | timer id | Disarm; cancelling an already-fired or stale id is STATUS_BAD_ARG. |
| SYS_CAP_DESCRIBE | 29 | slot, out[3] | Query a held cap: `out` receives `[kind, object_id, rights]`. Object ids are descriptive, never grant handles. Kind map: 1 Image, 2 Endpoint, 3 Notification, 4 Process, 5 ImageRegistrar, 6 BootImage, 7 SharedRegion, 8 MemoryPool, 9 SharedDma, 10 ProofToken, 11 Untyped(lent), 12 BadgedEndpoint (badge hidden — server's business), 13 Rtc, 14 VmRegion, 15 NotificationFactory, 16 SyncDomain, 17 SyncDomainFactory. Refuses dead/unknown kinds. |
| SYS_PROC_FINISH | 30 | slot, mode | Process cap + DESTROY; mode 0 reap exited only, mode 1 stop live then reap. Roots/supervised/self refused. |
| SYS_RESOURCE_SNAPSHOT | 31 | power slot, out[3] | `[free frames, live spawn records, occupied process slots]`; Power + WRITE. |
| SYS_IMAGE_REGISTER | 33 | registrar slot, addr, len, dst slot, 0, 0 | Register a dynamic image (user bytes copied, validated, ELF-checked); mint Image cap in dst. |
| SYS_IMAGE_REVOKE | 34 | registrar slot, id, 0, 0, 0, 0 | Revoke a dynamic image. |
| SYS_IMAGE_INFO | 55 | image slot, out[3] | `[entry, lowest PT_LOAD base, byte length]` for a held Image/READ. |
| SYS_DISPLAY_INFO | 35 | mmio slot, out[40] | Five scalars (width, height, pitch_pixels, format, bytes) for the holder of the exact GOP Mmio window. |
| SYS_SHARED_CREATE | 36 | pool slot, pages, out[3] | Allocate a zeroed generation-safe SharedRegion; mint first cap. `out` = `[slot, object_id, byte length]`. |
| SYS_SHARED_MAP | 37 | region slot, writable (0\|1) | Single NX registered user window over the whole region; writable needs WRITE. Any flag > 1 → STATUS_BAD_ARG. Returns VA (positive payload). |
| SYS_SHARED_PHYS | 38 | region slot, dma slot, out[24] | `[phys, pages, object_id]`; needs region READ and a held SharedDma/READ. |
| SYS_SHARED_INFO | 39 | region slot, out[2], 0, 0, 0, 0 | `[object_id, pages]` for a held SharedRegion (READ or WRITE). Descriptor-only; no phys or allocation authority. |
| SYS_SHARED_UNMAP | 40 | exact own VA, 0×5 | Remove only the registry-backed self-map; preflight before any mutation; exact VA/span required. |
| SYS_PROC_LIVE | 41 | process slot | Read-only liveness through a held Process/READ: 1 live, 0 exited/not tracked. |
| SYS_ENDPOINT_BIND | 46 | ep slot, notif slot, badge | Bind a notification to an endpoint's serve side; READ on endpoint, READ|WRITE on notification, one binding per endpoint. Queued calls OR `badge` into the notification. |
| SYS_ENDPOINT_UNBIND | 47 | ep slot | Unbind. READ on the endpoint. |
| SYS_ENDPOINT_MINT | 48 | ep slot, badge, rights | Mint a badged client endpoint cap (rights ⊆ WRITE|COPY, WRITE required); serve side (plain endpoint + READ). |
| SYS_RTC_READ | 50 | rtc slot, out[8] | Wall seconds since 1970 or STATUS_BUSY (unknown). Rtc cap + READ. |
| SYS_ENDPOINT_BADGE | 51 | server ep slot, badged cap slot | The badge of a landed badged cap — **serve-side only** (ADR-0074 amendment, Phase 11.5). Clients must not call it. |
| SYS_SHARED_PAGES | 52 | region slot | Page count of a held SharedRegion (READ or WRITE). |
| SYS_TLS_SET | 53 | base | Install the calling thread's FS.base (16-aligned, present+writable+NX user page); 0 clears. |
| SYS_CAP_OCCUPIED | 54 | slot | Read whether one caller-owned slot is occupied (0/1); no kind, identity, or rights disclosed. |
| SYS_VM_RESERVE | 56 | pages, out[3] | Process-local lazy VM reservation with guard pages; `out` = `[cap slot, base VA, capacity pages]`. |
| SYS_VM_COMMIT | 57 | region slot, offset pages, pages, protection | Commit pages in a reservation (VmRegion + WRITE). |
| SYS_VM_PROTECT | 58 | region slot, offset pages, pages, protection | Change protection (VmRegion + WRITE). |
| SYS_VM_RELEASE | 59 | region slot | Destroy a VmRegion cap (DESTROY; not in use as a stack). |
| SYS_VM_QUERY | 60 | region slot, out[6] | Exact-cap description and committed-page accounting (READ). |
| SYS_PROC_STATUS | 61 | process slot, out[2] | `[exited, status]` through a held Process/READ. |
| SYS_THREAD_YIELD | 62 | — | Voluntary reschedule; returns 0. |
| SYS_THREAD_CREATE | 64 | entry, argument, stack-cap slot, stack-low, stack-top, fs-base | Create a same-process ring-3 thread. Stack cap: READ|WRITE|DESTROY, no COPY. Returns thread id. |
| SYS_THREAD_JOIN | 65 | thread id, out status | Wait for a same-process user thread; collect stable exit status. |
| SYS_THREAD_DETACH | 66 | thread id | Detach for scheduler stack cleanup. |
| SYS_THREAD_COUNT | 67 | — | Live scheduler-thread count of the calling process. |
| SYS_NOTIFICATION_CREATE | 63 | factory slot, destination slot | Mint an owned notification (READ|WRITE|COPY|DESTROY) in an exact empty slot through a NotificationFactory + WRITE. |
| SYS_SYNC_DOMAIN_CREATE | 68 | factory slot, destination slot | Mint a SyncDomain through the trusted manager's factory (WRITE). |
| SYS_SYNC_KEY_CREATE | 69 | domain slot | Create a wait key; returns key token. WRITE on the domain. |
| SYS_SYNC_KEY_DESTROY | 70 | domain slot, key | Destroy a wait key. WRITE on the domain. |
| SYS_SYNC_SEQUENCE | 71 | domain slot, key | Read a wait key's sequence. READ on the domain. |
| SYS_SYNC_WAIT | 72 | domain slot, key, observed, timeout_us | Park against an observed value with a deadline; READ on the domain. |
| SYS_SYNC_WAKE | 73 | domain slot, key, mode (1\|2) | Wake one/all registered threads. WRITE on the domain. |
| SYS_SYNC_INFO | 74 | domain slot, out[4] | `[keys, keys_high, parked_waiters, waiters_high]`. READ on the domain. |
| SYS_RESOURCE_DETAIL | 75 | memory pool slot, out[10] | Aggregate occupancy `[frames, threads, endpoints, notifications, timers, VM regions, VM pages, sync domains, sync keys, parked waiters]`; MemoryPool/READ. |
| SYS_ENTROPY_SEED | 76 | kernel entropy seed slot, 32-byte ptr | Submit one exact 32-byte virtio-rng seed (KernelEntropySeed + WRITE); buffer wiped after copy. |

`CAP_NONE = 0xFFFFFFFFFFFFFFFF`. Rights bits: READ=1, WRITE=2, COPY=4,
DESTROY=8; RIGHTS_ALL = 15. `CAP_SLOTS = 128`.

## 2. Typed statuses (ADR-0017)

| Status | Value | Meaning |
|---|---|---|
| STATUS_OK | 0 | Plain OK. |
| STATUS_BAD_CALL | -1 | Invalid / unallocated syscall number. |
| STATUS_BAD_ARG | -2 | Invalid argument or insufficient rights. |
| STATUS_BAD_ADDRESS | -3 | User buffer not fully page-validated / not present. |
| STATUS_BUSY | -4 | Bounded object refused rather than blocked (full queue, second waiter, exhausted resource). Never retried by the client as a server status. |
| STATUS_SERVICE_GONE | -5 | The server process was destroyed while the request was queued or in hand; the request may or may not have been performed ("unknown", not "did not happen"). |
| STATUS_CALLER_GONE | -6 | |
| STATUS_QUOTA | -7 | Per-process bound refused. |
| STATUS_TIMEOUT | -8 | Bounded native synchronization wait deadline. |
| STATUS_NO_ENTROPY | -9 | PIE placement seed not ready. |
| STATUS_NO_SPACE | -10 | No collision-free PIE placement. |

## 3. Startup ABI v2 (ARST)

| Field | Value | Notes |
|---|---|---|
| ARST_BLOCK_BYTES | 4096 | |
| ARST_HEADER_BYTES | 128 | |
| ARST_ARGUMENT_MAX | 32 | |
| ARST_ENVIRONMENT_MAX | 32 | |
| ARST_CAPABILITY_MAX | 7 | Verified descriptors granted to the app. |
| ARST_STRING_BYTES_MAX | 3072 | |
| ARST_CAPABILITY_DESCRIPTOR_BYTES | 16 | `slot u16, role u8, kind u8, rights u32`. |
| ARST_STRING_DESCRIPTOR_BYTES | 8 | |
| ARST_VERSION | 2 | |
| ARST_NONE | 0xFFFF | |
| ARST_FLAG_MULTI_INSTANCE | 1 | |
| ARST_FLAG_BACKGROUND | 2 | |
| ARST_FLAG_HEADLESS | 4 | |
| ARST_FLAG_STANDARD_STREAMS | 8 | |
| ARST_FLAG_NATIVE_SYNC | 16 | |
| Role CURRENT_DIRECTORY | 1 | |
| Role STANDARD_INPUT | 2 | |
| Role STANDARD_OUTPUT | 3 | |
| Role STANDARD_ERROR | 4 | |
| Role OTHER | 5 | Graphical descriptors carry this role. |
| Role STANDARD_STREAM_SET | 6 | |
| Role STREAM_WAKE | 7 | |
| Role SYNC_DOMAIN | 8 | |

The verified record is presented to `main()` via `arena_startup()`; slot `i+1`
corresponds to descriptor `i`, but slot numbers play no role in
identification (B-GFX-1 / ADR-0114).

## 4. ADSK wire (userspace/desktop/src/wire.rs)

All frames: 64 bytes, `b[0..3] = "ADSK"`, `b[4] = version (1)`, `b[5] =
opcode`, `b[6..7]` reserved zero, `b[8..15]` = handle u64 LE. Decode is
strict (canonical form; reserved bytes checked; mutants refused).

| Opcode | Frame | Fields |
|---|---|---|
| 1 | Create | `b[24..26]` width u16 LE, `b[26..28]` height u16 LE (80..1024 × 60..768); handle 0 |
| 2 | Damage | `b[16]` = rect count n (0 = full); up to 5 rects `[x,y,w,h]` u16 at `24 + 8i + 2j` |
| 3 | Poll | handle |
| 4 | Title | handle, text [u8; 32] at b[32..] |
| 5 | Event (reply to Poll) | `b[28]` type: 1 Key (u16 key at b[24..26]), 2 Pointer (x i32 b[16..20], y i32 b[20..24], buttons b[29]), 3 Close, 4 Focus (b[29] 0/1), 5 Configure, 6 PopupPointer, 8 Chord, 9 Wheel, 10 Chosen |
| 6 | CancelClose | handle |
| 7 | Resize | handle, width, height |
| 8 | Resizable | handle, min width, min height |
| 9 | Popup | handle, kind, x, y, width, height |
| 10 | Dismiss | handle |
| 11 | DestroyWindow | handle |

Reply words: `[w0, w1, cap-slot-or-CAP_NONE]`; `w1` carries the window
handle for Create. The inline reply buffer is overwritten by the server's
reply frame (an Event frame for Poll when an event is queued; otherwise the
buffer is unchanged, i.e. still the Poll frame). An unexpected landed
capability (`out[2] != CAP_NONE`) is destroyed and the call refused.

## 5. Desktop service authentication (source: userspace/desktop/src/bin/desktop.rs)

- Requests arrive on `SERVER` via `SYS_IPC_RECV_BADGED`: `request[3]` is the
  kernel-delivered badge; `request[2]` is the landed cap slot.
- `select_live_badge` matches the badge to exactly one live session whose
  process is live; unknown, stale, duplicate, or dead-owner badges fail
  closed (no session or surface state change).
- `description = SYS_CAP_DESCRIBE(landed)` must be `[7, session.id,
  READ|WRITE|COPY]` for graphical frames (Create/Damage/Poll/…). The
  session's SharedRegion object id authenticates the surface.
- `SYS_ENDPOINT_BADGE` is serve-side only; clients never query badges.

## 6. V1 graphical client contract (experiments/c-runtime/app/c_gfx_app.c)

- Identification by discovery over the verified ARST record: exactly one
  BadgedEndpoint (rights exactly WRITE|COPY|DESTROY) and exactly one
  SharedRegion (rights including READ|WRITE|COPY); ambiguity → typed
  refusal, nothing guessed.
- Surface: 640×480 u32 pixels at offset 4096; region must provide ≥ 301
  pages (`SYS_SHARED_PAGES`); map with `SYS_SHARED_MAP(slot, 1)`.
- Renders a deterministic integer chequerboard pattern (16×16 cells,
  colors from `(cell_x + cell_y + frame)`), publishes with `Damage FULL`,
  polls events, flashes on input acknowledgment, animates 300 bounded
  frames, destroys the window, unmaps, and exits cleanly with a distinct
  positive exit code per failure class (1..11).
- Integer-only: build flags `-mgeneral-regs-only -msoft-float -mno-sse
  -mno-sse2 -mno-mmx -mno-80387`; static audit verifies zero x87/MMX/SSE/AVX
  instructions, ET_EXEC, no W+X.
