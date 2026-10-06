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
  over (since M4.1 the kernel can validate and load image bytes into a
  process space — `elf.rs`, ADR-0016; the image *object* and the spawn
  protocol are M4.5); the kernel creates the new process with *exactly* those. There is no
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
  via `syscall`/`sysret` on x86-64 (MSRs set up in M3.3a), with a well-defined
  register ABI (philosophy: ADR-0006; frozen encoding since M4.2: ADR-0017):
  number in RAX, up to six arguments in RDI/RSI/RDX/R10/R8/R9, typed i64
  status out (0 = OK, positive = payload, negative = dense error codes) — no
  errno namespace; capability handles are arguments like any other; small
  structured arguments inline, anything larger via IPC buffers (M4.4).
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

**Implementation status (M4.4, ADR-0018):** primitives 1 and 2 are
implemented and proven in-guest across two real processes — endpoints
with blocking call/reply (bounded queues, typed refusals, the scheduler's
`Blocked` state), badged merged notifications, and one-capability
transfer per message (copy-only, attenuation enforced). Primitive 3
(shared-memory channels) is deliberately absent until a bulk-data
consumer exists; priority donation is moot under strict round-robin and
returns when priorities do.

**IPC v1.1 (M5.3, ADR-0023)** added the inline half of "small messages
copied inline": CALL/RECV/REPLY take an OPTIONAL trailing 64-byte
buffer pointer (NULL = exact v1.0 behavior — wire-compatible),
range-checked against the caller's regions, kernel-copied in the
OWNER's context, and ridden inside the call slot. Names and dirents
move through it; bulk data stays in capabilities — the filesystem
service forwards its clients' LENT buffer caps untouched to the block
driver, so file data DMAs disk ↔ client page with no copy in any ring.

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
- **Substrate landed (M5.1, ADR-0021):** the promised kernel side now
  exists — OWNED `Untyped` frame caps (`SYS_ALLOC_FRAME`; destroy
  returns the frame, mapping consumes the cap into the address space),
  `SYS_MAP_MEMORY` self-map windows at kernel-chosen VAs (never
  executable; MMIO always uncached+NX), kernel-minted `Mmio` caps,
  IRQ relay vectors 48..63 delivering into notification badges, and
  boot-time PCI enumeration that resolves VirtIO capability structures
  and keeps config space / DMA authorization (MEM|BUS MASTER) as
  kernel policy. The VirtIO mechanism itself arrives in ring 3 (M5.2).
- **Block service landed (M5.2, ADR-0022):** `userspace/storaged` is
  the first resident driver SERVER — spawned at boot with an `Mmio`
  cap over the virtio structure BAR, an endpoint's serve side, and an
  interrupt notification. It discovers the device through
  `SYS_DEV_INFO` (the kernel's resolved scan record, gated on the
  Mmio cap; config space never crosses the boundary), runs the whole
  virtio 1.0 handshake and one split virtqueue in ring 3, and arms
  its MSI-X completions with `SYS_IRQ_RELAY` — the kernel programs
  the table entry and enable bit through a pre-wired kernel PCI
  window and registers a pid-OWNED relay swept at `proc::destroy`.
  Data flows zero-copy: callers lend a buffer cap (an `Untyped` copy
  is structurally LENT — `owned: false` — so it can never free or map
  the frame), the driver points descriptors at the phys it learns via
  `SYS_CAP_PHYS`, and the device DMAs the caller's own page. A driver
  crash loses its own state; restart supervision is a later phase.
