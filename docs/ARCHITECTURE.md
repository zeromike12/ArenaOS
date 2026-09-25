# ArenaOS — Architecture

Status: **Accepted** (living document — update via ADRs, keep in sync)
Scope: system-wide architecture. Milestone-level detail lives in
[ROADMAP.md](ROADMAP.md); decision records live in [adr/](adr/).

---

## 1. Bird's-eye view

```
┌───────────────────────────────────────────────────────────────────────┐
│ Applications        (desktop apps, CLI tools)                         │
├───────────────────────────────────────────────────────────────────────┤
│ Application APIs    (toolkit, async runtime libs — userspace, later)  │
├──────────────┬──────────────┬──────────────┬──────────────────────────┤
│ System       │ Filesystem   │ Network      │ Graphics/audio services  │
│ services     │ servers      │ stack        │ (compositor, mixer)      │
│ (namespace,  │ (on block    │ (on net      │                          │
│  supervisor) │  servers)    │  drivers)    │                          │
├──────────────┴──────────────┴──────────────┴──────────────────────────┤
│ Device driver servers (userspace: virtio-block, virtio-net, console,  │
│                        input, display — isolated, restartable)        │
╞═══════════════════════════════════════════════════════════════════════╡
│ KERNEL  (the only ring-0 code)                                        │
│  objects: threads, address spaces, capability spaces, endpoints,      │
│           untyped memory, interrupt ports                             │
│  mechanisms: scheduler, virtual memory, IPC fast paths, capability    │
│           invocation, interrupt dispatch, timer                       │
├───────────────────────────────────────────────────────────────────────┤
│ Boot stage (UEFI application: firmware → kernel hand-off)             │
├───────────────────────────────────────────────────────────────────────┤
│ UEFI firmware → x86-64 hardware                                       │
└───────────────────────────────────────────────────────────────────────┘
```

Everything above the double line runs in userspace and communicates through
capability-based IPC. Everything below it is the kernel's small, fixed
mechanism set.

## 2. Kernel architecture — options considered

| | Monolithic (Linux-style) | Classic microkernel (L4, seL4, MINIX 3) | Hybrid (NT, XNU) | **Capability hybrid (ArenaOS)** |
|---|---|---|---|---|
| Where drivers/FS/net live | kernel | userspace | kernel (mostly) | userspace |
| Fault isolation | none | strong | weak | strong |
| IPC on critical paths | rare | very frequent | frequent | control: sync rendezvous; bulk data: shared memory |
| Raw syscall throughput | best | worst-case bad, best-case fine (L4) | middling | fine (small ABI, tuned fast paths) |
| Kernel size/debuggability | huge, hard | tiny, provable | large, hard | small, auditable |
| Retro-fitting security | impossible | native | partial | native (capabilities are the object model) |
| Perf risk profile | known-good | historic weakness | known-mediocre | manageable with shared-memory design |

**Decision (ADR-0002): a capability-based hybrid microkernel.**

- *Micro* in structure: the kernel implements mechanisms only — address
  spaces, threads, scheduling, IPC, capability rights checking, interrupt
  dispatch, physical/virtual memory management. No filesystem, no network
  stack, no device drivers (beyond the interrupt controller, timers, and
  early boot console).
- *Hybrid* in pragmatism: performance-critical mechanisms (VM mapping, IPC
  copy paths, scheduling) live in the kernel rather than in servers, and the
  boot console/framebuffer path may keep a minimal in-kernel fallback for
  crash diagnostics. We do not chase seL4-style formal minimality; we chase
  *auditable* smallness.
- *Capability-based* as the object model: the only way to reference any
  kernel object or service resource is a capability stored in a process's
  capability space, carrying explicit rights. Syscalls are capability
  invocations. This simultaneously gives us the security model (ADR-0006),
  the sandboxing story, and the delegation/revocation semantics that a
  modern permission system needs.

Why not monolithic: our stated goals (driver isolation, recoverable services,
application sandboxing, 20-year security posture) are architecturally
impossible to retrofit into a monolith — Linux's own history is the evidence.

Why not exokernel/unikernel: they optimize for single-appliance trust domains;
we want a general-purpose multi-application desktop system. Their ideas
(minimal abstraction, app-managed resources) reappear in our *untyped memory*
objects and shared-memory IPC, where they fit.

Known cost, accepted: userspace drivers mean IPC discipline matters. Mitigation
is designed in from the start (§7), not discovered in Phase 6.

## 3. Boot architecture (ADR-0003)

No third-party bootloader (no GRUB, no Limine). **The boot stage is a UEFI
application** — a PE32+ image the firmware loads directly:

```
UEFI firmware
  → loads EFI/BOOT/BOOTX64.EFI (our boot stage, Rust, x86_64-unknown-uefi)
  → boot stage: verify CPU state (long mode, paging on, NX if present)
  → install own GDT **and IDT** — they must change hands together: firmware
      re-enables interrupts (`sti`) when boot-service calls restore
      TPL_APPLICATION, so a firmware IDT + our GDT (or vice versa) faults
  → interrupts stay disabled in our own code
  → bring up serial console (16550, polled)
  → parse UEFI memory map, configuration tables (ACPI RSDP, SMBIOS later)
  → [M2+] build kernel page tables (higher-half kernel, W^X, NX),
      set up interrupt/exception handling, frame allocator
  → ExitBootServices(), hand off to kernel proper with a boot-info record
  → kernel initializes scheduler, root task, capability subsystem
  → root task spawns first services (supervisor, console, storage)
```

