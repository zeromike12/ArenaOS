# ADR-0022: The userspace block service — storaged, the zero-copy block protocol, and owned vs. lent Untyped caps

- Status: accepted
- Date: 2026-09-26
- Milestone: M5.2 (ROADMAP phase 5 — Storage)
- Supersedes: — (extends ADR-0015 capability spaces, ADR-0018 IPC, ADR-0021 driver substrate)

## Problem

M5.1 built the substrate: owned `Untyped` frames, kernel-minted `Mmio`
caps, the IRQ relay's vector range, and the kernel's PCI/VirtIO scan as
POLICY. Turning that into a real block service forces five decisions:

1. **Who runs the VirtIO protocol?** ARCHITECTURE §10 says filesystems
   and device servers are userspace processes; ring 0 keeps no device
   protocol. But a virtio-pci driver needs things only the kernel has:
   the resolved capability records from the config-space walk, MSI-X
   table writes, and the MSI-X enable bit in config space. Where
   exactly is the line?
2. **How does data travel?** The classic easy path is a kernel bounce
   buffer: copy caller bytes in, let the driver copy again, DMA, copy
   back. That is two extra memory trips per request and a kernel
   mediator the capability model was built to avoid. Zero-copy means
   the device DMAs the CALLER's own frame — but then the driver must
   learn that frame's physical address, and a phys number in a message
   word is a bus-master hole: any process could aim the disk at any
   frame in the machine, including kernel text.