- **Network service landed (M6.1, ADR-0024):** `userspace/netd` is
  the SECOND resident driver server and the template's proof of
  generality — same grant shape (Mmio + serve side + notification),
  same discovery/handshake/relay discipline, but two virtqueues, two
  interrupt badges, and an async receive path. Link layer only: raw
  Ethernet frames in/out; protocols are Phase 7's business. RX v1
  delivers through the reply's inline message — the lent-cap rule
  (drivers can never map callers' buffers) makes a copy-into-caller
  structurally impossible, and the full-frame buffer-handoff design
  belongs to the stack that needs it. Absent device → the service is
  honestly offline and the suite SKIPs (never fakes a PASS).
- **The driver family became a family (M6.2, ADR-0025):** with a
  third instance the common code stopped being a guess, so
  `userspace/virtio.rs` now owns the virtio 1.0 core every driver
  server shares — discovery, the window map, the §3.1 handshake, the
  frame budget, the split-queue setup and ring primitives — while
  each driver keeps its grant layout, its exit-code contract, its
  serve loop, and its device specifics. The core reports typed stage
  failures (`VErr`); the driver maps them to ITS codes, so the
  suites' diagnostics are unchanged. The extraction ran mechanically
  under the full suite (11/11 green before the third driver existed)
  — the discipline this project applies to every refactor: prove
  sameness first, add behavior second.
- **Entropy service (M6.2):** `userspace/rngd` is the third resident
  driver and the first WRITE-into-caller-memory service — `RNG_GET`
  points a device-writable descriptor at the caller's LENT frame, so
  the device DMAs randomness straight into the client's own page. The
  proof is variance, not expected values (a fixed expectation would
  be fixed entropy — a fake): two draws must be full-length,
  non-zero, non-constant, and different, with the kernel counting one
  interrupt per draw. Quality of the entropy itself is the host
  backend's responsibility and is stated as such, never claimed.
- **Input service (M6.3):** `userspace/inputd` is the fourth resident
  driver and the first whose queue the DEVICE writes unprompted — the
  driver keeps 32 event buffers posted at all times, because QEMU's
  virtio-input drops a whole event batch it cannot place. It is also
  the first driver that acts on the kernel's behalf rather than only
  on a client's: holding `CapObj::ConsoleInput`, it pushes decoded key
  bytes into the console's line discipline through `SYS_CONSOLE_PUSH`
  — the same entry point the COM1 RX ISR uses. The shell therefore
  gained a keyboard without changing a line: one line editor, one
  blocking read, two hardware sources, and a machine with no keyboard
  behaves exactly as before. The capability doubles as the mode
  switch (a zero-length push is the probe), so the same image serves
  keys over IPC when the suite withholds it. Layout, repeat, LEDs,
  and pointing devices are explicitly out of v1 (ADR-0026).
- **Console channels (M6.4):** `userspace/consoled` is the fifth
  driver, and the milestone that settled what a console IS here. A
  port moves bytes; a console is the line discipline on the way in and
  everything the machine prints on the way out. Both of those are
  kernel objects, so the kernel keeps ONE console and drivers attach
  CHANNELS to it: inbound through the `ConsoleInput` gate the keyboard
  already uses, outbound through a new `ConsoleOutput`-gated MIRROR of
  the console's byte stream (`SYS_CONSOLE_ATTACH` / `SYS_CONSOLE_PULL`).
  The shell was not touched; serial remains the kernel's own for logs
  and panics; a machine with no console device is exactly what it was.
  The two capabilities are separate objects so that a keyboard cannot
  gain the power to read everything the machine prints — only the one
  process that is meant to BE a console holds both. The mirror's tap
  never wakes its reader: `serial::putc` runs inside the scheduler's
  own log lines, so the wake is owed there and paid on the timer tick
  (ADR-0027).
- **Supervision (M6.5):** isolation is only half a promise — "the
  kernel survives your driver" is worth little to the program that
  was USING the driver. Three things make the other half true. A
  service that dies now produces a typed `STATUS_SERVICE_GONE` for
  everyone it owed a reply to (found by capability: the endpoints it
  held `Endpoint`+READ for), instead of leaving them blocked forever
  with no timeout and no way to see it. A process with parked threads
  can finally be KILLED, which a supervisor needs because a driver
  worth restarting is almost always blocked. And `supervise.rs`
  respawns the image with its grant list replayed, while the ENDPOINT
  — and therefore every client capability — survives untouched, so a
  restart is not an event clients have to handle beyond retrying one
  call. What the kernel does NOT claim: that the failed request did
  not happen (it cannot know), or that a restarted driver has any of
  its old state (ADR-0028).

- **Time (M7.0):** ring 3 can now measure time and be woken by it.
  A timer delivers a BADGE on a notification the caller already
  holds, which means it composes with everything already built: every
  service parks in one `SYS_WAIT` on merged badge bits, so a timeout
  is one more bit rather than a second thing to block on. That choice
  is why no service needed restructuring to gain timeouts and why
  none can be written such that it cannot receive them. The
  capability gate is the notification itself — the third time an
  existing authority turned out to be the right gate for a new
  mechanism, which is usually the sign that the mechanism is doing
  exactly one thing (ADR-0029).

- **Where the network stack ends (M7.1):** `netstackd` owns protocol
  state; `netd` owns the device and has never heard of an ethertype.
  The split is enforced by capability rather than convention — the
  stack holds no device authority at all — and the reason is
  operational: a driver must survive its device and be restartable,
  a stack must hold state across time, and one process cannot do
  both without losing every connection each time the NIC wedges. The
  driver's one concession to protocols is a DEADLINE on receive,
  which is about time rather than packets, and exists because a
  client blocked in `SYS_IPC_CALL` cannot observe its own timer
  (ADR-0030, and ADR-0029's erratum).

- **"The service is gone" must be answerable everywhere (M7.1b):**
  a typed error that covers one of two paths leaves the other exactly
  as broken as it was, and looks tested. ADR-0028 answered callers who
  were in flight when a server died; a call sent afterwards still
  queued on an endpoint nobody would read. Endpoints are now marked
  orphaned when their server is destroyed and refuse new calls, with
  the flag cleared by whoever next takes up the serve side. The stack
  above treats that answer as "re-establish", not "retry" — it
  re-acquires its device facts and only repeats operations that are
  idempotent.

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

## 14. Application platform and foreign ABI boundary (ADR-0080)

Applications, app instances, process groups, and windows are distinct
userspace concepts. The kernel remains a mechanism provider for generic
processes, threads, capabilities, IPC and VM; app names, package IDs, window
handles and integer runtime handles never authorize kernel actions. Native
ArenaOS processes keep the capability-oriented syscall ABI. Any future
non-native syscall ABI is selected only by a trusted launcher and proxied to a
userspace compatibility server; the kernel does not interpret its syscall
numbers or semantics. Phase 12 is building this platform on the Phase-11
filesystem/desktop without turning ArenaOS into Linux. ADR-0081 freezes APB1,
ADR-0082 chooses protected AFS2 staged activation, and ADR-0083 freezes an
additive userspace-owned startup ABI. ADR-0086 adds a metadata-free occupancy
query so the entry gate verifies the complete 64-slot capability inventory,
not just listed descriptors. Host no_std verifier/installer/registry
foundations are not yet integrated into protected guest filesd or the desktop.
The ABI-v2 codec and reusable no-alloc startup gate have an independently
linked ring-3 boot proof with five startup RED controls, including refusal of
an unlisted live cap. ADR-0084's bounded native heap is guest-qualified at its
32-page cap with exact process-teardown accounting. ADR-0085 adds per-thread
FS-base save/restore and an independently tested TLS handoff while leaving
GS/`swapgs` unchanged. ADR-0087 records capability-native ProcessGroup
lifecycles. The M12 proof exercises the generation-safe runtime handle-table
core, real attenuated Notification copy/close, and child spawn/wait/reap via an
exact held Process cap. The protected Desktop manager now uses the same
capability-native ProcessGroup for its real children at the existing 12-session
limit; installed-app/helper policy, 32-window scaling, general VM, and
multi-user-thread support remain open. See ADR-0080–0087 and
`docs/phase12/ARCHITECTURE-AUDIT.md` for the audit, limits and staged gates.

## 15. Current implementation state

| Component | State |
|---|---|
| Boot stage (UEFI app, serial, GDT, CPU-state verification, memory-map parsing) | **Milestone 1 — implemented, tested in QEMU/OVMF** |
| Kernel proper: exceptions/TSS, timers & monotonic clock, frame allocator, own page tables (higher-half, W^X), guarded heap, spinlocks/irqsave, boot split (EBS → kernel entry, reclaimed timer chain) | **Milestone 2 — implemented, 21/21 in-guest + 100/100-boot stability (ADR-0007…0011)** |
| Kernel threads + cooperative context switch (callee-saved frame, canaried stacks, exact-accounting reap) | **M3.1 — implemented, 5/5 in-guest (ADR-0012)** |
| Timer-driven preemption (tick hook → nested cooperative switch, per-CPU run-queue structures, exact-RR determinism proven on yield-free threads) | **M3.2 — implemented, 7/7 in-guest + 100/100-boot stability (ADR-0013)** |
| Privilege machinery: ring-3 threads, syscall/sysret, TSS RSP0, SMAP/SMEP | **M3.3a — implemented, 9/9 in-guest (ADR-0014)** |
| Processes = address-space objects (private PML4, cloned kernel half, per-thread CR3, exact teardown) | **M3.3b — implemented, 11/11 in-guest + 100/100-boot stability (ADR-0014 addendum)** |
| Capability spaces: per-process slot tables, attenuation-only rights, copy/move/destroy, gated invokes (`process_root`, `map_memory`) | **M3.4 — implemented, 13/13 in-guest + 100/100-boot stability (ADR-0015)** |
| Executable format + image loader: ELF64 container with ArenaOS strict-subset semantics (ET_EXEC-only validator, W^X segments, zero-fill BSS, exact-accounting load into a process space) | **M4.1 — implemented, in-guest (ADR-0016)** |
| Syscall ABI v1: six argument registers, typed i64 status (0/positive/negative), frozen call registry (debug_write, thread_exit, suite proofs), callee-saved promise proven from ring 3 | **M4.2 — implemented, in-guest (ADR-0017)** |
| First user process: the real rust-lld image, loaded into its own address space (per-thread CR3), runs in ring 3 — verifies its META and zero-filled bss from the user side, writes its pinned message through debug_write, exits through thread_exit; message byte-identical to the file, exact frame teardown | **M4.3 — implemented, in-guest** |
| IPC v1: endpoint objects (sync call/reply rendezvous, two-word messages), badged merged notifications, capability transfer inside messages (copy with attenuation, first-free-slot placement), Blocked scheduler state with GS-side normalization across the switch — echo-server demo across two real processes | **M4.4 — implemented, 7/7 in-guest (ADR-0018)** |
| Spawn protocol: SYS_SPAWN from image capabilities (kernel registry, v1 = the embedded test image), explicit attenuating handle inheritance (COPY-gated, amplification refused), Process handles with READ\|DESTROY, exit-badge notifications wired into thread-exit, spawn registry as machine-state witness — supervisor restart demo across three address spaces | **M4.5 — implemented, 8/8 in-guest (ADR-0019)** |
| Console input service: COM1 RX on IRQ4/vector 33, kernel line discipline (echo, backspace, CR/LF commit, oldest-drop queue, one-reader reservation), blocking SYS_CONSOLE_READ (slot 13), SYS_PROC_LIST (14), Power-gated SYS_SHUTDOWN (15, CapObj::Power) | **M4.6 — implemented, 9/9 in-guest + interactive session test (ADR-0020)** |
| Minimal shell: the second real userspace image (`userspace/shell`, registry image 1), spawned at boot as the initial service via `spawn_init` (parentless creation, kernel-literal grants); builtins help/ps/echo/spawn/shutdown; the bootstrap thread becomes the idle thread | **M4.6 — implemented, end-to-end session proven from the harness (ADR-0020)** |
| Driver substrate: owned Untyped frame caps + destroy-returns-frame, SYS_ALLOC_FRAME (16) / SYS_MAP_MEMORY (17, kernel-chosen self-map windows joined to the region table), kernel-minted Mmio caps (uncached, NX, never consumed), IRQ relay vectors 48..63 → notification badges (live LAPIC-IPI-proven), kernel-side PCI bus-0 enumeration with BAR sizing, the virtio capability walk, and MEM\|BUS MASTER as kernel policy — the harness attaches a fresh scratch virtio-blk disk every boot | **M5.1 — implemented, 4/4 in-guest (ADR-0021)** |
| Userspace block service: `storaged` (registry image 2, spawned at boot, resident) — ring-3 virtio 1.0 handshake + split virtqueue over self-allocated owned frames, SYS_DEV_INFO (22) discovery gated on the Mmio cap, SYS_IRQ_RELAY (18) MSI-X arming through the pre-wired kernel PCI window with pid-owned relays swept at proc::destroy, zero-copy protocol over IPC v1 (lent buffer caps: SYS_CAP_COPY (21) / SYS_CAP_PHYS (19) / SYS_CAP_DESTROY (20), `Untyped{phys, owned}` structurally single-owner), poison-shutdown lifecycle; `blktest` (image 3) proves the boundary with a write→clear→read-back→verify cycle, 2 interrupt-delivered completions counted, frame-exact teardown | **M5.2 — implemented, 5/5 in-guest (ADR-0022)** |
| Filesystem service: `fsd` (registry image 4, spawned at boot, resident) — AFS1 own-design layout (checksummed superblock + ping-pong commit records + CoW object-table/bitmap runs + chained extent blocks with implicit file offsets; host mirror + mkfs in `tools/afs1.py`), transactional commits with two-generation-delayed freeing, IPC v1.1 inline 64-byte messages (names/dirents; NULL-compatible), block protocol v1.1 bounds-checked in-frame offsets, end-to-end zero-copy file I/O by forwarding clients' lent caps to storaged, FS protocol (CREATE/OPEN/READ/WRITE/CLOSE/LS/SHUTDOWN + typed FS_ERR_* statuses), 8-handle open-file table; `fstest` (image 5) proves create→write→close→re-open→read→verify→ls in ring 3 with the derived 33-delivery contract; `test_m5.py` verifies the committed on-disk bytes post-boot; shell builtins `ls`/`cat`/`write` on grant slot 3 (the shell now shares `userspace/abi.rs`) | **M5.3 — implemented, 6/6 in-guest (ADR-0023)** |
| Filesystem persistence & crash consistency (M5.4): two-boot persistence proven (written in boot N → read byte-exact in boot N+1, host-parsed after each boot); the crash-consistency gate — five SIGKILL-mid-write rounds, every reboot recovers with NO repair tool (the crashed write returns never-committed, committed-empty, or committed-full, never torn; host `afs1.audit()` fsck-lite clean after each); fstest's branch probe gives the suite two derived contracts (fresh: 34 device ops, exit 42 — persisted: 14, exit 43, zero writes); transactional UNLINK (`FS_OP_UNLINK` 8, `FS_ERR_BUSY` for open files) behind the shell's `rm`; mount-time reclamation of the superseded ping-pong generation; path→handle resolution at OPEN with handle-direct I/O (the fh is the file capability, scoped by the granted endpoint cap — ADR-0023 addendum) | **M5.4 — implemented and gated (v0.5.0)** |
| Network service (M6.1): `netd` (registry image 6, spawned at boot when the virtio-net fixture is attached) — the ring-3 virtio-net driver, LINK-LAYER ONLY (raw Ethernet frames in/out, no protocols): two split virtqueues (receiveq/transmitq) packed one frame per queue under the CAP_SLOTS budget at modern-virtio alignments, two MSI-X relay badges (bit-disjoint, one notification), VERSION_1+MAC-only feature negotiation, the config-space MAC through DEV_INFO word [6], zero-copy TX (caller's LENT frame chained behind netd's own virtio header), RX buffers posted/harvested interrupt-driven with a first-frame hold slot, RECV delivery through the reply's inline 64-byte message (lent caps cannot be mapped — the full-frame handoff is Phase 7's design); order-independent device discovery (storaged adopted the same probe loop + type assert); `nettest` (image 7) proves the wire: hand-built 42-byte ARP request → slirp reply verified field-by-field, one counted relay delivery per vector; absent fixture → honest SKIP + the network service offline (pre-v0.6.0 invocations stay green) | **M6.1 — implemented, 1/1 in-guest + the no-net SKIP boot (ADR-0024)** |
| Entropy service (M6.2): `rngd` (registry image 8, spawned at boot when a virtio-rng function exists) — the ring-3 virtio-rng driver on the SHARED virtio core (`userspace/virtio.rs`): one request queue packed into a single owned frame, `RNG_GET` filling the caller's LENT frame by device DMA (zero-copy, the write direction of storaged's read path), MSI-X completion relayed into `SYS_WAIT`, typed refusals that never leak the landed cap; `rngtest` (image 9) proves real variance — two 4 KiB draws, full-length by the device's own count, non-zero, non-constant, mutually different, two counted relay deliveries; absent fixture → honest SKIP + the entropy service offline | **M6.2 — implemented, 2/2 in-guest with all four fixture combinations green (ADR-0025)** |
| Input service (M6.3): `inputd` (registry image 10, spawned at boot when a virtio-input function exists) — the ring-3 virtio-input keyboard driver on the shared virtio core: one device-writable event queue with 32 posted 8-byte buffers (QEMU drops whole batches against a short ring), evdev events harvested to the used ring's end per interrupt (the device coalesces on `EV_SYN`), a US-ASCII keymap with modifier tracking, and a 64-byte decoded-key ring so asynchronous keystrokes survive between consumers; `SYS_CONSOLE_PUSH` (23) gated on `CapObj::ConsoleInput` feeds the kernel's line discipline so typing drives the shell while serial stays live, with a zero-length push as the capability probe that selects console vs service mode; `inputtest` (image 11) verifies harness-typed keystrokes decoded through the service boundary; absent fixture → honest SKIP + the keyboard service offline | **M6.3 — implemented, 3/3 in-guest + the live-typing boot test (ADR-0026)** |
| Console channel service (M6.4): `consoled` (registry image 12, spawned at boot when a virtio-console port exists) — the ring-3 virtio-console driver on the shared virtio core: two split virtqueues (receive stocked with 16 buffers posted AFTER DRIVER_OK, because QEMU pauses a chardev whose frontend cannot yet read), MULTIPORT declined so port 0 needs no control protocol, two MSI-X relays merged into ONE notification alongside the kernel's output-mirror wake so the driver has exactly one place to block; `SYS_CONSOLE_ATTACH` (24) + `SYS_CONSOLE_PULL` (25) gated on `CapObj::ConsoleOutput` drain a kernel-side mirror of the console's byte stream, while `SYS_CONSOLE_PUSH` (ADR-0026, unchanged) carries the inbound half — so the port is a full second console in both directions with serial still the kernel's own; `contest` (image 13) proves a round trip the host can see on its socket; absent fixture → honest SKIP + the channel service offline | **M6.4 — implemented, 4/4 in-guest + the shell driven entirely over the port (ADR-0027)** |
| Supervised restart (M6.5): `STATUS_SERVICE_GONE` (-5) answering every call a destroyed process owed (endpoints served resolved from its own `Endpoint`+READ capabilities, `Delivered` and `Waiting` slots failed, `Replied` left alone), `sched::kill_threads_of` zombieing parked threads after `ipc::release_blocked_of` drops every kernel reference to them, and `kernel/src/supervise.rs` replaying a dead service's grant list into a fresh instance behind the SAME endpoint with a restart bound and honest abandonment; `faultd`/`faulttest` (images 14/15) drive the whole cycle — live, blocked client, kill, restart, a new client reaching the new instance through its old capability — frame-exact, and the spawn-record table flat across it | **M6.5 — implemented, 6/6 in-guest (ADR-0028)** |
| Timers (M7.0): `SYS_CLOCK_NOW` (monotonic microseconds, no capability — a clock reading is not authority), `SYS_TIMER_ARM(notif_slot, badge, delay_us)` and `SYS_TIMER_CANCEL`, delivering a badge bit on a notification the caller holds WRITE on (the same gate `SYS_IRQ_RELAY` uses, so no new capability kind); relative delays, one-shot, checked on the 100 Hz tick so a deadline means NOT BEFORE with ~10 ms lag; owned by the arming process and swept at `proc::destroy`; `tick.rs` dispatches deferred tick work now that the console mirror and the timer wheel both need it; `timertest` (image 16) measures real deadlines against the clock — never early, within a tick of lag, cancellation honoured, a second cancel refused, two due timers merged into one wake | **M7.0 — implemented, 1/1 in-guest (ADR-0029)** |
| Network stack service (M7.1): `netstackd` (registry image 17) — ARP over IPv4 and its TTL cache, holding NO device capability, talking to netd as an ordinary client; `NET_OP_RECV` gains a caller-supplied deadline that NETD enforces with an M7.0 timer on its own notification (a blocked caller cannot enforce its own); cache aging by clock rather than timer; retry limited to the idempotent broadcast query; `arptest` (image 18) proves resolution on the wire, a cache hit that moves no packets, and a silent address terminating in UNREACHABLE | **M7.1 — implemented, 2/2 in-guest (ADR-0030)** |
| IPv4 + ICMP (M7.2): a receive DEMULTIPLEXER in netstackd — one place that recognises a frame and dispatches it by ethertype, with operations waiting on state it updates rather than reading the wire themselves; IPv4 with no options, fragmentation or routing, but both checksums computed on send and VERIFIED on receive (failures counted and dropped), packets accepted only if addressed to us, echo replies matched on identifier AND sequence, echo requests deliberately unanswered; round-trip time measured on the monotonic clock, and the layering visible in the failure modes (UNREACHABLE = no host, NO_REPLY = a silent one) | **M7.2 — implemented, in-guest (ADR-0031)** |
| Truncate/append-overwrite (v1 refuses to clobber), directories, per-file permissions/kernel file caps (security phase), power-loss-grade barriers (`cache=none` + virtio FUA/FLUSH — v1's proven model is process-crash prefixes), full mark-sweep fsck (interrupted generations leak conservatively), the network PROTOCOL stack (Ethernet/ARP/IP/UDP/TCP services — the link layer landed in M6.1), virtio console/input drivers, graphics, userspace programs beyond the shell and test images, driver restart supervision | not started |

The architecture above is the commitment; the roadmap is the sequence.
