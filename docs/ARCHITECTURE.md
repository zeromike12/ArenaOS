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

### Physical memory (M2.3)

One gateway to physical memory: `frames.rs`, a flat bitmap (one bit per
4 KiB frame over [0, 4 GiB), ADR-0007) punched out of the captured UEFI
map's *conventional* regions — runtime services, ACPI NVS, reserved/MMIO,
and firmware memory are structurally unallocatable, not filtered at alloc
time. Frames below 1 MiB stay reserved (legacy structures). Checked error
semantics: double frees and out-of-span frees are rejected, exhaustion is
reported, and the M2 test proves the managed set equals the map exactly
(recomputed independently), then stresses it (uniqueness, real-RAM pattern
round-trips, 200-round alloc/free accounting, contiguous runs, ~100 ns/op
hot-path benchmark). The allocator is single-CPU by boot contract; the
kernel proper replaces the mutation discipline (not necessarily the layout)
once locks (M2.6) and SMP structures (M3) exist.

### Virtual memory (M2.4)

The boot stage installs its own 4-level page tables (`paging.rs`,
ADR-0008) and switches CR3 in place — no trampoline, because the identity
view keeps every executing address valid across the switch. Two views coexist
until ExitBootServices (M2.7):

* **identity** `[0, 4 GiB)` — every firmware-described region, RW+X. The
  permissive flags are a documented firmware-compatibility carve-out:
  EDK2 executes from memory the map labels Reserved/BootServicesData, and
  firmware (up to the final `ResetSystem`) must keep running. The LAPIC
  MMIO page is mapped explicitly from IA32_APIC_BASE — it is absent from
  the UEFI memory map but the EOI path writes it.
* **kernel direct map** `VA = phys + 0xFFFF_FFFF_8000_0000`, first 2 GiB,
  RW+NX — the kernel's permanent view and the address space habit the
  kernel proper inherits.