3. **Who owns a frame that crossed an IPC boundary?** IPC v1 transfers
   a cap with every message. If the landed cap is "the same" `Untyped`
   cap, the receiver can free the sender's frame out from under it (a
   use-after-free primitive) or map it into its own space (teardown
   then frees a frame the sender's address space still references).
   Ownership must be structurally singular — but the receiver still
   needs the frame's phys for the descriptor.
4. **How does a ring-3 driver sleep on an interrupt?** No ring-3 code
   ever runs in interrupt context here (ADR-0021). The 5.1 relay
   proved stub→notify→wake with a self-IPI; a real device MSI-X has to
   walk the same chain, and the arming itself (table entry, enable
   bit) is privileged.
5. **Lifecycle.** The service is a resident process spawned at boot.
   It also must be killable *by its own hand* for testing: destroying
   a process whose threads are parked in `recv`/`wait` is a
   catastrophe in this kernel (no blocked-thread eviction exists), so
   shutdown has to be a protocol citizen.

## Decision

### 1. `storaged`: the whole VirtIO protocol in ring 3

`userspace/storaged` is the third real userspace crate (registry image
2). The kernel spawns it at boot — after the m5 suite, before the
shell — with three kernel-literal grants:

- slot 0: `Mmio {bar4 base, 4 pages}` READ|WRITE — the device window
  (the virtio structures all live in one BAR on the reference
  fixture),
- slot 1: `Endpoint` READ — the serve side of the block service,
- slot 2: `Notification` READ|WRITE — the interrupt relay's target.

Everything else the driver does itself: `SYS_DEV_INFO` discovery, the
virtio 1.0 handshake (reset → ACK → DRIVER → VERSION_1 → FEATURES_OK
→ queue setup → DRIVER_OK) through volatile accesses to its mapped
window, one split virtqueue built from three frames it ALLOCATES
(`SYS_ALLOC_FRAME`) and self-maps (caps consumed — teardown stays
exact), a fourth request page (16-byte `virtio_blk_outhdr` + the
device-writable status byte behind a `0xFF` sentinel), and the
doorbell write at `notify_off + queue_notify_off × multiplier`.

**`SYS_DEV_INFO` (call 22)** is the ring-3 view of the kernel's scan:
12 u64 words (pci_index, structure-BAR base, the four
offset|length pairs, the notify multiplier, MSI-X present|size,
device_id|transitional). It is gated on the caller holding an `Mmio`
cap whose phys is the device's structure-BAR base — caps are the only
authority, and the gate keeps the surface honest. Config space itself
is never exposed. v1 requires all four virtio structures on ONE BAR
(a typed refusal otherwise); multi-BAR devices get per-BAR caps later.
The driver drives virtio-device index 0 (v1 has exactly one virtio
device; the index space is the kernel's virtio table, NOT the PCI
function index — a bring-up bug proved the distinction).

### 2. Owned vs. lent: `Untyped { phys, owned }`

The single refinement that makes zero-copy safe:

- `SYS_ALLOC_FRAME` mints `owned: true` — the one owning cap.
- `cap::copy` and the IPC cap-landing path FORCE `owned: false`.
  Ownership is never duplicated *structurally* — no code path exists
  that could produce a second owner. `cap::move_cap` transfers the
  object as-is (a move keeps ownership; it empties the source in the
  same IF=0 window).
- `cap::destroy` (and its ring-3 twin **`SYS_CAP_DESTROY`, call 20**)
  frees the frame only when `owned` — a lent reference just goes away.
- `cap::map_memory` / `SYS_MAP_MEMORY` refuse lent `Untyped` caps:
  mapping consumes the cap and hands the frame to an address space
  whose teardown would free someone else's frame.
- **`SYS_CAP_PHYS` (call 19)**, READ-gated, answers the phys of any
  memory-kind cap (`Untyped` owned or lent, `Memory`, `Mmio`). This is
  the zero-copy seam: a driver learns a caller's buffer address ONLY
  through a cap the caller handed over.
- **`SYS_CAP_COPY` (call 21)** exposes attenuation-only copy to ring 3
  (destination must be empty; amplification refused loudly). It
  enables the *lend-and-keep* pattern every zero-copy client uses:
  allocate (owned cap), copy it, self-map the ORIGINAL (the map
  consumes it — ownership moves to the address space, the window
  stays valid for the process's life), send the COPY with the call.

### 3. The zero-copy block protocol

IPC v1 is unchanged — two words plus one cap each way:

- request: `w0` = sector (512-byte units), `w1` = op (`0` read, `1`
  write, `2` poison-shutdown), send-cap = the caller's LENT buffer cap
  (`CAP_NONE` only for poison),
- reply: `w0` = the virtio status byte the DEVICE wrote (the `0xFF`
  sentinel proves it), `w1` = the used-ring byte count.

The driver never maps the caller's buffer: it reads the phys from the
landed cap (`SYS_CAP_PHYS`), points descriptor 1 at it (device-writable
iff read), and discards the cap (`SYS_CAP_DESTROY` — frees nothing,
lent) after the completion. The caller blocks in `ipc::call` for the
whole request, which is the frame's lifetime guarantee. One sector per
request in v1; one request in flight (synchronous IPC).

Poison-shutdown (`op 2`, sector `u64::MAX`) is the safe-death path:
the driver replies FIRST (the caller blocks), then exits through
`SYS_THREAD_EXIT` by its own hand. A service process is never
destroyed with parked threads.

### 4. `SYS_IRQ_RELAY` (call 18): the kernel arms, the driver waits

`SYS_IRQ_RELAY(dev_idx, msix_entry, notif_slot, badge) → vector`:

1. gate: WRITE on the caller's notification cap, nonzero badge, and
   the `SYS_DEV_INFO` device gate (the caller holds the device's
   `Mmio` cap);
2. allocate the first free relay vector 48..63;
3. program the MSI-X table entry — addr `0xFEE0_0000 | apic_id<<12`,
   data = vector, unmasked — through the **kernel PCI window**: a
   16-page supervisor window at `0xFFFF_FF00_0000_0000` whose PML4
   entry (index 510) `build_kernel_view` PRE-WIRES with its full
   PDPT/PD/PT hierarchy. Two hard-won facts: `mmio_alias_va` cannot
   serve sub-2 GiB BARs (the fixture's MSI-X table at `0x8108_5000`
   aliases onto the RAM direct-map page at `0x0108_5000`), and a
   window root created lazily would exist only under the kernel CR3 —
   process clones share kernel-half entries only where PRESENT at
   clone time (bring-up #PF, cr2 = window base). Pre-wiring also makes
   arming allocation-free, keeping the suites' frame accounting exact;
4. set MSI-X Enable (clear Function Mask) in config space, read-back
   verified (`pci::msix_enable` — the capability's config offset is
   recorded by the 5.1 scan as `MsixInfo.cap_ptr`);
5. register the relay OWNED by the calling pid
   (`relay::register_owned`), and return the vector.

The driver then writes `queue_msix_vector` itself (mechanism, ring 3)
and sleeps on `SYS_WAIT`. Ownership means `proc::destroy` sweeps the
dead driver's vectors (`relay::release_by_owner`): no dead process can
leave a live vector notifying a dead notification. Deliveries land
while ring-3 threads run IF=1 — no kernel-side interrupt window is
needed anywhere in the service path.

### 5. Lifecycle and the proof

The m5 suite's fifth test, `block_service`, spawns a TEST instance of
the same image (kernel-internal `spawn_init`, grants as above, plus
exit-badge notifications) and `blktest` (registry image 3, one grant:
the endpoint's call side). `blktest` runs the acceptance cycle: fill a
deterministic 512-byte pattern → WRITE sector 0 through the service →
CLEAR the buffer (the read-back must come from the disk, not
leftovers) → READ sector 0 → verify byte-for-byte → poison → exit 42.
The kernel-side test then proves the machine facts: both exit badges
exact and unmerged, both exit codes 42, exactly TWO relay-vector
deliveries (one per completed request — interrupt-delivered, never
polled), the registration swept by `proc::destroy`, and
frame-exact teardown of both processes. Only then does `entry.rs`
spawn the PRODUCTION instance, which re-resets the device, re-runs the
handshake, re-arms (getting vector 48 back — the sweep freed it), and
parks in `recv` as a resident service (`ps` shows it next to the
shell).

## Approaches considered

- **In-kernel virtio-blk driver.** Rejected: contradicts ARCHITECTURE
  §10 and the 5.1 policy/mechanism split; a kernel driver puts a
  storage protocol's whole bug surface in ring 0.
- **Kernel bounce buffers (copy through the call).** Rejected: two
  extra memory trips per request, kernel mediation the capability
  model exists to avoid, and it would prove nothing about DMA — the
  hard part of storage.
- **Phys address in a message word (no cap).** Rejected: the
  bus-master hole. Any process could point the disk (a DMA-capable
  device the kernel already authorized with BUS MASTER) at any frame —
  kernel text included. `SYS_CAP_PHYS` behind a landed cap makes the
  caller's consent explicit and revocable.
- **IOAPIC/INTx pin interrupts.** Rejected: virtio 1.0's modern path
  is MSI-X, the relay range was built for it, and level-triggered pin
  machinery (mask/unmask around handlers) is work with no consumer.
- **Driver polls the used ring.** Rejected: burns CPU, contradicts the
  milestone's exit criterion (interrupt-delivered completions,
  counted), and polling from a blocked-in-`recv` server is impossible
  anyway without a second thread.
- **Hand-assembled emitter payload for the client** (the m3/m4/m5.1
  test style). Rejected for this test: the client is protocol-heavy
  (copy/map/call/verify/poison); a real Rust image is the honest
  artifact and doubles as the second consumer of the new ABI surface.
  `blktest` shares `src/abi.rs` with the driver — one protocol, two
  programs.
- **Reply-cap ping-pong instead of SYS_CAP_COPY** (send the owned cap,
  driver returns it in the reply). Rejected: the sender could not keep
  a live mapping while the cap is away (mapping requires ownership),
  so it could not fill the buffer before the WRITE or inspect it after
  the READ without remapping acrobatics; lend-and-keep is one syscall
  and no races.

## Consequences

**Gained:**

- The first resident userspace SERVICE: a process, spawned at boot,
  that other processes call — the shape 5.3's `fsd` will reuse
  (fsd becomes storaged's first production client).
- Zero-copy DMA proven end-to-end on real virtio hardware emulation:
  caller frame → landed lent cap → descriptor → device DMA → verified
  read-back, with exactly one frame owner at every instant.
- ABI v1 slots 18–22 (additive, frozen): IRQ relay, cap phys/destroy/
  copy, dev info. MSI-X arming is generic — the 5.5 network device
  reuses `SYS_IRQ_RELAY` verbatim (different entry, same window).
- Ownership refinement is structural: the type system + two forced
  `owned=false` sites (copy, IPC landing) make double-ownership
  unrepresentable, and the `proc::destroy` relay sweep makes
  dead-driver vectors unrepresentable.
- The kernel PCI window is general infrastructure for any future
  kernel-side device-register write (idempotent per page, 16 slots,
  shared under every CR3 by construction).

**Costs and v1 limitations (each a deliberate scope fence):**

- One virtqueue, one in-flight request, fixed 512-byte requests, no
  feature negotiation beyond VERSION_1, no flush op, no capacity
  checks (an out-of-range sector comes back as a device IOERR — the
  honest answer). Queue depth is clamped to 256 so each ring fits one
  frame.
- The driver drives virtio-device index 0 by convention; there is no
  device-selection story until a second virtio device exists (5.5).
- A caller blocked in `ipc::call` on an endpoint whose server DIED is
  stranded (no endpoint-death wake exists in IPC v1). The suite never
  hits it (poison-shutdown ordering), and 5.3 must decide whether
  endpoint liveness belongs in `recv`/`call` error paths.
- `blktest` (image 3) exists purely as the suite's client — a test
  image shipping inside the kernel binary, like image 0 before it.
- The production storaged's endpoint id lives only in the boot log
  until 5.3 wires fsd to it (no name service exists yet).
- `storaged` restart/re-spawn after a crash is untested territory; v1
  treats the resident instance as part of the boot contract (a
  `fail()` exit halts the boot in the suite window and leaves a dead
  service afterwards — supervisor policy is a later milestone).