Consequences: full control of the boot chain (a prerequisite for secure boot
later), zero external runtime dependencies, and our UEFI bindings are our own
code (ADR-0004). During Milestone 1 the boot stage *is* the whole kernel —
one binary; the split into boot stage + kernel proper arrives with paging in
Milestone 2.

### Interrupt-controller handoff (M1 findings, probed at runtime)

EDK2/OVMF on QEMU q35 hands off with the platform in **IOAPIC → local-APIC
mode**: the 8254 PIT tick arrives as LAPIC vector 32, while the legacy 8259
pair is fully masked (IMR=0xFF/0xFF) **with its vector base left unprogrammed
(irq_base=0)**. Two consequences, both verified the hard way (see the case
study in `docs/TESTING.md`):

1. An interrupt stub that EOIs only the 8259 leaves the LAPIC in-service bit
   set, silently blocking every subsequent vector at that priority — at most
   one interrupt is ever delivered. Our absorb stub EOIs **both** controllers
   (8259 cascade ports, then the LAPIC EOI register at the default APIC base
   0xFEE00000; both are no-ops when nothing is in service).
2. Unmasking 8259 IRQ0 without first reprogramming ICW2 makes the ExtINT
   acknowledge through LINT0 deliver **vector 0x00** — a #DE through gate 0.
   Never touch the 8259 masks until the vector base is programmed.

From M2 (post-`ExitBootServices`) the kernel owns both controllers outright:
it will read `IA32_APIC_BASE`, keep the LAPIC as the delivery path (per-CPU
timer/IPI later), and either fully reprogram or permanently disable the
legacy PIC.

Fallback plans documented: if UEFI diversity becomes painful, a thin
multiboot2/BIOS path could be added *under the same boot-info contract* —
the kernel proper never knows who produced the boot-info record.

## 4. Memory model

- **Physical:** firmware memory map → boot-info record → physical frame
  allocator (M2). UEFI runtime regions are preserved forever; boot-services
  memory is reclaimed after ExitBootServices.
- **Virtual:** the kernel runs in a higher-half address space with strict
  W^X, NX everywhere it applies, and no mapping that is both writable and
  executable. Each process gets its own address space object; the kernel
  mapping is not visible to user pages. Page-table machinery is arch-isolated
  (`kernel/*/src/arch/`).
- **User-visible memory:** *untyped memory* capabilities (Zircon/seL4
  lineage): the basic allocatable resource is a range of physical memory with
  no semantics; address spaces are built by mapping untyped into VMAR-like
  region objects. This keeps the kernel's memory ABI tiny while letting
  userspace allocate anything (heap, shared buffers, DMA-able regions) from
  the same primitive.
- Kernel heap (M2) is a segregated allocator over the frame allocator, with
  poisoning and bounds checks in debug builds.

## 5. Process & thread model

- **Process** = address space + capability space + resource limits. Not a
  unit of execution.
- **Thread** = execution context scheduled by the kernel, belonging to
  exactly one process.
- **Spawning** is explicit and capability-mediated: a spawner holds a
  capability to an executable image object and a set of capabilities to hand
  over; the kernel creates the new process with *exactly* those. There is no
  `fork`: no inherited address space, no inherited handle table, no COW
  semantics leaking into the object model. (A `posix_spawn`-like convenience
  can live in a userspace compatibility layer someday.)
- No signals. Asynchronous events are delivered as IPC notifications to
  endpoints the process holds (§7).
- Scheduling: per-CPU run queues, preemption via timer interrupt, priority
  bands with anti-starvation aging; deterministic-enough for testing (an
  explicit "round-robin, no heuristics" mode for regression tests). Details
  in M3 ADR when it lands.

## 6. Kernel/user boundary

- Ring 3 for all userspace; ring 0 for the kernel only. Single syscall entry
  via `syscall`/`sysret` on x86-64 (MSRs set up in M3/M4), with a well-defined
  register ABI (ADR-0006): syscall number + capability handle + arguments in
  registers; small structured arguments inline, anything larger via IPC
  buffers. Typed status codes returned — no errno namespace.
- SMAP/SMEP enabled when the CPU supports it; supervisor never dereferences
  user pointers except through explicit, checked copy routines.
- The kernel ABI is versioned and small on purpose (~30 calls target): object
  creation/destruction, capability manipulation (move/copy/attenuate/destroy),
  IPC (call/notify/wait), thread/address-space/memory management, time.
  Everything else is userspace.

## 7. IPC philosophy

Three primitives, one rule ("never copy what you can share"):

1. **Synchronous call/reply** between *endpoints* (kernel objects). Rendezvous
   semantics: small messages copied inline; the caller's thread donates its
   time slice to the callee (priority inheritance falls out naturally).
