# ADR-0024: virtio-net in ring 3 — `netd`, the link-only driver server, and the ARP link proof

Status: accepted (M6.1)

## Context

Phase 6 opens with the VirtIO family completion promised in §8 of the
architecture: drivers are **userspace servers**, and the substrate
(ADR-0021) plus its first resident driver (ADR-0022, `storaged`) have
survived a full milestone of daily use — including the M5.4 crash gate,
which kills QEMU mid-DMA and reboots clean. The next driver is
virtio-net, the gateway to Phase 7 (Ethernet → ARP → IPv4 → UDP → TCP).

The problem this ADR solves is scoping: **what is the smallest useful
increment that proves a ring-3 NIC driver genuinely exchanges frames
with the outside world**, without dragging protocol-stack work
(Phase 7) into a driver milestone — and without faking anything. A
driver that only initializes registers proves nothing; the proof has
to be a real packet leaving the guest and a real reply coming back.

## Approaches considered

1. **Kernel-embedded NIC driver.** Fastest path to bytes on the wire,
   directly rejected by §8: no driver may be loaded into the kernel.
   The isolation property (a driver crash loses its own state, not the
   machine) is the product.
2. **Synthetic fixture: two NICs back-to-back, or a socket netdev with
   a host-side echo script.** Real traffic, but adds a second device,
   host-side tooling, and a fixture whose failure modes (host script
   bugs) pollute driver diagnosis.
3. **DHCP probe against QEMU's slirp.** A genuine round trip, but the
   request is UDP/IP — implementing even throwaway IP framing inside a
   driver milestone blurs the Phase 6/7 boundary and tests more stack
   than driver.
4. **ARP probe against QEMU's slirp (chosen).** Slirp (`-netdev user`)
   answers ARP for its gateway 10.0.2.2. One 42-byte Ethernet frame
   out, one 42-byte frame back: exercises every part of the driver —
   feature negotiation, config-space MAC read, TX descriptor chains
   over a *caller-lent* page, RX buffer posting, two MSI-X vectors,
   interrupt-driven wake, frame integrity — with zero protocol work
   beyond byte offsets the test client fills in itself.
5. **Extract a shared `userspace/virtio` library now** (storaged +
   netd + future drivers). Tempting, but the shared surface is only
   proven across ONE driver so far, and net's dual-queue async-RX
   shape differs from block's single synchronous queue. Abstracting at
   N=2 risks freezing the wrong seams. Deferred by the **third-driver
   rule**: when virtio-rng (6.2) lands, three instances will show
   which code is genuinely common; extract then, mechanically, under
   the full regression suite.

## Decision

**`netd` — spawn-registry image 6 — is the virtio-net driver server,
link-layer only: raw Ethernet frames in, raw Ethernet frames out.**
Protocols are the callers' business until Phase 7 gives them a home.

- **Spawn shape mirrors storaged exactly**: kernel-granted `Mmio` cap
  over the virtio structure BAR (R|W), the serve side of an endpoint,
  and one notification. No new syscalls; no kernel ABI change —
  `SYS_DEV_INFO`'s frozen 12 words (ADR-0022) already carry the
  device-config cap location in word `[6]`, which becomes
  load-bearing here: the MAC address is read from the device config
  region through the self-mapped window, never from PCI config space.
- **Order-independent discovery.** Drivers no longer hardcode
  `dev_idx = 0`. The Mmio-cap gate inside `SYS_DEV_INFO` accepts only
  the device the caller was granted, so a driver probes indices
  upward and adopts the first index the kernel answers — then asserts
  its virtio type from the returned `device_id` (modern
  `0x1040 + type`; net = 1, block = 2). storaged adopts the same
  loop (asserting block), so fixture device order on the QEMU command
  line stops being load-bearing.
- **Queues and interrupts.** q0 = receiveq, q1 = transmitq (no
  control queue — `VIRTIO_NET_F_CTRL_VQ` not negotiated). Two MSI-X
  entries → two `SYS_IRQ_RELAY` vectors → two distinct badge BITS
  into the one notification (notifications merge pending badges by
  OR, so bit-disjoint badges decode unambiguously even when a TX
  completion and an RX arrival race), and the kernel counts one
  delivery per vector. Features negotiated: `VERSION_1` + `MAC`,
  nothing else (no csum/GSO offload, no mergeable buffers) —
  `virtio_net_hdr` is the 12-byte VERSION_1 form, zeroed on TX and
  stripped on RX; callers see pure Ethernet frames.
- **Ring memory fits the cap budget.** `CAP_SLOTS = 16` leaves a
  driver eight frame slots (8..15) after its three grants, so each
  queue's three ring areas are PACKED into ONE frame at the
  modern-virtio alignments (desc 16 B, avail 2 B, used 4 B — virtio
  1.0 §4.1.4.3 dropped legacy's page alignment) with the queue size
  capped at 64: q0 rings 1 frame + three RX buffers 3 + q1 rings
  sharing their frame with the static TX header at +3072 (the used
  area ends at 1678) = five frames, slots 8..12. The fixture turned
  out TRANSITIONAL (QEMU's `virtio-net-pci` default: device_id
  0x1000 driving the modern structures) — drivers assert
  transitional-or-modern IDs per type, never one form only.
- **TX is zero-copy like storaged's READ**: the 12-byte virtio header
  lives in netd's own page and chains (header → caller's lent frame)
  in the descriptor table; the device DMAs the caller's page
  directly. The lent-cap discipline of ADR-0022 applies verbatim
  (phys via `SYS_CAP_PHYS`, reference discarded after completion).
