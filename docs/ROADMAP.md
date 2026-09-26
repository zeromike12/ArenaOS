# ArenaOS — Roadmap

Rule of the road: **the tree is always bootable, every milestone exits only
with its automated test green, and all previous milestone tests stay green.**
Scope control: each milestone below is the *smallest useful version* of its
subsystem; anything bigger waits for its own milestone + ADR.

Status legend: ✅ done (test-verified) · 🔨 in progress · ⬜ not started

---

## Phase 0 — Architecture ✅

- [x] Vision, architecture, kernel-architecture comparison and choice
      ([VISION.md](VISION.md), [ARCHITECTURE.md](ARCHITECTURE.md))
- [x] Language decision (ADR-0001: Rust stable `no_std`)
- [x] Boot strategy (ADR-0003: own UEFI application)
- [x] Dependency policy (ADR-0004: zero runtime crates)
- [x] ABI philosophy (ADR-0006: capability invocation)
- [x] Testing strategy (ADR-0005)
- [x] Repo layout, dev environment bootstrap, conventions
- [x] Offline toolchain: Rust 1.97 + bootstrapped bare-metal sysroots
      (`x86_64-unknown-uefi`, `x86_64-unknown-none`), QEMU + EDK2, ESP tooling
      ([DEV-ENV.md](DEV-ENV.md))

## Milestone 1 — First boot ✅

Kernel image boots under QEMU/OVMF and produces *verified* diagnostics.

- [x] UEFI application entry (`efi_main`, win64/efiapi ABI), hand-written
      UEFI table bindings with static size/offset assertions
- [x] CPU state established: interrupts disabled, own GDT loaded with
      far-jump CS reload, control registers/EFER inspected and logged
- [x] 16550 serial driver (COM1, 38400 8N1, polled) + structured logging
- [x] Panic handler with location output
- [x] Real hardware self-tests: UART loopback + scratch register, long-mode
      verification (CR0/CR4/EFER bits), CPUID vendor/features, GDT
      read-back via `sgdt`, u128 arithmetic through `compiler_builtins`
- [x] UEFI memory map parsed and classified (conventional / reclaimable /
      runtime / reserved, largest free region, map key captured)
- [x] Own IDT: exception stubs with full serial diagnostics + safe halt,
      external-interrupt absorb stub (8259 **and** LAPIC EOI — see the
      ARCHITECTURE handoff findings), live-gate audit as a self-test
- [x] Real hardware interrupt test: PIT ticks absorbed through our IDT,
      full deliver → absorb → EOI → re-deliver cycle required (2 ticks)
- [x] Safe halt via UEFI `ResetSystem(EfiResetShutdown)`
- [x] Automated test: `tools/test_m1.py` — build → fresh ESP → QEMU/EDK2 →
      serial marker assertions → clean-exit check
- **Exit criteria (all machine-checked):** `m1: RESULT PASS (8/8)` on serial,
  QEMU exits cleanly (code 0) under `-no-reboot`, no `PANIC` marker.

## Milestone 2 — Kernel foundations ✅

Each step boots and adds markers (`m2:test:...`), previous tests re-run.

- [x] 2.1 **Interrupts & exceptions**: TSS with IST1 fault stack (#DF/NMI/
      #MC), exception path preserving the full interrupted context, CR2 in
      #PF diagnostics, controlled fault-injection protocol, deliberate-fault
      tests (divide-by-zero and bad-address #PF delivered, diagnosed, and
      *recovered*). Harness: `tools/test_m2.py`; markers `tss_installed`,
      `exc_de_recovered`, `exc_pf_recovered`.
- [x] 2.1b regression note: recovery taught two hardware lessons, both now
      encoded as tests/comments — LTR requires the TSS descriptor type
      *available* (0x9; LTR itself sets busy), and an exception handler that
      can return must treat every caller-saved register as live.
- [x] 2.2 **Timers**: PIT driver (one-shot/periodic/latch/read-back —
      modes chosen from measured QEMU behavior, see `drivers/pit.rs`);
      TSC calibrated against the PIT oscillator in two agreeing windows
      (boot halts on implausible/disagreeing calibration); monotonic
      `now_us()` clock; kernel tick left running at 100 Hz mode 3; markers
      `pit_oneshot`, `tsc_frequency`, `clock_monotonic`, `tick_rate`.
- [x] 2.3 **Physical memory**: flat-bitmap frame allocator over the
      captured conventional regions (ADR-0007, decided with the in-guest
      benchmark the test logs: ~100 ns/op hot path); runtime/reserved
      regions structurally unallocatable; stress test with integrity checks
      (uniqueness, in-region, RAM round-trip, exact accounting, double-free
      rejection, contiguous runs); marker `frame_allocator`.
- [x] 2.4 **Virtual memory**: own PML4, dual view (ADR-0008) — identity for
      firmware compat (RW+X carve-out, torn down at EBS/M2.7) + higher-half
      direct map (RW+NX); image window per PE section (.text R+X, .rdata
      RO) via Loaded Image Protocol; CR0.WP + EFER.NXE enforced and
      *tested*: ring-0 write to RO .text → #PF ec=0x3, NX data fetch →
      #PF ec=0x11, higher-half call executes (markers `vm_address_space`,
      `vm_write_protect`, `vm_nx`). UEFI gotcha paid for in blood:
      HandleProtocol is slot 16 (CloseEvent/CheckEvent are slots 11/12).

