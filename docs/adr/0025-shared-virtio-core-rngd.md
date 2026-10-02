# ADR-0025: the shared virtio core (`userspace/virtio.rs`) and `rngd` — zero-copy entropy service

Status: accepted (M6.2)

## Context

ADR-0024 recorded netd's duplication of storaged's virtio
handshake/queue core (~200 lines) as **debt with a named trigger**:
the third-driver rule. Abstracting at N=2 risked freezing the wrong
seams — block's single synchronous queue and net's dual-queue
async-RX shape disagreed about what "the virtio core" even is. M6.2
lands virtio-rng, the third driver, and with it the evidence: three
instances now show exactly which code is genuinely common.

Two problems, one milestone:

1. **Execute the extraction** — mechanically, under the full
   regression suite, with zero behavior change to the two shipping
   drivers (every harness-asserted log line and exit-code contract
   survives byte-identical).
2. **Design the smallest useful entropy service** — and its honest
   proof. Randomness cannot be asserted by expected values (a fixed
   expectation is fixed entropy); so what CAN a suite prove about an
   RNG driver without faking anything?

## Approaches considered

### The shared core's shape

1. **Keep the duplication** (three copies with rngd). Rejected — the
   rule exists precisely because copy-drift between drivers is a
   class of bug the suite cannot see (each copy passes its own test).
2. **A real cargo library crate** (`userspace/virtio` as a path
   dependency). Workable — each image is a standalone `no_std` static
   ELF — but rejected for v1: `abi.rs` already established the
   single-file `#[path]` inclusion convention for cross-image code
   ("one wire contract, all programs"), the core has no dependencies
   beyond `abi`, and a crate adds workspace/profile plumbing for zero
   runtime difference. Revisit if the core grows modules.
3. **Fold the core into `abi.rs`.** Rejected — different stability
   contracts: `abi.rs` mirrors the frozen kernel syscall/IPC surface;
   the virtio core is device-protocol code that evolves with the
   driver family (new layouts, new queue shapes).
4. **A single `userspace/virtio.rs`, included by `#[path]` like
   `abi.rs` (chosen).** The includer declares `mod abi` (every image
   already does); the core resolves its syscall/console surface
   through `crate::abi`. Compiled per image — each driver carries
   only what it uses, dead subsets silenced the same way `abi.rs`
   silences its overlapping subsets.

### The error seam

5. **Shared failure logging + exits inside the core.** Rejected —
   exit-code contracts are per-suite (m5 maps storaged's 60..68, m6
   maps netd's 70..74 and rngd's 80..88); the core cannot own them.
6. **Typed stage errors (chosen)**: `VErr::{DevInfo,Map,Handshake,
   Queue,Relay}(reason)` — the five setup stages every driver's
   exit-code block already shares, in order. The driver maps stage →
   ITS code (`vfail`, six lines) and fails with the carried reason.
   The suites' diagnostic contracts are unchanged.

### The rng proof

7. **Assert expected output bytes.** Impossible without fixing the
   entropy — a fake.
8. **Host-side comparison** (guest bytes vs. the QEMU backend's
   source). Rejected — couples the suite to host internals
   (`rng-builtin`'s source is platform-specific and undocumented at
   byte level).
9. **In-guest variance + machine-side witnessing (chosen).** rngtest
   takes TWO 4 KiB draws into separate caller frames and asserts:
   neither draw is all-zero, neither is a single repeated byte, the
   two draws differ — checks any genuine entropy source passes with
   probability 1 and any stuck/zeroed/looping device fails. The
   KERNEL witnesses the mechanics: exactly one relay delivery per
   GET (interrupt-driven, never polled), exact exit badges, exact
   exit codes, frame-exact teardown. This proves transport and fill;
   entropy QUALITY is the host backend's job — stated honestly, never
   claimed by the suite.

### The fixture

10. **Explicit `-object rng-random,filename=/dev/urandom,id=rng0`.**
    POSIX-host-only (`filename` does not exist on Windows QEMU
    builds) — rejected for the default path.
11. **Bare `-device virtio-rng-pci` (chosen).** QEMU's
    `virtio_rng_device_realize` auto-creates a `rng-builtin` default
    backend (the platform CSPRNG) when none is given — verified in
    QEMU v11.0.2 `hw/virtio/virtio-rng.c`, and the default since
    QEMU 4.1 (2019). Portable across host OSes, no host files, no
    privileges. Museum-piece QEMUs without the default backend use
    the explicit form (documented in RUNNING.md); hosts with no rng
    device at all get the honest SKIP (boots stay green, ADR-0024's
    optionality discipline).

## Decision

**`userspace/virtio.rs`** — the shared virtio 1.0 core, extracted
mechanically from storaged and netd under the full suite (11/11
scripts green before rngd existed):

- `discover(prefix, kind, id_modern, id_transitional, min_msix)` —
  the order-independent SYS_DEV_INFO probe loop (ADR-0024), the
  12-word parse, the device-class assert, the MSI-X table check.