- **RX v1 posts three netd-owned receive frames** and keeps them
  posted; on the RX badge, used entries are harvested into a
  single-frame hold slot — the FIRST undelivered frame is held,
  later arrivals are dropped with an honest log and an immediate
  re-post, so the ring never shrinks. **`NET_RECV` delivers through
  the REPLY's inline 64-byte message** (the fstest-LS pattern), not
  into a caller buffer: a lent cap CANNOT be mapped (the M5.2
  structural rule that keeps lending safe), so netd could never copy
  into the caller's frame — and the caller's frame cannot be posted
  to the device retroactively either. Frames longer than `MSG_BYTES`
  are dropped with a typed refusal and a log line. Both are
  documented v1 simplifications: zero-copy full-frame RX needs a
  buffer-handoff design (register buffers in advance, or transfer
  frame ownership) that Phase 7's stack will motivate properly; the
  ring itself absorbs bursts of three.
- **API v1** on netd's endpoint (op in `msg[1]`, statuses `NET_S_*`
  in `userspace/abi.rs`): `NET_SEND` (1: `msg[0]` = frame length
  14..=1514, send cap = the caller's LENT frame, bytes at offset 0),
  `NET_RECV` (2: no cap; blocks until a frame is held; reply `w1` =
  frame length, frame bytes in the reply's inline message),
  `NET_MAC` (3: no cap; reply `w1` = the six config-space MAC bytes
  packed little-endian), `NET_SHUTDOWN` (0: poison — reply first
  with the lifetime completion count, then exit by its own hand,
  exactly storaged's discipline). Unknown ops are refused with a
  typed status; every refusal path destroys the landed cap (a cap is
  never leaked, never silently accepted on the wrong op).
- **The milestone proof** (`nettest`, image 7, spawned by the new m6
  suite): fetch the MAC via `NET_MAC`, hand-build the 42-byte ARP
  request (`who-has 10.0.2.2 tell 10.0.2.15`), `NET_SEND` it,
  `NET_RECV` slirp's reply, and assert ethertype 0x0806, opcode
  reply, sender IP 10.0.2.2, and target MAC == our MAC — then poison
  netd and exit 42. Per-step typed exit codes (43…) mirror blktest's
  failure taxonomy so the kernel-side verdict maps codes to causes.
- **Graceful absence.** If bus 0 has no virtio-net function, the
  kernel logs that the network service is offline, skips the
  production spawn, and the m6 suite's test reports a loud, honest
  **SKIP** — not PASS, not FAIL. Rationale: the scratch disk is
  load-bearing for v0.5+'s identity (persistence) and stays
  hard-required, but the net device is NEW in v0.6.0; pre-v0.6.0
  QEMU invocations (users' shell history, older RUNNING.md copies)
  must keep booting green. The compatibility window is revisited in
  Phase 7, when the documented command has carried the device for a
  full release cycle.
- **Harness**: every boot path grows
  `-netdev user,id=net0 -device virtio-net-pci,netdev=net0`
  (`arena_env.net_args()`, spliced into both mtest paths, the release
  verification boot, and RUNNING.md's command).

## Consequences

- Image registry slots 6 and 7 are consumed; `MAX_IMAGES = 8` is now
  full. The next driver milestone (6.2, rngd) must raise the cap —
  one constant plus the registry match arm, noted here so it is a
  decision, not a surprise.
- netd deliberately duplicates storaged's virtio handshake/queue
  core (~200 lines). This is recorded debt with a named trigger: the
  third-driver rule (6.2) extracts the shared library under the full
  suite.
- Slirp is a user-mode toy peer: no multicast, no promiscuous
  honesty, one built-in gateway, and it pads the 42-byte ARP reply to
  the 60-byte Ethernet minimum (the probe asserts fields at their
  wire offsets and a 42..64 length range, never one exact count).
  Sufficient for link proofs and Phase 7's early protocols
  (ARP/DHCP/UDP/DNS all work against slirp); TCP-level and
  multi-host tests will add tap-based fixtures when Phase 7 reaches
  them.
- The RX hold slot drops bursts beyond one undelivered frame (the
  ring absorbs three; the fourth concurrent arrival is lost), and
  inline delivery caps received frames at 64 bytes in v1. Acceptable
  for a link-probe milestone; Phase 7's stack client will drive the
  real buffering design.
- Boot output grows two lines (production netd spawn; suite verdict),
  and the m6 suite joins the release gate automatically through the
  `tools/test_m*.py` glob once `test_m6.py` exists.