2. **Asynchronous notifications** — badged, merged event flags (L4 lineage).
   This replaces signals and interrupt delivery to userspace drivers: the
   kernel turns hardware interrupts into notifications on a capability the
   driver server holds.
3. **Shared-memory channels** — bulk data moves through memory shared by
   capability (untyped mapped into both address spaces), with rings and
   futex-like waits. A file read of 1 MiB must not cost a 1 MiB kernel copy.

No global names. Service discovery is capability introduction: the root
namespace service is itself reached by a capability given to processes at
spawn. This is the Plan 9/seL4 lesson: names are data, not ambient magic.

## 8. Driver philosophy

- Drivers are **userspace servers**. The kernel provides: interrupt
  notification capabilities, MMIO/PIO range capabilities, DMA-capable untyped
  memory, and IOMMU mappings (when hardware allows). A driver crashing loses
  its own state; the supervisor restarts it and re-grants capabilities.
- **VirtIO first.** QEMU's virtio devices (console, block, net, GPU later)
  give us one well-specified family covering all Phase 6/7 needs. Real
  hardware drivers are explicitly out of scope until the driver framework has
  survived VirtIO in daily use.
- No driver may be loaded into the kernel. Crash diagnostics keep a minimal
  polled serial/console path in the kernel for panic output only.

## 9. Security philosophy

- Capabilities are the security model (§2). Rights are attenuable on
  delegation; revocation is a first-class operation (derived-capability
  revocation trees, design due by M4).
- **Permission manifests:** applications declare required permissions in
  their package metadata; the installer/supervisor translates user grants
  into capability sets at spawn. Enforcement is structural (you can't use a
  capability you don't hold), not advisory.
- W^X, NX, SMAP/SMEP, no user mappings in the kernel's view, KASLR once the
  loader supports it.
- Secure boot chain (signed boot stage → measured kernel) and signed packages
  are Phase 8+ concerns, but the boot design (§3) already owns every link of
  the chain, so nothing must be retrofitted.
- Kernel attack surface shrinks by construction: no ioctl forest, no
  filesystem parsers, no network stack in ring 0.

## 10. Filesystem philosophy (direction; design ADR due in Phase 5)

- Filesystems are **userspace servers** on top of block-device capabilities.
  The kernel stores no file semantics.
- Files are **objects, not byte-stream simulacra of devices**: a handle is a
  capability to a file object with rights (read/write/truncate/…); sockets,
  devices, and files are *different typed objects*, not one namespace
  pretending otherwise. "Everything is a file" is explicitly rejected.
- Storage model direction: extent-based data with copy-on-write and
  transactional metadata updates (crash consistency without fsck rituals),
  content-addressed dedup considered but not promised.
- Namespaces: a mount-graph service resolves paths *once* at open time into
  capabilities; afterwards, all I/O is direct to the file object. Paths are a
  UI convenience, not an API substrate.

## 11. Syscall/API philosophy (ADR-0006)

- Small, stable, versioned **kernel ABI** (capability invocation register
  ABI) + **userspace protocols** (files, net, graphics) negotiated between
  services, free to evolve.
- Errors: typed status codes (domain + code), never string tables; no
  restartable-syscall/signal interplay because there are no signals.
- Async-first userspace ergonomics: blocking kernel calls stay simple and
  synchronous; asynchrony is achieved with notification endpoints + worker
  threads in libraries, and later an io_uring-style batching call if profiles
  demand it. We do not put an async runtime in the kernel.
- No POSIX names by default (`open/read/write/ioctl` will not define our
  semantics); a compatibility subsystem can map them onto our objects later.

## 12. Graphics (Phase 9+, sketch only)

```
display driver server (virtio-gpu / GOP fallback)
  → kernel-provided buffer sharing (untyped + scanout caps)
  → compositor service (owns screen, input routing)
  → toolkit library (client-side rendering into shared buffers)
  → shell / applications
```
GOP gives us "pixels on screen" from day one of Phase 9 without a real GPU
driver; virtio-gpu gives us acceleration in QEMU. Deliberately not designed
further until storage/drivers/userspace are stable — designing a compositor
on top of a nonexistent IPC layer is how OS projects die.

## 13. Portability

- Arch-specific code is quarantined in `arch/` modules behind narrow
  interfaces (cpu state, paging, context switch, interrupt glue, syscall
  entry). The rest of the kernel must compile for any arch.
- Boot-info record is arch-neutral by contract: a future ARM64 boot stage
  produces the same shape of record.
- x86-64 is the only target for Phases 0–8. ARM64 is a design constraint
  (nothing may assume x86 paging/interrupt models outside `arch/`), not a
  deliverable.

## 14. Current implementation state

| Component | State |
|---|---|
| Boot stage (UEFI app, serial, GDT, CPU-state verification, memory-map parsing) | **Milestone 1 — implemented, tested in QEMU/OVMF** |
| Kernel proper, paging, interrupts, scheduling, IPC, capabilities | not started (roadmap M2–M4) |
| Userspace, drivers, FS, net, graphics | not started |

The architecture above is the commitment; the roadmap is the sequence.
