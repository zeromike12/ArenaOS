# ADR-0021: Driver substrate — untyped caps, user MMIO, IRQ relays, and the kernel/user split for PCI

- Status: accepted
- Date: 2026-09-26
- Milestone: M5.1 (ROADMAP phase 5 — Storage)
- Supersedes: — (extends ADR-0015 capability spaces, ADR-0018 IPC, ADR-0020 spawn/hand-off)

## Problem

Phase 5 puts a VirtIO-blk driver in USERSPACE (ARCHITECTURE §10:
filesystems and device servers are userspace processes; ring 0 keeps no
device protocol beyond the interrupt controllers and the console). A
userspace driver on this kernel hits four walls, each needing a
decision:

1. **Memory for DMA.** The device DMAs into guest-physical frames. A
   userspace driver must obtain frames AND know their physical
   addresses — its own virtual addresses are useless to the device.
   Today every `Memory` cap is minted kernel-side for tests; there is
   no user-reachable allocation, and `Memory` caps are *descriptive*
   (destroy frees nothing — right for test fixtures, wrong for
   allocated memory).
2. **Device registers.** VirtIO-pci's modern interface is MMIO in PCI
   BARs. Ring 3 cannot execute `in/out` (PIO) at all, and no mapping
   path exists from a device's physical registers into a user address
   space. Whoever maps MMIO into ring 3 decides which processes can
   touch which devices — an isolation decision, not a convenience.
3. **Interrupts.** Completions arrive as MSI-X writes to the LAPIC.
   The IDT, the vector space, and the interrupt controllers are
   kernel-owned forever; the driver must nevertheless *wait* on its
   device's interrupts with the only blocking primitive userspace has:
   notification caps (ADR-0018).
4. **PCI itself.** Enumeration walks config space (privileged PIO),
   sizes BARs (destructive-ish write-1s probes), sets command-register
   bits — including BUS MASTER, the bit that authorizes a device to
   DMA into RAM at all. How much of this does a driver process get to
   do?

## Decision

### 1. `CapObj::Untyped {phys}` — the first OWNED cap kind

`SYS_ALLOC_FRAME` (registry slot 16) takes one frame from the bitmap
allocator and grants the caller an `Untyped` cap naming it. Unlike
`Memory` caps (descriptive: the granter vouches, destroy frees
nothing), an `Untyped` cap is **owned**: `cap::destroy` on it (DESTROY
right, as always) returns the frame to the allocator — exactly once,
double-frees caught by the allocator itself. One frame per cap in v1:
the allocator hands out single frames, and a virtqueue's three ring
regions (desc/avail/used) plus 512-byte sector buffers never need more
contiguity than that (the virtio spec gives each ring its own
`queue_address`). `cap::map_memory` accepts `Untyped` alongside
`Memory`, so a driver maps its own DMA frames into its own space —
frames whose PHYS it knows from the cap. This is the untyped-memory
seed cap.rs reserved in its own enum comment since M3.4.

### 2. `SYS_MAP_MEMORY` (slot 17) — self-service mapping with kernel-chosen windows

`SYS_MAP_MEMORY(mem_slot)` maps the caller's `Untyped`/`Mmio` cap into
the CALLER's own user half (v1: self-map only — cross-process mapping
stays the existing `cap::map_memory` invoke shape) and returns the
window VA as a positive payload. The kernel chooses the VA: scanning
up from `MMAP_BASE = 0x4000_0000` in 2 MiB strides for a gap clear of
the calling thread's registered regions. Caller-chosen VAs would make
every user process a page-table-collision oracle; kernel-chosen
windows keep the region table authoritative. The window is appended to
the calling thread's region table — `USER_REGIONS_MAX` rises 4 → 16
(driver processes legitimately hold: image, stack, ring windows, data
buffers, several MMIO windows). The requested ACCESS MODE decides the
right: a writable window needs WRITE on the cap, a read-only window
needs READ (the old WRITE-for-everything rule of `cap::map_memory` —
relaxed here and in the invoke alike — made read-only descriptor
windows impossible). **Executability is never granted** through this
call (drivers need data windows, not code pages — W^X without an X
door); MMIO maps uncached (PCD|PWT) and NX always.