- [x] 2.5 **Kernel heap**: typed allocator over frames; debug poisoning/redzones;
      host-side unit tests for the allocator logic + in-guest stress test.
      DONE: arena-heap core crate (host suite 10/10 incl. model-checked
      stress) + boot glue (ADR-0009); guards always on; m2 14/14.
- [x] 2.6 **Synchronization**: spinlocks (ticket or queued, decided by ADR when
      SMP lands — single-core correctness first), critical-section helpers,
      lock debugging (owner tracking in debug builds).
      DONE: arena-sync crate (host suite 6/6 incl. 4-thread
      contention + recursion #[should_panic]), without_interrupts
      irqsave/irqrestore, heap lock-wrapped, crit_section proves zero
      interrupts cross a 25ms section on real PIT hardware (ADR-0010);
      m2 16/16.
- [x] 2.7 **Boot split**: boot stage → `ExitBootServices()` → kernel proper
      entry with boot-info record; boot stage and kernel become separate
      crates; reboot-stability test loop (100 clean boots).
      DONE (ADR-0011): BootInfo ABI record + kernel-view trampoline;
      .reloc replay (101 DIR64) at +KERNEL_OFFSET under cli; farewell
      island for post-EBS ResetSystem (fw CR3 + identity stack); timer
      chain reclaimed — the PIT lives on IOAPIC **pin 2** (QEMU's
      ISA-IRQ0→GSI-2 override), firmware's masked RTE2 re-routed to
      vector 32, kernel_irq_live proves 2 real ticks through the
      relocated IDT + kernel-alias LAPIC EOI; m2 21/21; stability:
      100/100 clean boots (tools/stability_loop.sh); release artifacts
      (tools/release.sh, docs/RUNNING.md) shipped as GitHub release.

## Milestone 3 — Multitasking 🔨

- [x] 3.1 **Kernel threads + context switch** (assembly fast path; full state:
      GPRs, FPU/SSE state lazily or XSAVE — ADR at the time).
      DONE (ADR-0012): Win64 callee-saved + RFLAGS + RSP switch frame; no
      FPU/SSE state — the image is audited SSE/MMX-free at *every build*
      (build.sh fails otherwise); 32 KiB frame-allocated stacks with
      bottom canaries (checked at every switch-away + reap); stable-slot
      thread table, RR ready ring, deferred zombie reaping with exact
      frame/heap accounting; bootstrap thread = kmain. Tests: spawn/run/
      reap, exact RR interleave (1,2,3 × 4), callee-saved register
      round-trip through a live switch (asm probe), two-thread stack
      isolation with disjoint ranges + depth-200 recursion, 127-thread
      churn with exact accounting + MAX_THREADS refusal. m3 5/5; Rust
      trap documented in CODING-CONVENTIONS (expect-assign on Copy
      places is a silent no-op — the scheduler's first bug).
- [x] 3.2 **Preemptive scheduler**: timer-driven, per-CPU-ready run queues (SMP
      *structures*, single-core execution), deterministic RR test mode.
      DONE (ADR-0013): vector-32 PIT tick gets a full-save stub whose Rust
      handler EOIs, counts, and calls a scheduler hook; the hook counts the
      quantum down and — when it expires with another thread ready — runs the
      *same* decision + assembly switch as `yield_now`, nested inside the
      interrupt (resume returns up the IRQ chain and `iretq`s into the
      interrupted body). Per-CPU `CpuSched` structures (`MAX_CPUS=4`,
      `this_cpu()`=0 until SMP). Soundness: every scheduler mutation runs
      IF=0, so a tick never observes a half-finished decision. New threads'
      first frame now carries IF=1 (amends ADR-0012) — an IF=0 first-run
      thread would be un-preemptible (hang observed + fixed). Tests:
      three threads containing NO yield call rotated in *exact* RR order
      (≥12 timer switches, entry-by-entry) with ~200k loop iterations each;
      cooperative yields and 10 ms quanta compose across 15 ms busy-waits
      (mid-wait rotation). m3 7/7; mtest.py now fails a milestone run when
      any prior-milestone RESULT in the same boot is not PASS.
- [x] 3.3 **Processes** = address-space objects + kernel/user privilege
      separation machinery (the capability-space half is tracked under
      3.4). DONE (ADR-0014 + its M3.3b addendum). **3.3a — machinery:**
      hand-encoded payloads run at CPL 3 (kernel-view CR3);
      `syscall`/`sysret` entry (STAR/LSTAR/SFMASK, ring-3 segments), TSS
      RSP0 + per-CPU entry stacks, SYS_WRITE/SYS_EXIT dispatch, SMAP
      enforced (bare kernel touch of a user page #PFs; STAC-bracketed
      reads work), enter_user via iretq; a ring-3 spin survives ticks and
      mid-user preemption. **3.3b — address-space objects:** `proc.rs`
      (32 slots, pid 1+) — a private PML4 per process, user half empty,
      kernel half cloned entry-wise from the kernel view (lower tables
      shared); `spawn_with_cr3` binds threads to process roots;
      `plan_switch` installs the incoming CR3 together with RSP0 and the
      syscall scratch so syscalls/IRQs never switch CR3; destroy frees
      the user half + root with exact accounting. The first timer tick
      under a process CR3 exposed a real design bug: the kernel view's
      aliases for above-2 GiB MMIO were computed `phys + KERNEL_OFFSET`,
      which WRAPS INTO THE USER HALF (LAPIC → 0x7EE0_0000) — invisible
      while the kernel view was the only address space, fatal under any
      clone. Fixed with `paging::mmio_alias_va` (kernel-half alias rule
      for below-4 GiB MMIO), a build-time direct-map collision check, and
      a test assertion that the EOI slot reads identically — same value,
      same physical chain — through both CR3s. Tests: two processes
      isolate the same VA (distinct frames, cross-invisible writes,
      persistence), kernel-half .data identical under both CR3s,
      unmapped-VA #PF recovery, teardown frees exact; 8x
      create/map/destroy churn; table-full refusal at 32; a full ring-3
      round inside a process (SYS_WRITE byte-exact, SYS_EXIT(42), RSP0
      evidence, CR3 restored, frame accounting exact). m3 11/11 +
      100/100-boot stability.
- [x] 3.4 **Capability spaces** (minimal): slots, rights, copy/move/destroy —
      kernel-internal use only at first.
      DONE (ADR-0015): `cap.rs` — a 16-slot table per process, embedded in
      `Process` (born with `create`, dies with `destroy`). A cap is
      `(object, rights)` over the two real object kinds that exist today:
      `Process{pid}` and untyped-style `Memory{phys, pages}`. Rights:
      READ / WRITE / COPY / DESTROY. `grant` is the only creation path
      (the boot/root-task trust primitive); `copy`/`move` attenuate only
      — amplification is a loud error and the source must hold COPY;
      `destroy` removes the *reference*, never the object, so caps can
      dangle and every invoke re-validates liveness. Rights gate real
      actions: `process_root` (READ — a target's root PHYS is
      information) and `map_memory` (WRITE on the memory cap *and* on a
      live process cap — the untyped → address-space binding, W^X per
      ADR-0008). Tests: the mechanics round (ordered grants, attenuated
      copy, amplification/COPY-less/occupied-slot refusals, move clears
      the source, right-gated + double-destroy refusals, cross-space
      isolation, capacity refusal at 16, dangling reference survives its
      target) and the invoke round (READ gate returns the live root;
      three map refusals measured; a 0xC3 pattern in the untyped frame
      read back under the target's CR3 through the cap-mapped page;
      exact frame accounting throughout). m3 13/13 + 100/100-boot
      stability. **Milestone 3 complete** — threads, preemption,
      processes, ring 3, capability spaces: the object model M4 puts
      userspace on top of.

## Milestone 4 — Userspace & first program 🎯 (assignment's "first userspace program")

- [x] 4.1 **Executable format decision + loader** (ADR due: ELF container with
      our own semantics vs. bespoke format — container pragmatism, semantics
      ours).
      DONE (ADR-0016): ELF64 container, ArenaOS strict-subset semantics —
      `kernel/kernel/src/elf.rs` validates (ET_EXEC static only, ≤8
      page-aligned `PT_LOAD`s in the canonical lower half, W^X per segment,
      filesz ≤ memsz inside the file, non-overlapping, entry inside an X
      segment, `PT_INTERP` an explicit refusal; all other phdr types inert,
      section headers never consulted) and loads (per page: occupied-VA
      refusal *before* allocation, fresh zeroed frame, file-backed prefix
      copied, mapped with the segment's own flags — W^X re-enforced by the
      mapper; frames belong to the address space, `proc::destroy` reclaims).
      Test image is a genuine cargo/rust-lld artifact — `userspace/payload`
      (`x86_64-unknown-none`, own `payload.ld`, fixed layout: entry stub
      0x200000, self-describing META 0x201000, 4 KiB NOLOAD bss 0x202000),
      embedded via `include_bytes!` (build.sh compiles it first). m4 suite:
      parse + META cross-check, 25-mutation rejection corpus, load round
      (7 frames exact, PTE W^X flags, contents under the target's CR3
      through STAC, double-load/dead-target refusals, exact teardown) —
      m4 3/3, m1/m2/m3 regressions green.
- 4.2 **Syscall ABI v1** (register encoding ADR), dispatch, typed status
      codes; first calls: debug-write (temporary console backdoor),
      thread_exit.
- 4.3 **First user process**: statically linked ring-3 binary executes,
      writes via syscall, exits. Test proves ring transition (RIP/CS checks
      in both directions, SMAP faults if kernel touches user memory wrong).
- 4.4 **IPC v1**: endpoints, sync call/reply, notifications; capability
      transfer in messages; echo-server demo (two processes).
- 4.5 **Root task + spawn protocol**: process creation from image
      capabilities with explicit handle inheritance; supervisor restart demo
      (kill a service, watch it restart) — the recovery story proven early.
- 4.6 **Minimal shell**: line input from console service, spawn builtins +
      images, `help/ps/echo/shutdown`. First "real" userspace surface.

## Phase 5 — Storage (outline)

Block-device abstraction over VirtIO-blk (userspace driver, kernel
notification caps) → extent/transactional filesystem server (own design,
ADR) → namespace/mount service → persistent root FS with crash-consistency
tests (kill VM mid-write, verify recovery).

## Phase 6 — Drivers (outline)

VirtIO family completion (net, console, rng, later gpu) → input (keyboard via
VirtIO-console/PS2 fallback decision by ADR) → driver framework hardening:
restart under fault injection, capability re-grant tests.

## Phase 7 — Networking (outline)

NIC TX/RX via virtio-net driver server → Ethernet framing → ARP → IPv4 →
ICMP → UDP → DNS resolver service → TCP (own stack server, async API) →
userspace net API library. Each protocol a separately testable milestone with
QEMU netdev (user-mode/slirp + tap tests, packet capture assertions).

## Phase 8 — Mature userspace (outline)

Service manager + declarative service manifests → transactional configuration
store → permission manifests & grant UI (CLI first) → standard libraries →
package format & signed packages (ADR) → installer/updater.

## Phase 9 — Graphics (outline)

GOP/virtio-gpu display server → compositor → input routing → font rendering
(own rasterizer or audited import — ADR) → toolkit.

## Phase 10 — Desktop (outline)

Shell, launcher, terminal, settings, file manager, editor, system monitor —
each an ordinary ArenaOS application, no privileged status.

---

## Explicitly NOT building yet (scope firewall)

These are *forbidden* until their phase gate; if a PR/commit touches them
early, it's a scope violation:

- ❌ Any graphics/compositor/window code (before Phase 9 gate: storage,
  drivers, IPC, and userspace services stable)
- ❌ Networking of any kind (before Phase 7; and no protocol beyond the one
  being milestone-tested)
- ❌ Filesystem/disk code (before Phase 5)
- ❌ POSIX compatibility layer, `fork`, signals, errno — ever, unless a
  superseding ADR appears
- ❌ SMP execution (structures are SMP-ready from M3, but second cores stay
  parked until a dedicated SMP milestone after M4)
- ❌ Dynamic linking, shared libraries (static everything until Phase 8
  package design says otherwise)
- ❌ ARM64 code paths (interfaces stay arch-neutral; no second arch)
- ❌ Real-hardware drivers (VirtIO/QEMU only until Phase 8 hardening)
- ❌ Audio, USB, Bluetooth, printers, Wi-Fi
- ❌ Package manager, installer, updater (Phase 8)
- ❌ Kernel heap before M2.5; paging before M2.4; no "temporary" versions of
  them either
- ❌ Secure boot / signing infrastructure (design is reserved — ADR-0003 —
  implementation is Phase 8)
- ❌ Any third-party crate in the OS image (ADR-0004, permanent)

## Milestone mechanics

- One milestone = one or more commits, each leaving `tools/run_tests.sh`
  green.
- New subsystem ⇒ new `m<N>:test:*` markers + new `tools/test_m<N>.py`;
  old test scripts are never deleted.
- Significant decisions inside a milestone ⇒ ADR before merging the code
  that depends on them.
- A milestone that cannot be demonstrated by a test does not count as done,
  no matter how good the code looks.