Our image window is re-mapped at 4 KiB granularity in *both* views, per PE
section (base/size from the Loaded Image Protocol, bounds from our own PE
headers): `.text` R+X, `.rdata` RO+NX, rest RW+NX. CR0.WP is set, so RO
binds ring 0 itself; EFER.NXE is asserted. The M2 tests prove all three
enforcement paths with recovered faults: a ring-0 write to the RO `.text`
alias (#PF ec=present+write), an instruction fetch through the NX data
alias (#PF ec=present+I/D), and a *call* through the higher-half alias of
`.text` that executes and returns. Bulk mappings use 2 MiB pages; the
window splits huge pages where needed. Total cost at boot: 8 frames.

At `ExitBootServices` (M2.7, ADR-0011) the identity view is torn down as
planned: the kernel-only tables map the direct map, the image window at
its link base, and the interrupt-controller MMIO the kernel drives
(IOAPIC `0xFEC00000`, LAPIC base from `IA32_APIC_BASE`, HPET
`0xFED00000` — the latter two share/adjacent 2 MiB alias blocks). One
deliberate exception survives the tear-down: the **farewell island** page
in `halt.rs` stays mapped executable, because the clean-shutdown path
must be able to hand the machine back to firmware's `ResetSystem` (with
firmware's own CR3) from a kernel-only address space.

### Kernel heap (M2.5) — `kernel/libs/heap` + boot glue

The allocator core is a first-class workspace library crate (`arena-heap`,
zero deps, `no_std`) so its logic is **host-testable**:
`cargo test -p arena-heap --lib --target x86_64-unknown-linux-gnu` runs
12 tests natively, including a 4000-round pseudo-random stress checked
against a `BTreeMap` reference model (ADR-0009).

- First-fit over an address-ordered free list; free coalesces both
  neighbours (invariant: no two free blocks are adjacent).
- 32-byte block header: footprint (×16, FREE in bit 0), user size, front
  red-zone word, free-list link — the link lives in the *header*, so
  poisoning covers every user byte.
- Memory arrives in 64 KiB **chunks** from physically contiguous frame
  runs (`FrameChunkProvider` over M2.3's `alloc_contiguous`); blocks never
  span chunks, chunks are never shrunk; max single alloc ≈ chunk − 64 B.
- **Guards always on at boot**: red zones around every payload, 0xAA fill
  on alloc, 0xDD poison on free. `free` validates the pointer by walking
  chunk block chains; double frees, red-zone violations, and foreign
  pointers are rejected with typed errors and mutate nothing (each
  rejection logs an ERROR line — the causal free is caught, not the
  eventual symptom).
- Boot API: `heap::alloc/free/alloc_typed/free_typed` + stats. Deliberately
  **not** wired as `#[global_allocator]` yet (no Box/Vec users at boot;
  OOM + locking policy belongs to the kernel proper, see ADR-0009).
- Measured in guest: 1920 ops in 1.27 ms (~661 ns/op under TCG with
  guards), accounting exact to the byte.

### Synchronization (M2.6) — `kernel/libs/sync` + irqsave helpers

`arena-sync` (second `kernel/libs/` crate, host-tested) provides
`Spinlock<T>`: test-and-set CAS loop + RAII guard (release-on-drop only —
"forgotten unlock" is unrepresentable), `try_lock`, and **owner tracking**:
each lock records the acquiring executor's token (constant BSP id at
boot); `lock()` debug-asserts on recursive acquisition (located panic
instead of silent self-deadlock), and `owner_token()`/`is_locked()` serve
diagnostics. Host suite proves real parallel behavior (4 threads × 50k
non-atomic increments → exact 200k; recursion `#[should_panic]`); the
guest proves the state machine single-core honestly (ADR-0010). Ticket/
queued variants are a scheduled ADR when SMP lands.

Interrupt control stays an explicit, composed layer:
`arch::x86_64::{read_flags, restore_flags, cli, sti}` and
`sync::without_interrupts(f)` — full-RFLAGS irqsave/irqrestore, so IF is
re-enabled *only* when it was set on entry (nesting-safe). The spinlock
never masks interrupts internally. Boot posture unchanged: IF=0 except
bounded tested windows; the PIT tick still flows through the IDT's
absorb-and-EOI stub for vectors 32..255. The M2.5 heap is now
lock-wrapped (`Spinlock<Heap<…>>`), discharging ADR-0009's promise; all
heap tests re-pass through the lock.

### Timekeeping (M2.2)

The monotonic clock is the TSC; the *meaning* of a microsecond comes from
the PIT oscillator (1 193 182 Hz), the platform's only hardware real-time
reference at this stage. Boot calibrates once, loudly: two independent
~20 ms windows of linear PIT countdown versus TSC delta must agree within
5% and land in plausible bounds, or boot halts rather than run on a
made-up clock. After calibration the PIT channel 0 is left ticking at
100 Hz in square-wave mode 3 — measured fact about QEMU: mode 2's 1 ns OUT
pulses do not reliably drive the IOAPIC→LAPIC edge path, mode 3 (what the
firmware itself used) does. `timekeeping::now_us()` (TSC-based, no I/O) is
the kernel's monotonic time source from M2.2 onward; there is no wall clock
until an RTC/NTP-class source exists (Phase 5+), and no sleeping until the
scheduler (M3).

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

From M2.7 (post-`ExitBootServices`) the kernel owns both controllers
outright and *reclaims* the timer chain in `kmain` (ADR-0011): PIT
channel 0 re-armed at 100 Hz, IOAPIC **pin 2** — QEMU applies the classic
PC convention that ISA IRQ0 maps to GSI 2, so the PIT never arrives on
pin 0 — re-routed from firmware's masked RTE to our vector 32
(fixed/physical/edge/unmasked), LAPIC TPR zeroed, spurious EOI, SVR
enabled. The 8259 stays permanently masked: the LAPIC is the only
delivery path. `kernel_irq_live` proves the whole chain by requiring two
real ticks through the relocated IDT with EOI via the kernel-alias LAPIC
MMIO, and the HPET legacy-replacement bit is defensively cleared (it
reads 0 — firmware never enabled the HPET — because that mode would
silently suppress PIT IRQs at the source).

Since M2.1 the exception side is complete enough to *survive*: a 64-bit TSS
provides IST1 (a dedicated 16 KiB fault stack) for the exceptions that must
work even with a broken stack (#DF, NMI, #MC); the common exception entry
preserves the entire interrupted context (all caller-saved registers saved
around the Rust handler, which is an ordinary ABI-conformant function); #PF
diagnostics include CR2; and the test suite can arm *expected* faults and
resume from them (see `docs/TESTING.md` §exception-path testing). Two
hardware facts learned while building this, now regression-guarded: `ltr`
requires the TSS descriptor type to be *available* (0x9 — LTR sets the busy
bit itself; presenting 0xB is a #GP), and before our IDT is live the firmware
IDT is still active, so early-boot faults cascade silently — the TSS/IDT
install order matters (TSS first, then IDT, both before the first firmware
call).

Fallback plans documented: if UEFI diversity becomes painful, a thin
multiboot2/BIOS path could be added *under the same boot-info contract* —
the kernel proper never knows who produced the boot-info record.

### Kernel threads & context switch (M3.1)

`sched.rs` + `arch/x86_64/context.rs` (ADR-0012) give the kernel its
first execution contexts beyond `kmain`:

* **Switch frame**: the eight Win64 callee-saved registers + RFLAGS +
  RSP (80 bytes), swapped by a ~20-instruction assembly fast path. No
  FPU/SSE state: the image is *audited* to contain zero FPU/SSE/MMX
  instructions at every build (`tools/build.sh` fails otherwise), and
  the audit's escalation path is written into the ADR.
* **Threads**: fixed 64-slot table with stable indices (the RR ready
  ring stores indices), `Ready`/`Running`/`Zombie` states, and deferred
  reaping — a thread never frees the stack it runs on; the next
  scheduler entry does.
* **Stacks**: 32 KiB of contiguous frames per thread, direct-mapped,
  bottom qword canary checked at every switch-away and at reap; a
  corrupt canary halts with diagnostics instead of limping on.
* **Bootstrap thread**: `kmain` itself is slot 0 on the reserved boot
  stack the scheduler does not own; `sched::init()` is an ABI-checked
  step of kernel entry.
* **Discipline**: every decision phase runs under `without_interrupts`
  and ends its borrows *before* the assembly switch runs on raw values —
  nothing may be live across a switch except the frame itself.
  Cooperative (`yield_now`) since M3.1; timer-driven preemption (M3.2)
  reuses the same decision and the same frame — see below.

The M3 suite pins all of it to machine-checked evidence: exact
round-robin interleave order, callee-saved registers round-tripped
*through* a live switch (asm probe), disjoint stack ranges with
depth-200 recursion, and 127-thread churn with frame/heap accounting
exact to the unit.

### Timer-driven preemption (M3.2)

`sched/preempt.rs` + the vector-32 timer stub in `arch/x86_64/idt.rs`
(ADR-0013) turn the RR ring into a preemptive scheduler:

* **Tick path**: the reclaimed PIT tick (100 Hz, IOAPIC pin 2 → vector
  32) hits a dedicated stub that saves *all* caller-saved registers
  (the interrupted thread may hold live values in any of them), EOIs
  both controllers, bumps the absorb counter, and calls a Rust handler
  that invokes the installed scheduler hook — with IF=0 for free
  (interrupt gate).
* **Nested cooperative switch**: when the running thread's quantum
  (PIT ticks per slice, armed by `preempt::enable`) expires and another
  thread is ready, the hook calls the *same* `plan_switch` + assembly
  `switch_context` as `yield_now` — nested inside the interrupt, on the
  interrupted thread's own stack. The switched-away thread's saved RSP
  points into its IRQ frame; resuming it later returns up through the
  hook, handler, and stub epilogue, and its `iretq` restores the exact
  interrupted state (including IF=1). No separate IRQ stacks, no
  switch-at-iretq complexity.
* **The IF=0 invariant**: every scheduler-state mutation runs with
  interrupts masked — cooperative sections via `without_interrupts`,
  the hook via the interrupt gate. A tick can therefore never observe a
  half-finished decision, and preemption needs no locks beyond the
  existing cell discipline.
* **Per-CPU structures**: `CpuSched { current, ready-ring, slice,
  remaining }` × `MAX_CPUS=4` (SMP *structures*, single-core execution;
  `this_cpu()` is 0 until M5 brings up APs).
* **First-run frames are interruptible**: a new thread's synthesized
  frame carries RFLAGS=0x202 (IF=1, amending ADR-0012's 0x2) so it is
  preemptible from its first instruction — an IF=0 first-run thread
  would wedge the CPU if it never yielded (observed live, then fixed).
* **Determinism kept testable**: the preempt tests run threads with no
  yield call at all and assert the *exact* rotation order from a capped
  transition log, and prove timer rotation composes with cooperative
  yields. Phase discipline: no serial output while IF=1 (no console
  lock yet — ADR-0013).

## 4. Memory model

- **Physical:** firmware memory map → boot-info record → physical frame
  allocator (M2). UEFI runtime regions are preserved forever; boot-services
  memory is reclaimed after ExitBootServices.
- **Virtual:** the kernel runs in a higher-half address space with strict
  W^X, NX everywhere it applies, and no mapping that is both writable and
  executable. Each process gets its own address space object; the kernel
  mapping is not visible to user pages. Everything the kernel itself must
  reach under *every* CR3 (device MMIO aliases included) lives in the
  kernel half — `paging::mmio_alias_va` for below-4 GiB MMIO, since
  `phys + KERNEL_OFFSET` wraps into the user half above 2 GiB (M3.3b). Page-table machinery is arch-isolated
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
  unit of execution. (Since M3.3b the address-space half is implemented:
  `proc.rs` objects own a private PML4 whose kernel half is cloned from
  the kernel view; threads carry their process's root as CR3.) Since
  M3.4 each process also anchors its capability space (`cap.rs`,
  ADR-0015); resource limits follow with the M4 syscall surface.
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
| Kernel proper: exceptions/TSS, timers & monotonic clock, frame allocator, own page tables (higher-half, W^X), guarded heap, spinlocks/irqsave, boot split (EBS → kernel entry, reclaimed timer chain) | **Milestone 2 — implemented, 21/21 in-guest + 100/100-boot stability (ADR-0007…0011)** |
| Kernel threads + cooperative context switch (callee-saved frame, canaried stacks, exact-accounting reap) | **M3.1 — implemented, 5/5 in-guest (ADR-0012)** |
| Timer-driven preemption (tick hook → nested cooperative switch, per-CPU run-queue structures, exact-RR determinism proven on yield-free threads) | **M3.2 — implemented, 7/7 in-guest + 100/100-boot stability (ADR-0013)** |
| Privilege machinery: ring-3 threads, syscall/sysret, TSS RSP0, SMAP/SMEP | **M3.3a — implemented, 9/9 in-guest (ADR-0014)** |
| Processes = address-space objects (private PML4, cloned kernel half, per-thread CR3, exact teardown) | **M3.3b — implemented, 11/11 in-guest + 100/100-boot stability (ADR-0014 addendum)** |
| Capability spaces: per-process slot tables, attenuation-only rights, copy/move/destroy, gated invokes (`process_root`, `map_memory`) | **M3.4 — implemented, 13/13 in-guest + 100/100-boot stability (ADR-0015)** |
| IPC, endpoints | not started (roadmap M4) |
| Userspace, drivers, FS, net, graphics | not started |

The architecture above is the commitment; the roadmap is the sequence.