- `map_window(prefix, kind, info, slot)` — the writable self-map of
  the granted Mmio cap + the unified geometry log (identity first,
  geometry second — ADR-0023's HELP lesson).
- `handshake(prefix, w, f0_req, f1_req, min_queues)` — §3.1: reset
  (spin-checked), ACK|DRIVER, feature words read, REQUIRED bits
  checked (VERSION_1 special-cased with its own reason), acceptance
  of exactly the required bits, FEATURES_OK stickiness, NUM_QUEUES
  floor.
- `alloc_frames(prefix, slot_base, n, phys, va)` — the alloc/map/zero
  budget loop (rings must START zeroed).
- `ring_offsets(qsz)` + `Queue` (`publish` with the fence discipline
  slot→fence→idx→fence→doorbell, `used_idx`, `used_entry`) +
  `queue_setup(prefix, w, info, qidx, qmax_cap, RingMem, msix_entry,
  slot_notif, badge)` → `(Queue, relay_vector)`. `RingMem::Packed`
  (all three areas in one frame at the CLAMPED size's offsets —
  netd/rngd under CAP_SLOTS pressure) or `RingMem::Split` (a frame
  per area — storaged's 256-entry rings). DRIVER_OK is the driver's
  to write (`driver_ok`), after its own setup (buffer posting,
  static headers) completes.
- `w64` (lo-then-hi, §4.1.4.3) and `desc_write` (the 16-byte split
  descriptor).

Driver-side by design: grant layouts, exit-code contracts, serve
loops, class specifics (netd's MAC/queues/RX hold/TX chaining,
storaged's request header/status sentinel, rngd's fill path), and
every harness-asserted log line — the assertion inventory
(`virtio-net ready` ×2, `netd: starting` absent in no-net boots, the
`storaged: WRITE` crash-gate markers) is untouched; only failure-path
reason strings unified (no harness reads them; the suites map CODES).

**`rngd`** (spawn-registry image 8, `userspace/rngd`): the virtio-rng
driver (transitional ID 0x1004 / modern 0x1044), one virtqueue — the
device advertises only 8 entries, which the shared clamp honors
(cap 64, device wins). Protocol (abi.rs, the wire contract):

- `RNG_OP_SHUTDOWN = 0` — the poison: reply `w1 = completions`, exit
  42 (`EXIT_OK`).
- `RNG_OP_GET = 1` — `w0 = requested bytes (1..=RNG_DRAW_MAX=4096)`,
  send cap = the caller's LENT frame. rngd resolves the frame's phys
  (`SYS_CAP_PHYS` — never a message word), points ONE
  device-writable descriptor at it, publishes, and blocks on the
  relay badge: **the device DMAs entropy directly into the caller's
  page — zero-copy fill** (storaged's READ direction, as a service).
  Reply `w0 = RNG_S_OK`, `w1 = bytes the device wrote`. One request
  in flight; MSI-X entry 0 → relay, badge 0x7201; the completion is
  always an interrupt, never a poll.
- Typed refusals: `RNG_S_BAD_OP` (-1), `RNG_S_BAD_LEN` (-2),
  `RNG_S_NO_BUF` (-3) — the landed cap is destroyed on every refusal
  path (never leaked).
- Exit codes 80..88 — the same nine-stage block as storaged/netd,
  mapped in m6.rs.

**`rngtest`** (image 9, same crate): two owned frames, two 4 KiB
GETs, the three variance checks (non-zero, non-constant, draws
differ), the poison-reply completions assertion (== 2), exit 42;
failure codes 52..57 mapped in m6.rs. Badges: RNGTEST 0x727C, RNGD
0x72D0.

**m6 suite test 2 (`rng_service`)**: the net_service shape — spawn
both, wall-clock-bounded interruptible drain, exact badges, exact
exit codes (both 42), the relay delivery count EXACTLY 2 on the
suite instance's vector (one MSI per GET; the net test's teardown
released 48/49 first, so rng deterministically lands on 48),
`proc::destroy` sweeps the dead driver's relay, teardown frame-exact
(rngd's ONE ring frame + rngtest's TWO draw frames). No device →
honest SKIP; `RESULT` reports `SKIP (n/2 passed, m skipped)` in any
fixture subset — every combination boots green.

**Production rngd** (entry.rs, after netd): resident, parked on its
endpoint for Phase 7+ consumers; absent device → the honest offline
note, never a fake init. In a full-fixture boot the relay pool hands
it vector 51 (48 storaged, 49/50 netd — first-free allocation).

**Registry caps raised (the ADR-0024 consequence, executed):**
`MAX_IMAGES` 8 → 12 (rngd=8, rngtest=9; headroom for Phase 6's
remaining driver pairs — input, console) and `MAX_SPAWN_RECS` 8 → 12
(five resident production services now hold records forever, and
user-spawned children leak theirs until a GC design exists —
headroom now, the debt named below).

## Consequences

- One copy of the virtio 1.0 core. storaged −268 lines, netd −268
  lines, the shared core +~590 (docs-heavy); rngd's device-specific
  code is ~300 lines instead of the ~600 a third copy would have
  cost. Phase 6's remaining drivers (input, console) start from the
  core.
- The extraction ran under the full suite at every step (house rule:
  old tests are never deleted; green means every milestone still
  completes) — 11/11 before a line of rngd existed.
- `RingMem` records the ONE seam where the drivers genuinely differ
  (frame budget vs. ring depth); the third driver validated it —
  rngd is `Packed`, like netd.
- rngd's zero-copy fill is the first WRITE-into-caller-memory service
  path; Phase 7's stack will reuse the pattern for RX delivery into
  stack-owned buffers.
- The entropy proof is transport-and-fill, not cryptographic quality
  (the backend's responsibility) — documented here so no future
  reader mistakes the suite's reach.
- Fixture portability: bare `-device virtio-rng-pci` relies on QEMU's
  `rng-builtin` default backend (the platform CSPRNG via
  `qemu_guest_getrandom`) — the default since QEMU 4.1 (2019), so
  every host in practical use; museum-piece QEMUs use the explicit
  `rng-random` object form (RUNNING.md documents both). Omitting the
  device entirely is legal forever (honest SKIP).
- Spawn-record debt: records of user-spawned children are never
  reclaimed (no GC in v1). Five resident services + shell sessions
  stay under 12, but the driver-hardening step of Phase 6 should
  design the reclamation (auto-release on child destroy unless a
  waiter holds the record) before the caps matter again.