Mapping an `Untyped` cap CONSUMES it: ownership of the frame transfers
to the address space (the cap slot goes empty, which is why the call
also demands DESTROY — handing a frame away is an act of ownership).
From then on the frame has exactly one owner — the page tables — and
`proc::destroy`'s teardown walk reclaims it like any other mapped RAM
leaf. The walk, in turn, learned to skip leaves whose physical address
lies outside allocator-managed RAM: those are MMIO windows, and device
registers were never ours to free. Exactly one owner at every instant
means exactly one free, with no ordering discipline asked of drivers.

Known v1 limits, recorded not hidden: the window uniqueness check is
per-THREAD (the region table's granularity) while page tables are
per-process — sound for the single-threaded services of M5, with a
process-wide VA-space object deferred until multithreaded drivers
exist; no unmapping call yet (windows live as long as the process;
teardown reclaims them — RAM leaves through the allocator, MMIO leaves
never).

### 3. `CapObj::Mmio {phys, pages}` — minted by the kernel, never by users

MMIO caps describe device register windows. They exist because the
kernel says so — minted during PCI enumeration (one per virtio
structure region: common cfg, notify, ISR, device cfg, MSI-X table)
and, for the M5.1 test, over the HPET main-counter page. No syscall
mints an MMIO cap from a user-supplied address: if any process could
declare `0xFEE0_0000` "my MMIO", it could reprogram the interrupt
controller from ring 3. The 5.2 driver receives its device's MMIO
caps as spawn grants/invoke results, keyed by a `VirtioDevice` cap —
possession of the device cap is the authority; the cap kinds are the
mechanism. (Ring-3 MMIO proof for 5.1: a user process maps the HPET
counter page RO and observes it strictly increase across two reads —
a real device register, ticking, read at CPL 3.)

### 4. IRQ relays: vector range 48..63 → notification caps

Sixteen IDT slots in the MSI range get relay stubs (the exception-stub
macro pattern: push vector, common frame, dual-controller EOI BEFORE
any scheduler-adjacent work — the ADR-0013/0020 discipline). A relay
registration binds `(vector → nid, badge)`; the handler's whole body
is `ipc::notify(nid, badge)` — the same enqueue-only wake the console
RX ISR uses, so completions park and resume drivers through the
PROVEN `wait` path. `SYS_IRQ_RELAY` (slot 18, arriving with 5.2) arms
one relay against the caller's notification cap (WRITE) and returns
`(vector, msi_address, msi_data)`; the kernel writes the device's
MSI-X table entry itself (the table is inside a BAR the kernel also
maps — a driver that could write arbitrary MSI-X entries could aim
interrupts at other devices' relays). Vector allocation is the
kernel's; nothing else about the interrupt is.

### 5. The PCI split: kernel owns POLICY, userspace owns MECHANISM

The kernel enumerates PCI at boot (bus 0 scan via config PIO
0xCF8/0xCFC, vendor/device/class/BAR capture), walks each virtio
device's vendor-specific capability list (cap ID 0x09: the virtio
structures' BAR/offset/length records + notify multiplier), sets the
command register's MEMORY SPACE and BUS MASTER bits, and records one
descriptor per device. Config space is never exposed to ring 3:
BUS MASTER is the DMA authorization decision and BAR sizing writes are
bus-wide side effects — both are exactly the "policy" the kernel keeps
(ARCHITECTURE §12's shrink-by-construction: no driver can enable
another device's DMA, retarget another's BAR, or spoof enumeration).
The virtio PROTOCOL — feature negotiation, status machine, virtqueue
setup and kicking, request handling — is all common-cfg/notify MMIO
per the virtio 1.x spec, so it lives entirely in the userspace driver
(5.2) with zero config-space access. Legacy (PIO) virtio interfaces
are thereby unsupported by design: a ring-3 driver cannot do PIO, and
a kernel PIO proxy would rebuild the ioctl forest this OS refuses.
QEMU's `virtio-blk-pci` provides the modern interface; physical-hardware
bring-up (Phase 8) inherits the same rule.

### 6. The scratch disk is a harness fixture

QEMU grows `-drive file=build/scratch.img,format=raw,if=none,id=scr0
-device virtio-blk-pci,drive=scr0` (8 MiB, recreated fresh per test
run; the stability loop and release verification reuse it read-consistently).
The ESP stays the boot medium — the milestone's persistence story
(5.4) is about the scratch disk surviving across boots within a test,
not about booting from it.

## Approaches considered

- **Kernel virtio-blk driver (5.1 shortcut), userspace later** — fastest
  path to a working disk, and rejected: the phase outline says
  userspace driver, ARCHITECTURE §10 says the kernel keeps no device
  protocol, and a kernel-first transport is scaffolding that gets
  thrown away (or worse, stays). The substrate-first order builds only
  parts that survive.
- **ECAM/MMCONFIG config access mapped into the driver** — real systems
  do this; it needs ACPI/MCFG parsing (a new table walker) and then
  STILL leaves bus-master/DMA authorization as a kernel decision, so
  the "driver does config" story buys nothing for v1. Kernel PIO
  enumeration is ~200 lines and keeps policy where it belongs. Deferred
  until a use case needs runtime config changes (hotplug, Phase 8).
- **Raw syscalls for config read/write (`pci_conf_read(bdf, off)`)** —
  the ioctl-forest anti-pattern with an isolation hole (any process
  reads/writes any device's config). Rejected.
- **PIO proxy syscall for legacy virtio** — rejected above (rebuilds
  the ioctl forest; QEMU's modern interface makes it unnecessary).
- **Interrupt delivery without relays (driver polls ISR-status MMIO)**
  — works, and is how 5.2's fallback path could survive a relay bug,
  but polling as the design burns the CPU this OS's async-first
  ergonomics (ARCHITECTURE §11) explicitly refuse. Relays reuse the
  notification/wait machinery already proven under 100-boot stability.
- **Caller-chosen VAs in `SYS_MAP_MEMORY`** — rejected above
  (collision oracle; kernel-chosen windows keep the region table the
  single source of truth for `user_range_ok`).
- **Multi-frame Untyped caps now** — the allocator is single-frame and
  the virtqueue geometry never needs contiguity beyond one frame;
  pages-in-cap grows when a real demand (huge rings, buffer chains)
  shows up.

## Consequences

- The registry grows slots 16/17 in 5.1 (18 with 5.2's relay arming);
  the ABI's frozen-encoding promise is untouched (same six-register
  marshalling, same typed statuses).
- `cap::destroy` becomes kind-aware: the first cap kind whose
  destruction mutates global state (the frame allocator). The
  rights gate (DESTROY) and the teardown-exactness discipline now
  carry real weight — m5 asserts frame accounting through
  alloc/map/destroy cycles.
- `frames::free` now REFUSES addresses outside conventional RAM — the
  module always promised "freeing an unmanaged address is a checked
  error", and 5.1 bring-up made it true (live evidence: the teardown
  walk "freed" the HPET page into the RAM pool, because the
  APIC/HPET/PCI apertures all sit BELOW the allocator's 4 GiB span
  limit — only a memory-map containment check, `frames::is_ram`,
  tells RAM leaves from MMIO leaves; the walk uses the same predicate).
- Region table growth (4→16) touches `sched`'s Thread, `spawn`'s
  records, and every region-array constant in one mechanical pass;
  per-thread regions stay the v1 truth (process-wide VA object
  deferred, recorded above).
- The IDT's relay range (48..63) is carved out of the absorb-stub
  space: `init` and `audit_gates` change together (the M1 boot audit
  pins every gate encoding — the M4.6 lesson applied preemptively).
- IRQ relay handlers are the third wake-from-interrupt-context site
  (tick preemption, console RX, relays) — all enqueue-only by
  invariant; the stability loop keeps hammering the pattern.
- Every later userspace driver (net, gpu, input — Phases 6-9) inherits
  this substrate unchanged: Untyped for DMA, MMIO caps for registers,
  relays for interrupts, kernel enumeration for authorization. That is
  the point of doing it first.
- Deferred (documented, deliberate): process-wide VA spaces, unmap,
  multi-frame/huge allocations, ECAM, config-space delegation, IRQ
  sharing, INTx, IOMMU (QEMU-default off; a hardening-phase topic),
  MSI-X per-queue vector fan-out beyond one relay per device.
