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

## Milestone 3 — Multitasking ✅

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

## Milestone 4 — Userspace & first program ✅ (assignment's "first userspace program")

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
- [x] 4.2 **Syscall ABI v1** (register encoding ADR), dispatch, typed status
      codes; first calls: debug-write (temporary console backdoor),
      thread_exit.
      DONE (ADR-0017): the v0 boundary formalized and frozen — RAX = call
      number; six arguments in RDI/RSI/RDX/R10/R8/R9 (stub marshals all
      six plus the recorded frame through Win64 to the dispatcher); typed
      i64 status out: 0 = OK, positive = call payload, negative = dense
      error codes (`STATUS_BAD_CALL` -1 — the v0-compatible wire value —
      `STATUS_BAD_ARG` -2, `STATUS_BAD_ADDRESS` -3); RBX/RBP/R12–R15
      preserved, user RFLAGS restored, kernel side IF=0. Registry:
      1 `debug_write` (validated SMAP-aware copy-out, temporary console
      backdoor), 2 `thread_exit` (full-width code through the scheduler
      reap), 3 unallocated forever, 4/5 the M3 ring-3 proof calls, 6
      `abi_echo6` (six-register marshalling probe). Renames only for m3
      (`SYS_WRITE`→`SYS_DEBUG_WRITE`, `SYS_EXIT`→`SYS_THREAD_EXIT`) —
      wire-identical, m3's ring-3 assertions untouched. m4 suite: the
      ring-3 payload proves the echo fingerprint across all six argument
      registers, six callee-saved canaries, the success count, all three
      typed refusals *observed in ring 3*, unknown nr → -1, and
      thread_exit's 64-bit code fidelity; bring-up caught three live
      bugs (Win64 stack-arg alignment, REX.B-vs-REX.R cmp encodings,
      rel8 overflow) — each now a comment where it was fixed. m4 5/5,
      m1/m2/m3 regressions green.
- [x] 4.3 **First user process**: statically linked ring-3 binary executes,
      writes via syscall, exits. Test proves ring transition (RIP/CS checks
      in both directions, SMAP faults if kernel touches user memory wrong).
      DONE: the M4.1 loader, M4.2 ABI, and M3.3a per-thread-CR3 scheduler
      (`spawn_with_cr3`) compose into the full lifecycle — `proc::create`
      + `elf::load` + one stack leaf (8 frames total) + `enter_user` at
      the image's `e_entry`. The payload became a real program: it
      verifies META's magic and the zero-filled NOLOAD bss FROM ring 3,
      stamps bss slot 0, writes its linker-pinned message through
      `debug_write`, and exits with META.exit_ok — diagnostic codes
      43/44/45/99 name any failed check. The payload grew a
      self-describing META (msg_va/msg_len/exit_ok): the kernel derives
      every expectation from the image FILE — captured bytes compared
      against the pinned message in the file, stamp derived from
      META_MAGIC, success code from META. Bring-up caught two live
      bugs: the target's default PIC codegen grew a .got at 0x203000
      (fixed with relocation-model=static) and (lo,hi) user regions
      were written as (base,len) — the typed -3 the payload reported
      through its own exit code located it immediately. m4 6/6,
      m1/m2/m3 regressions green.
- [x] 4.4 **IPC v1**: endpoints, sync call/reply, notifications; capability
      transfer in messages; echo-server demo (two processes).
      DONE (ADR-0018): endpoint + notification kernel objects reached
      only through caps (WRITE = call/notify side, READ = serve/wait
      side); five frozen registry slots (7 ipc_call, 8 ipc_recv,
      9 ipc_reply, 10 notify, 11 wait); two-word inline messages with
      at most one transferred cap per message (copy-only under the
      ADR-0015 attenuation rule, first-free-slot placement, landing
      index reported in the message); STATUS_BUSY (-4) for bounded
      refusals (queue depth 4, one server per endpoint, one waiter per
      notification). The scheduler grew State::Blocked +
      block_current/wake + spawn_in_proc (threads now know their
      process — the dispatcher resolves cap spaces through it, no
      global names). The echo demo: two real address spaces, the server
      parks in recv first, the client's call delivers both words + a
      Memory cap and blocks, the reply echoes (verified IN RING 3), the
      badge rides notify → wait's immediate path; cap landed
      rights-intact, sender kept its copy, counters exact, teardown
      frame-exact. Bring-up caught one deep bug: the per-CPU GS pair is
      flipped by the syscall stub and NOT saved by switch_context — a
      thread blocking inside the dispatcher resumed on the wrong side
      after another thread's exit-swapgs and triple-faulted silently
      (the GS-side capture/normalize/restore now lives in
      block_current, with the whole cascade documented there). m4 7/7,
      m1/m2/m3 regressions green.
- [x] 4.5 **Root task + spawn protocol**: process creation from image
      capabilities with explicit handle inheritance; supervisor restart demo
      (kill a service, watch it restart) — the recovery story proven early.
      DONE (ADR-0019): SYS_SPAWN (frozen registry slot 12) — the parent
      presents an Image cap (READ; kernel registry, v1 = the embedded
      rust-lld test image), an inheritance spec read under STAC from the
      parent's own memory (≤4 (slot, rights) pairs; source must hold
      COPY; any amplification is a typed refusal, never a clamp), an
      optional notification cap (WRITE) + nonzero badge registered as
      the child's exit notification (fires when the child's last live
      thread exits — hooked into thread-exit before the diverging
      swapgs), and receives the child's pid as a positive status; the
      parent's Process handle (READ|DESTROY — DESTROY powers rollback
      and future user-driven reaping) lands at its first free slot. The
      child's first thread reads its start facts (image entry, the
      stack page derived from the image's top segment VA, page-granular
      user regions) from its spawn record — the same records the suite
      reads as machine-state witnesses. Every partial failure rolls
      back to exactly zero frames and zero objects. The restart demo:
      a ring-3 supervisor spawns the untouched M4.3 image twice, each
      child inheriting one attenuated Memory cap at its slot 0; the
      child's console message appears twice (the restart, visible on
      the wire), both lives badge the supervisor at exit, both children
      exit with the image's own success code, handles land in spawn
      order, the supervisor keeps its originals (copy, not move),
      counters exact (9 dispatches, 2 notifies, 2 parked waits),
      teardown frame-exact across three address spaces. m4 8/8,
      m1/m2/m3 regressions green.
- [x] 4.6 **Minimal shell**: line input from console service, spawn builtins +
      images, `help/ps/echo/shutdown`. First "real" userspace surface.
      DONE (ADR-0020): COM1 RX interrupt-driven through IOAPIC pin 4 →
      vector 33 (the timer stub's frame discipline, EOI-before-work);
      a kernel line discipline (echo, backspace, CR/LF commit, 4-line
      queue that drops oldest, one-reader reservation) behind three
      frozen registry slots — 13 SYS_CONSOLE_READ (blocking), 14
      SYS_PROC_LIST ((pid, threads) pairs), 15 SYS_SHUTDOWN (gated on a
      new CapObj::Power: the authority to halt the machine is an object
      a process HOLDS, never a public verb). The shell is the second
      real userspace image (userspace/shell: cargo/rust-lld ET_EXEC at
      0x400000/0x410000, embedded as spawn-registry image 1), spawned
      at boot by spawn_init — the M4.5 creation sequence without a
      parent — with Power/Image0/Notification grants; the bootstrap
      thread becomes the idle thread (yield + sti;hlt). Builtins: help,
      ps, echo, spawn (SYS_SPAWN image 0 with an empty inheritance spec
      + exit-badge wait — the payload's message lands mid-session),
      shutdown. Bring-up caught one deep bug: the farewell island is
      identity-mapped only in the kernel view, so reset_shutdown called
      under a PROCESS CR3 (the shell's syscall, or any panic on a
      process thread) fetch-faulted at the island's phys — the island
      path now normalizes to the kernel view first. Harness: serial
      runs through a stdio chardev and every boot is marker-paced-fed
      `shutdown` at the shell's first prompt, so all seven test scripts
      and the stability loop prove the full console chain (UART RX IRQ
      → line discipline → blocking read → shell → Power-gated shutdown
      → ResetSystem) on every run; test_m4_shell.py drives the whole
      interactive session (26 checks). m4 9/9, m1/m2/m3 regressions
      green.

## Milestone 5 — Storage 🔨

The phase-5 outline, decomposed (per ARCHITECTURE §10: filesystems are
userspace servers on top of block-device capabilities; the kernel stores
no file semantics; the scratch VirtIO-blk disk is the milestone's
medium — the ESP stays the boot medium throughout).

- [x] 5.1 **Driver substrate** (DONE — m5 suite 4/4 in-guest: pci_scan,
      untyped_alloc, mmio_user, irq_relay; m1–m4 green on the same boot;
      the scratch-disk fixture attached in every harness path — mtest,
      run.sh, stability loop, release verification): the kernel primitives userspace drivers
      run on (ADR-0021) — Untyped frame caps (`SYS_ALLOC_FRAME`; destroy
      RETURNS the frame — the first owned cap kind), self-service
      mapping (`SYS_MAP_MEMORY`: Untyped/Mmio cap → the caller's own
      user half at a kernel-chosen window VA, appended to its region
      table), Mmio caps + ring-3 MMIO proof (the HPET main counter,
      read-only, observed ticking from a user process), IRQ relay
      vectors (MSI-range IDT stubs → registered notification + badge),
      and kernel-side PCI enumeration with the virtio-pci capability
      walk (config space, bus-master/DMA authorization, and interrupt
      vector allocation stay kernel POLICY; the virtio protocol itself
      is userspace MECHANISM). Harness grows a scratch `virtio-blk-pci`
      disk. Exit: m5 suite green in-guest, every prior suite unaffected.
- [x] 5.2 **Userspace VirtIO-blk driver**: `userspace/storaged` — the
      third real userspace crate (registry image 2, spawned at boot
      with kernel-literal grants: an `Mmio` cap over the virtio
      structure BAR, the endpoint's serve side, an interrupt
      notification — device discovery through `SYS_DEV_INFO`, gated on
      the Mmio cap): modern virtio 1.0 handshake, one split virtqueue
      over three self-allocated Untyped frames, MSI-X completions
      armed by `SYS_IRQ_RELAY` (kernel programs the table + enable bit
      through the pre-wired kernel PCI window; relays are pid-owned
      and swept at `proc::destroy`) and received through `SYS_WAIT` —
      never polled. Synchronous sector read/write on the scratch disk
      over a zero-copy block protocol (IPC v1: `[sector, op]` words +
      the caller's LENT buffer cap; the device DMAs the caller's own
      frame — phys via `SYS_CAP_PHYS`, ownership refined to
      `Untyped{phys, owned}` so copy/IPC-landing can never duplicate
      an owner). Exit proven by m5 `block_service` (5/5): `blktest`
      (image 3) drove a write→clear→read-back→verify cycle THROUGH
      the service boundary from another process — all 512 bytes
      verified, exactly 2 interrupt-delivered completions counted on
      the relay vector, both exit badges/codes exact, dead driver's
      relay swept, teardown frame-exact; markers on the wire; the
      PRODUCTION storaged instance spawns after the suite and parks
      resident (`ps` shows it beside the shell). ADR-0022.
- [x] 5.3 **Filesystem v1 — own design**: AFS1 (ADR-0023) —
      extent-based data written in place; copy-on-write transactional
      metadata: the object table (32 × 64 B records) and allocation
      bitmap are CoW'd to fresh contiguous runs per commit, and a
      ping-pong commit record (sectors 1/2, slot = seq % 2, FNV-1a
      checksummed) flips generations in ONE sector write;
      two-generation-delayed freeing makes every crash a leak, never
      corruption. Host-side `mkfs` (`tools/afs1.py` — the layout's
      single source of truth, mirrored byte-for-byte by fsd; the
      harness formats the scratch disk every run). IPC v1.1: CALL/
      RECV/REPLY gained an OPTIONAL 64-byte inline message buffer
      (NULL = v1.0 behavior, wire-compatible) carrying names and
      dirents; block protocol v1.1: `w1 = op | offset << 8` with a
      bounds-checked in-frame buffer offset. `userspace/fsd`
      (registry image 4, spawned at boot between storaged and the
      shell; grants: block endpoint WRITE + FS endpoint READ — no
      Mmio, no IRQ: fsd never sees the device): superblock + newest-
      commit mount, RAM metadata, per-write transactions, 8-handle
      open-file table, typed FS_ERR_* statuses, poison-shutdown
      lifecycle — and the end-to-end zero-copy chain: clients LEND
      their buffer frame, fsd FORWARDS the untouched cap to storaged,
      the device DMAs between disk and the CLIENT's page (a lent cap
      cannot be mapped — the cap system enforces the zero copy).
      Exit: m5 `fs_service` (6/6) — `fstest` (image 5) ran
      create→write→close→RE-OPEN→read→byte-for-byte verify→ls-walk
      entirely in ring 3; three exact exit badges/codes; relay
      deliveries EXACTLY 33 = the derived disk-op contract (fsd's own
      count and storaged's completions agree — three counters, one
      number); dead driver's relay swept; frame-exact teardown across
      all three children. AFTER the boot, `test_m5.py` parses the
      committed image: newest commit seq 3, `arena.txt` size 512,
      extents resolved to bitmap-marked sectors, pattern bytes
      verified ON DISK. The production fsd mounts the suite-committed
      volume in the same boot — the in-boot persistence-across-re-open
      proof — and the shell (migrated onto the shared
      `userspace/abi.rs`) serves `ls` / `cat NAME` / `write NAME TXT`
      through its new slot-3 endpoint cap; the shell session test
      drives all three (write creates only — v1 has no truncate;
      honest refusal over silent clobber).
- [x] 5.4 **Persistence + namespace + crash consistency** — DONE
      (ADR-0023 addendum). Two-boot persistence:
      `tools/test_m5_persist.py` boots the SAME `scratch.img` twice —
      boot N's shell writes `persist.txt`, boot N+1's shell `cat`s it
      back byte-exact, `rm`s it transactionally across the reboot, and
      the host parses the committed sectors after each boot. The suite
      itself became a persistence witness: fstest now PROBES the
      volume first — a fresh disk runs the create contract (34 device
      ops, exit 42); a disk that survived a reboot runs the persisted
      contract, verifying the committed file with ZERO writes (14
      device ops, exit 43) — the exit code selects the relay-delivery
      contract the kernel asserts. Crash-consistency gate:
      `tools/test_m5_crash.py` SIGKILLs QEMU at five observed points
      of an in-flight shell write (mid-CREATE-commit through
      just-after-the-final-commit); every reboot recovers with NO
      repair tool — fsd simply mounts, the suite verifies the
      pre-crash file byte-for-byte, the crashed write returns
      never-committed, committed-empty, or committed-full (never
      torn), and the host-side `afs1.audit()` (an fsck-lite over the
      newest committed generation) finds zero problems. Namespace:
      OPEN exchanges a name for a handle exactly once; all I/O is
      handle-direct (paths are UI, per ARCHITECTURE §10) — the fh is
      the file capability, scoped by the client's granted fsd endpoint
      cap (kernel file objects rejected as v1 scope; per-file
      permissions belong to the security phase — ADR-0023 addendum).
      FS_OP_UNLINK (8) completes the v1 namespace behind the shell's
      `rm` (two-generation-delayed extent freeing; honest FS_ERR_BUSY
      for open files), and fsd's mount reclaims the superseded
      ping-pong generation so multi-boot volumes stop bleeding
      metadata sectors. The v0.5.0 run bundle ships
      `scratch-template.img` — the FORMATTED volume — and the release
      verification boot runs on the shipped template itself.

## Phase 6 — Drivers

VirtIO family completion (net, console, rng, later gpu) → input (keyboard via
VirtIO-input/PS2 fallback decision by ADR) → driver framework hardening:
restart under fault injection, capability re-grant tests.

- [x] 6.1 **virtio-net: `netd` + the ARP link proof** — DONE
      (ADR-0024). The link-only driver server (registry image 6):
      two split virtqueues packed one frame per queue (the
      CAP_SLOTS=16 budget), two MSI-X relay badges into one
      notification, zero-copy TX chaining the caller's LENT frame
      behind netd's own virtio header, the config-space MAC read
      through DEV_INFO word [6], and order-independent device
      discovery (storaged adopted the same probe loop + type assert).
      Proof: the m6 suite spawns netd + nettest (image 7); nettest
      hand-builds the 42-byte ARP request for slirp's 10.0.2.2,
      sends it, and verifies the reply's ethertype/opcode/sender-IP/
      target-MAC/sender-MAC-consistency at their exact wire offsets;
      the kernel witnesses exactly ONE relay delivery per vector
      (48 RX + 49 TX — no polling), both exit badges, both exit 42s,
      and frame-exact teardown. Absent device → honest SKIP:
      `tools/test_m6.py` boots BOTH ways — with the fixture (m6
      RESULT PASS 1/1 + the production netd spawned) and without it
      (the SKIP markers + "network service stays offline" + m1–m5 all
      green in the same boot). Every harness boot path attaches
      `-netdev user,id=net0 -device virtio-net-pci,netdev=net0`
      (arena_env.net_args). Gates: run_tests 11/11 scripts, the
      persistence + crash gates green with the NIC attached, fmt
      clean + clippy clean on the new code, and the stability loop
      100/100 with the full fixture family — whose first with-NIC run
      earned its keep by catching a 1-in-100 host-coupled flake: the
      device-bound suite drains were bounded by a *yield count*,
      which under host load burns out before QEMU's iothread delivers
      the awaited MSI, so all three (m5 block/fs, m6 net) are now
      bounded by an HPET wall-clock deadline (~21 s,
      DRAIN_DEADLINE_TICKS); the fixed kernel re-ran 100/100 clean.
- [x] 6.2 **virtio-rng: `rngd` + the shared-library extraction** —
      DONE (ADR-0025). ADR-0024's third-driver rule executed in two
      steps, each gated: (a) the extraction — `userspace/virtio.rs`
      now owns discovery, the window map, the §3.1 handshake, the
      frame budget loop, `ring_offsets`/`Queue`(publish/used_idx/
      used_entry)/`queue_setup`/`driver_ok`/`w64`/`desc_write`, with
      typed stage errors (`VErr`) each driver maps to ITS exit codes
      and a `RingMem` seam recording the one genuine difference
      (netd/rngd pack a queue into one frame; storaged's 256-entry
      rings take a frame each). storaged −268 lines, netd −268, zero
      behavior change: 11/11 scripts green before rngd existed.
      (b) `rngd` (registry image 8): one queue, `RNG_GET` pointing a
      device-writable descriptor at the caller's LENT frame so the
      device DMAs entropy into the client's own page — zero-copy
      fill, interrupt-completed. Proof: `rngtest` (image 9) draws two
      4 KiB frames and asserts real variance — full length (the
      device's own count), not all-zero, not one repeated byte, and
      the two draws different; the kernel witnesses exactly two relay
      deliveries (one MSI per draw), exact badges, both exits 42, and
      frame-exact teardown. Entropy QUALITY is the host backend's
      business and is never claimed by the suite. Fixture: bare
      `-device virtio-rng-pci` (QEMU's `rng-builtin` default backend
      — no host files, no privileges); absent → honest SKIP, and all
      four fixture combinations (both / net-only / rng-only /
      neither) boot green. `MAX_IMAGES` and `MAX_SPAWN_RECS` raised
      8 → 12, the consequence ADR-0024 named. Two bugs the FIRST BOOT
      caught, not the reading: QEMU presents the transitional entropy
      ID 0x1005 (the legacy IDs are a hand-assigned list, not
      0x1000+type), and the client's first cut self-mapped its frame
      before copying the lend — the map CONSUMES the cap, so the copy
      must come first. Gates: run_tests 11/11, clippy + fmt clean on
      all three drivers, entropy verified different across boots, and
      the stability loop 100/100 with the full fixture family.
- [x] 6.3 **virtio-input keyboard: `inputd` + live typing** — DONE
      (ADR-0026). Transport: virtio-input-hid
      (`-device virtio-keyboard-pci`), modern-only — QEMU forces
      virtio 1.0 on the class, so the only id is 0x1052 (0x1040 + 18)
      with no transitional alias. i8042 PS/2 stays the DOCUMENTED
      fallback and was not needed: it would have cost a new I/O-port
      capability kind for ring 3, an 8042 mode-byte state machine, and
      nothing reusable (real hardware is USB HID). `inputd` (registry
      image 10) is the fourth driver on the shared virtio core, and
      the first with a device-writable queue that must stay stocked:
      32 event buffers posted from the same frame as the rings,
      because QEMU's virtio-input drops an ENTIRE event batch it
      cannot place, flushes only on `EV_SYN`, and raises one interrupt
      per batch — so every wake drains the used ring to its end.
      **The roadmap's own plan was rejected here and the ADR records
      why:** switching the shell's read path off serial would have
      broken every automated boot and every headless user. Instead a
      new `SYS_CONSOLE_PUSH` (23), gated on a new `CapObj::ConsoleInput`
      singleton (the `Power` pattern, second use), lets inputd feed
      DECODED bytes into the kernel's existing line discipline — the
      identical entry point the COM1 RX ISR uses. One line editor, one
      blocking read contract, two hardware sources; the shell needed
      ZERO changes. A zero-length push is the documented capability
      PROBE, so one image discovers whether it is the production
      console feeder or a suite's IPC service — the m6 instance is
      deliberately denied the cap, making the mode switch part of the
      test. Proofs, both real: `m6:test:input_service` (the harness
      types `arena` on the virtual keyboard over QMP; inputtest reads
      the decoded bytes back through the service and verifies them
      byte-for-byte, with counted interrupt batches, exact badges,
      both exits 42, relay swept, frame-exact teardown) and
      `tools/test_m6_typing.py` — the PRODUCTION path with the serial
      input channel dead: `echo Hello-From-The-Keyboard` (capitals
      prove modifier tracking), `psX<backspace>` (the line discipline
      erases the typo before the shell sees it), and `shutdown` — the
      machine halts because someone typed it. The harness gained a
      second channel for this (`tools/qmp.py`, test-only), and every
      boot in the gate now types on the keyboard as well as the
      serial port. Registry images 10 + 11 fill `MAX_IMAGES`
      exactly — 6.4 must raise it. Gates: run_tests 12/12 (311
      assertions), fmt + clippy clean on the new code, and the
      stability loop 100/100 with the full fixture family.
- [x] 6.4 **virtio-console: `consoled`** — DONE (ADR-0027). The
      registry bound was raised first (12 → 16), as ADR-0026 required.
      **The roadmap's plan was decided against, and the ADR says why:**
      moving the shell's stdout/stdin onto the driver would duplicate
      the line discipline, break every headless boot, and leave "which
      console is real?" unanswered. A port is not a console — the
      kernel keeps ONE console and drivers attach CHANNELS to it.
      Inbound reuses ADR-0026's `ConsoleInput` gate unchanged (the
      keyboard's mechanism paying for itself immediately); outbound is
      the one new kernel mechanism: a `ConsoleOutput`-gated MIRROR of
      the console's byte stream, drained by `SYS_CONSOLE_ATTACH` (24)
      and `SYS_CONSOLE_PULL` (25). Two separate capabilities on
      purpose — a keyboard must not thereby gain the power to read
      everything the machine prints. Transport: `virtio-serial-pci` +
      a `virtconsole` port with MULTIPORT declined (QEMU marks a
      non-multiport port 0 guest-connected at DRIVER_OK, so two queues
      and no control protocol suffice), buffers posted AFTER DRIVER_OK
      (QEMU pauses a chardev whose frontend cannot read, and only a
      post-DRIVER_OK doorbell resumes it). Proofs: `m6:test:console_service`
      (a real round trip — bytes out the transmit queue that the
      harness reads off the host socket, and the harness's answer back
      in on the receive queue, verified byte-for-byte, both directions
      interrupt-completed, frame-exact teardown) and
      `tools/test_m6_console.py`, which drives the shell ENTIRELY over
      the port with the serial input channel dead: banner and prompt
      arrive on the port, `echo` is echoed and executed, a backspace
      erases a typo inside the same line discipline, and the machine
      halts because someone typed `shutdown` there. **Two latent
      kernel bugs surfaced and were fixed:** the M5.1 relay interrupt
      stubs clobbered `rcx` before saving it (any code interrupted with
      a live `rcx` resumed with the vector number in its place — a
      spawn record index arrived as 54), and boot-time TSC calibration
      halted the machine whenever a host stall skewed one PIT window
      (a busy laptop could fail to boot; now up to three rounds, and
      the same for the m2 cross-check). Gates: run_tests 13/13 (349
      assertions), fmt + clippy clean, stability 100/100.
- [x] 6.5 **driver framework hardening: supervised restart** — DONE
      (ADR-0028). ADR-0022 put drivers in ring 3 so a broken one could
      not take the kernel with it, and deferred the other half:
      isolation only pays if the SERVICE comes back. Three things had
      to be built, and the first two were not about supervision at
      all. (a) **A dead server is an ANSWER**: a client blocked in
      `SYS_IPC_CALL` has no timeout and no way to see its server's
      liveness, so `proc::destroy` now fails every call the dying
      process owed — Delivered or Waiting — with a new typed
      `STATUS_SERVICE_GONE` (-5) and wakes the caller. Which endpoints
      it owed answers on is decided by CAPABILITY (`Endpoint` + READ),
      so there is no registry to fall out of step. The status
      deliberately does not claim the request did not happen; the
      kernel cannot know. (b) **A blocked process can be KILLED**:
      `State::Blocked` has carried the note "until woken or (later
      milestones) killed" since M3.1, and this is that milestone —
      `sched::kill_threads_of` zombies parked threads where they
      stand, the reaper reclaims their stacks with the usual canary
      check, and `plan_switch` skips stale ready-ring entries.
      Ordering is load-bearing: every kernel reference to those
      threads is released FIRST, because waking a corpse halts the
      machine by design. (c) **The supervisor** (`kernel/src/supervise.rs`):
      a supervised service is an image plus the exact grant list it
      was spawned with, and a restart replays that list verbatim — an
      Mmio window is a physical base and a page count, so "the same
      grants" is identity, not reconstruction. Clients never learn a
      pid: their capability names the ENDPOINT, which outlives its
      server, so the new instance simply picks the serve side back up.
      Deaths are NOTICED in destroy and ACTED ON in `poll` (restarting
      allocates and maps; destroy's context must not). `MAX_RESTARTS`
      bounds it, and giving up leaves the service honestly OFFLINE.
      Also closes **ADR-0025's spawn-record GC debt** — the supervisor
      reaps the corpse's record, and the test asserts the record count
      is flat across a restart cycle. Proof: `userspace/faultd`
      (images 14/15), the smallest service in the system and the only
      one written to be murdered, driving the whole cycle on real
      processes — live service, blocked client, kill, restart with 3
      caps replayed, the blocked client answered and exiting cleanly,
      and a NEW client reaching the restarted instance through the
      capability it held before the crash; frame-exact throughout.
      NOT claimed: back-off, dependency ordering, state recovery, a
      ring-3 supervisor, or production wiring (`poll` must be called;
      the suite calls it). Gates: run_tests 13/13 (362 assertions),
      fmt clean.

## Phase 7 — Networking ✅ (bounded protocol/API v1; production stack supervision closed in 8.0)

NIC TX/RX via the virtio-net driver server (done, M6.1) → ARP → IPv4 →
ICMP → UDP → DNS resolver service → TCP (own stack server, async API) →
userspace net API library. Each protocol a separately testable milestone
with QEMU netdev (user-mode/slirp + tap tests, packet capture
assertions).

### Milestones

- [x] 7.0 **the timer facility + production supervision** — DONE
      (ADR-0029). Built first, before any protocol exists to need it,
      because building protocols first is how a codebase ends up with
      polling loops nobody removes. `SYS_CLOCK_NOW` (monotonic
      microseconds — the M2.2 clock the m2 suite re-checks every
      boot), `SYS_TIMER_ARM(notif_slot, badge, delay_us)` and
      `SYS_TIMER_CANCEL`. A timer delivers a BADGE BIT on a
      notification the caller already holds, so "device interrupt OR
      client request OR timeout" is an ordinary `SYS_WAIT` with one
      more bit set — no second thread, no new blocking primitive, and
      no timeout can be missed by a service that was waiting on
      something else. The capability gate is the notification (WRITE,
      as `SYS_IRQ_RELAY`), so no new capability kind was needed.
      Delays are RELATIVE (an absolute deadline races the clock read
      that produced it), timers are one-shot (a back-off is not a
      period), and resolution is stated rather than implied: checked
      on the 100 Hz tick, so a deadline means NOT BEFORE with ~10 ms
      of lag. Owned and swept at `proc::destroy` — the fourth sweep
      there, now a rule rather than four special cases. Supporting
      change: `kernel/src/tick.rs`, a dispatcher for deferred tick
      work, because the console mirror had taken the single auxiliary
      hook in M6.4. PROVEN by measurement, not assertion: `timertest`
      (image 16) armed a 50 ms timer that delivered at 52096us on the
      first boot — never early, 2096us of lag against a 10000us tick
      — a cancelled timer stayed silent, a second cancel was REFUSED,
      two due timers merged into one wake, and a timer abandoned at
      exit was swept. Also closes ADR-0028's open item: `supervise::poll`
      now runs in the boot thread's idle loop, so production
      supervision is a fact before the stack depends on netd. Gates:
      run_tests 14/14 (391 assertions), fmt + clippy clean.
- [x] 7.1 **ARP as a service (`netstackd`)** — DONE (ADR-0030). The
      first protocol, and the decision about where protocol state
      lives, settled before four more protocols each answer it by
      accident. `netstackd` (image 17) owns ARP and its cache; netd
      is unchanged and still has never heard of an ethertype. The
      split is STRUCTURAL: the stack is granted no device capability,
      so it could not touch the NIC if it tried. Proven on the real
      wire — 10.0.2.2 resolved to 52:55:0a:00:02:02 with the kernel
      witnessing transmit and receive interrupts; a second lookup
      served from CACHE with the count of requests actually put on
      the wire unmoved (the only honest proof of a cache); and a
      silent address reported UNREACHABLE after three real deadlines,
      which above all TERMINATED. That last part is why 7.0 came
      first: `NET_OP_RECV` used to block until a frame arrived, so a
      lost reply would have parked the stack forever, and a client
      blocked in `SYS_IPC_CALL` cannot observe its own timer
      (ADR-0029's erratum). The bound is passed DOWN — netd takes a
      timeout and arms a timer on its own notification — which makes
      the first production use of the timer facility the thing that
      makes the first protocol possible. Aging is by CLOCK not timer
      (nothing must happen when an entry expires); retry covers only
      the idempotent broadcast query, never a datagram send. Gates:
      run_tests 14/14 (402 assertions), fmt + clippy clean.
- [x] 7.1b **netd under real supervision, and re-attach** — DONE
      (ADR-0030, plus an amendment to ADR-0028). Decision 3 of the
      phase, done rather than promised: netd registered with the
      supervisor, KILLED mid-flight, restarted with its grants
      replayed, and netstackd re-establishing with the new instance —
      backing off on a real timer, re-acquiring its device facts, and
      re-sending only because an ARP request is idempotent. **It also
      found a hole in ADR-0028**: that decision gave a typed answer to
      callers in flight when a server died and left the neighbouring
      case open, so a call sent AFTERWARDS queued on an endpoint
      nobody would ever read and blocked forever — the same hang, one
      instant later, and exactly the one a stack hits because a stack
      discovers its driver is gone BY CALLING IT. Endpoints whose
      server is destroyed are now ORPHANED and refuse new calls;
      a restarted service clears that by its first `recv`. Making the
      test honest took three attempts (twice the driver was already
      back before the stack noticed, and zero re-attaches were
      reported); it now waits for the kernel's own evidence that the
      stack met the corpse before letting the supervisor work.

- [x] 7.2 **IPv4 + ICMP, on a receive demultiplexer** — DONE
      (ADR-0031). The simplification ADR-0030 named is retired: with
      one protocol an operation could read the wire itself, with two
      that is wrong — an echo reply arriving mid-resolve would be
      discarded and the ping waiting for it would time out for no
      reason. Recognising a frame is now one job in one place, with
      counters that must account for every frame (a stack that cannot
      say what it dropped is not demultiplexing, it is guessing).
      IPv4 is the smallest honest amount — fixed 20-byte header, no
      options, no fragmentation, no routing, everything on-link — but
      what exists is done properly: both checksums computed on send
      and VERIFIED on receive with failures counted, packets accepted
      only if addressed to us, and echo replies matched on identifier
      AND sequence, since one that merely arrived could answer
      somebody else's ping. Echo requests are deliberately NOT
      answered: nothing asked ArenaOS to be pingable and an untested
      reply path is worse than none. Proven first boot: echo reply
      from 10.0.2.2 in 1281us, timed on the monotonic clock, demux
      sorting 2 ARP and 1 IPv4 with none dropped. The layering shows
      in the failures too — an unresolvable address fails UNREACHABLE
      (no host), a silent resolved host would fail NO_REPLY.
      **Reviewed and tightened before UDP** (v0.14.0 review): an echo
      reply is now bound to the remote ADDRESS as well as the
      identifier and sequence; IHL must be exactly 5; fragments are
      refused; the payload is governed by the IPv4 header's
      total_length rather than the Ethernet frame (padding was being
      fed to the checksum and only passing because zero padding does
      not change a ones-complement sum); and echo replies must carry
      code 0. Those reject paths are exercised by a parser self-test
      on synthetic frames, which itself had to be hardened after it
      passed against a deliberately reintroduced regression. Gates:
      run_tests 14/14 (415 assertions), fmt + clippy clean.
- [x] 7.3 **frames larger than one IPC message** — DONE (ADR-0032).
      A REORDERING, stated rather than done quietly: the plan said UDP
      next, but netd dropped every frame over 64 bytes, so UDP would
      have been a protocol for tiny datagrams with a test that passed
      for reasons the real world would not reproduce. The receive path
      came first. netd now STAGES a frame in the ring where the device
      put it and serves it by offset, returning the full length with
      the first chunk and releasing the buffer on the last. Not
      zero-copy and does not pretend to be — one IPC round trip per 64
      bytes, right for a DNS answer and wrong for throughput; the
      caller's-frame-in-the-ring design is named as its own future
      milestone. Proven by making the EXISTING proof require it: the
      ICMP echo payload went 8 → 200 bytes, so every boot now puts a
      242-byte frame on the wire, and the reply is verified
      byte-for-byte with a POSITION-DEPENDENT pattern — a reassembly
      that dropped, duplicated or reordered a chunk could not pass,
      where matching identifier and sequence would not have caught it.
      Gates: run_tests 14/14 (418 assertions), fmt + clippy clean.
- [x] 7.4 **UDP, with authority by possession** — DONE (ADR-0033).
      `BIND` returns a 64-bit handle drawn from rngd and possession of
      it is the authority to use that port; deliberate passing is
      delegation. This is the design C proposed in review, and it is
      better than what I had planned: I had concluded that per-binding
      authority needed per-client endpoints, which confused AUTHORITY
      with IDENTITY. A service-issued token is exactly the capability
      model the rest of this system uses, one layer below the kernel —
      and the kernel still does not know what a port is. What remains
      true is only the narrower claim: non-transferable per-process
      ownership is unavailable on a shared endpoint under IPC v1.
      Handles are random on purpose (a guessable handle is authority
      by arithmetic) and without entropy the stack REFUSES to bind
      rather than issue a predictable one. Bind-once is enforced
      separately as a namespace rule, and both are tested: a second
      bind refused, a forged handle refused. Proven against a real
      server — a DNS query for example.com to slirp's resolver, the
      61-byte response matched on OUR transaction id and the response
      bit, so a datagram that merely arrived would not pass. Stated
      rather than hidden: the UDP checksum is sent as zero (legal) and
      a nonzero one is not yet verified, the inbox is single-slot, and
      RECV delivers the first message-worth with the full length
      reported. Gates: run_tests 14/14 (423 assertions), fmt + clippy
      clean.
- [x] **7.5 DNS A resolver** — DONE (ADR-0034). DNS_OP_LOOKUP in
      netstackd sends an A/IN question to slirp's actual resolver and
      returns an address only after checking source tuple, fresh
      rngd-drawn transaction id, response flags, matching question and
      answer owner. A bounded parser refuses bad compression, wrong
      questions, truncation and errors; host tests exercise negative
      cases. UDP now stages the whole payload per binding (bounded by
      FRAME_MAX=512), serves the rest under the SAME bearer handle in
      chunks, and computes/verifies UDP checksums with the IPv4
      pseudo-header. An odd-length synthetic control and corruption
      test keep the checksum honest. CLOSE+rebind now rotates the
      handle instead of reviving authority in a reused slot — a
      correctness fix required by ADR-0033, not a new identity model.
      A real 61-byte DNS answer crosses the inline boundary in the
      boot proof; the resolver independently parses a live answer and
      refuses malformed names before sending. No caching, CNAME,
      DNSSEC, DHCP/configuration, or TCP fallback is claimed. Gates:
      run_tests 15/15 (two host parser cases + M1–M7 boots),
      netstackd fmt and clippy clean.
- [x] **7.6 bounded TCP active-open** — DONE (ADR-0035). `netstackd`
      exposes a client-driven OPEN/POLL/WRITE/READ/CLOSE/RELEASE state
      machine behind an rngd-backed bearer: possession, not caller
      identity, authorizes use. OPEN returns after bounded ARP and SYN
      send; POLL advances handshakes, data and FIN without blocking the
      service on an unbounded receive. Pseudo-header checksums, tuple
      validation, MSS/window 400, in-order bounded receive, a
      stop-and-wait transmit with same-sequence bounded retransmission,
      and explicit failure/close/revocation are implemented. An
      independent Linux TCP peer sees the exact request and orderly EOF;
      the guest checks all 200 position-dependent response bytes across
      IPC chunks, FIN acknowledgement and handle revocation. The TCP
      burst exposed two driver bugs: a single held RX frame dropped
      subsequent completions, and overlapping RX/TX notification badge
      *masks* misclassified interrupts. netd now retains all three
      completed buffers in order and uses disjoint single-bit badges.
      No passive listening, concurrent connections, reordering queue,
      congestion control or general sockets claimed. A no-peer boot
      explicitly SKIPs TCP yet reaches the shell cleanly; qualification
      boots require the real peer and refuse SKIP. Gates: run_tests
      17/17, netstackd/netd fmt + clippy clean, artifact-bound 100/100
      boots with host and guest wire/close proofs.
- [x] **7.7 userspace networking API library** — DONE (ADR-0036).
      The no_std `userspace/net.rs` now provides a typed endpoint Client,
      explicit transferable UDP/TCP bearers, ARP resolve, ICMP ping, DNS
      A, UDP bind/send/full-datagram receive/close, and TCP active-open/
      poll/write/read/close/release. The library enforces request sizes,
      validates replies and drains UDP IPC continuations under the same
      bearer; `close`/`release` are explicit, not unreliable Drop-time
      side effects. No kernel UDP capability or caller-identity claim.
      Strict host tests verify byte-exact calls, offsets, errors and
      authority lifecycle. The guest uses the public library against
      real ARP, ICMP, DNS and Linux TCP peers and sends a second real
      DNS query over its UDP API, checking all bytes beyond the inline
      boundary and revocation. Gates: run_tests 18/18, final-image
      100/100 boots, matching SHA-256 receipt. Phase 7 bounded v1 is
      complete; this is NOT a POSIX sockets or unrestricted TCP claim.

**Phase 7 lifecycle obligation — CLOSED in Phase 8.0:** The Phase 7
protocol/API milestone originally left production `netstackd` supervision
open. ADR-0037 assigned its lifecycle to one ring-3 service manager,
not both manager and kernel. ADR-0039–0045 now prove manager-owned initial
spawn, bounded orderly/crash/forced-stop recovery, exact accounting,
real held-cap refusals and active external probes before *every* new
production child. `tools/test_m8_*.py` and final-EFI-bound 100/100 boots
qualify the closure. Drivers remain exclusively kernel-supervised.

### Three decisions taken BEFORE any protocol code

Recorded here rather than discovered later, because each one is
expensive to reverse once a protocol depends on it. Each gets a full
ADR at implementation; these are the commitments.

**1. `netd` stays strictly at L2. Protocol state lives elsewhere.**
netd owns the device and nothing above it: raw Ethernet frames in and
out, the MAC from config space, virtqueues, interrupts. It does not
parse an ethertype, hold an ARP entry, or know what an IP address is —
exactly as ADR-0024 built it and as `nettest` demonstrated by
hand-building its ARP frame.

Everything above L2 goes in a separate resident service,
**`netstackd`**: the ARP cache and its aging, IPv4, ICMP, UDP demux,
DNS, and eventually TCP. Applications talk to netstackd; only
netstackd talks to netd.

The reason is not tidiness. A driver's job is to survive its device
and be restartable (M6.5); a protocol stack's job is to hold
connection state across time. Mixing them means a wedged NIC takes
every connection with it, and a protocol bug can only be fixed by
restarting the thing that owns the hardware. Keeping the split means
netd can be killed and restarted under a live stack — which is
precisely what decision 3 has to prove.

**2. The timer facility is built FIRST, as 7.0, before ARP.**
Ring 3 has no timers today. TCP retransmission, ARP aging, DNS
timeout and retry, connection establishment timeouts, and later
TIME_WAIT all need real ones, and ADR-0028 already recorded the gap
from the other side: an IPC client cannot currently time itself out.
Building protocols first and discovering this later is how a codebase
ends up with polling loops that never get removed.

The shape, to be ADR'd with the implementation:

- `SYS_CLOCK_NOW` — monotonic microseconds, the clock M2.2 already
  calibrated, read-only.
- `SYS_TIMER_ARM(notif_slot, badge, deadline_us)` — a one-shot timer
  that NOTIFIES an existing notification with a badge bit when the
  deadline passes. Returns a timer id.
- `SYS_TIMER_CANCEL(timer_id)`.
- Timers are owned by the process and swept at `proc::destroy`, the
  same way relay vectors (M5.2), the console mirror (M6.4), and
  blocked-thread references (M6.5) already are. That sweep is now a
  pattern, not a special case.

Delivery by NOTIFICATION is the whole point: every service already
blocks on exactly one notification with merged badge bits, so "wait
for a device interrupt OR a client request OR a timeout" needs no new
blocking primitive and no second thread. **Explicit anti-goal:** no
polling loops and no busy-waits anywhere in the stack. If a milestone
in Phase 7 finds itself spinning on a clock read, the timer facility
is wrong and gets fixed, not worked around.

Granularity is stated honestly rather than implied: the tick is
100 Hz, so a deadline means "not before", with ~10 ms resolution.
That is ample for RTO minimums, ARP aging, and DNS retry; nothing in
Phase 7 may quietly assume finer.

**3. Production supervision comes before the stack depends on netd,
and retry policy is the stack's — per operation.**
ADR-0028 built supervised restart but left `supervise::poll` uncalled
outside the suite. Wiring it into the boot/idle path is part of 7.0,
so that by the time netstackd exists, a netd that dies is a netd that
comes back. 7.1 then proves it from the stack's side: kill netd
mid-exchange and show the stack recovers.

`STATUS_SERVICE_GONE` means the outcome is UNKNOWN, not "did not
happen", and the stack must act accordingly. Blanket retry is
forbidden; the policy is per operation:

- **Datagram TX (UDP/IP): never resend on SERVICE_GONE.** The frame
  may already be on the wire. UDP is unreliable by contract, so "may
  or may not have been sent" is within what the application signed up
  for — manufacturing a duplicate is not.
- **ARP requests: safe to re-send.** A broadcast query is idempotent.
- **TCP: let the protocol handle it.** Retransmission with sequence
  numbers is TCP's own job; a SERVICE_GONE during send is exactly the
  case the RTO covers, so the stack adds no special case and
  certainly no duplicate.
- **Receive paths: re-establish, never retry.** A restarted netd has
  lost its posted RX buffers and queue state, so the stack must
  RE-ATTACH (re-lend buffers, re-arm) rather than assume continuity.
  This is an explicit step in the netd protocol, not an implicit
  recovery.

`netstackd` supervision remains OPEN: the test instance exists, but no
production resident stack is registered. ADR-0037 makes a single
user-space manager own its production lifecycle in 8.0; neither this
paragraph nor the Phase 7 protocol/API verdict marks that done.

### Scope firewall for Phase 7

Phase 7 is not a BSD-sockets compatibility project. The API is
designed for this OS (capability-addressed, message-based,
asynchronous) exactly as ADR-0001 requires. A POSIX/sockets
compatibility layer remains an optional, later, separately-ADR'd
decision — if a milestone starts shaping the stack around
`bind`/`listen`/`accept` semantics, that is a scope violation unless
an ADR has explicitly chosen it first.

## Phase 8 — Mature userspace 🔨 (8.0–8.3 complete; 8.4+ not started)

Authority model: accepted ADR-0037–0045 and ADR-0047. The roadmap below is the sequence,
not permission to implement later steps early. Every completed
milestone gets its own real negative-space tests, the entire historical
suite and a fresh, artifact-bound 100/100 boot qualification. Phase 8
is not completed by accepting an ADR.

- [x] **8.0 service manager + declarative service manifests v1**.
      COMPLETE (27 historical suites and fresh final-EFI-bound 100/100
      QEMU boots; ADR-0037–0045) — the bounded resolver consumes a caller-cap-only
      syscall inventory; two real shell child cycles exercise the
      Process-cap finish ABI. A ring-3 manager now boots with kernel-
      audited literal grants, observes its real caps and waits for
      device-originated readiness badges (ADR-0038). It now requests
      one production `netstackd` through `SYS_SPAWN`, checks its ready
      badge and Process handle, and the kernel audits all four child's
      installed grants (ADR-0039). It now uses its held Process cap to
      reap an orderly exiting production child, waits for bounded timer
      backoff, revalidates live grants, and starts a replacement behind
      the SAME endpoint. A privileged shell client proves an old bearer
      fails and new ARP work reaches the real wire (ADR-0040). An opt-in
      administrator test now measures exact free frames, spawn records
      and process slots after EACH of three production restarts, then
      proves the fourth exit exhausts the bounded budget (ADR-0041).
      An opt-in test now causes a genuine production-child #UD during an
      unanswered IPC; the kernel isolates the ring-3 fault, the manager
      reaps and restarts, and the client gets typed SERVICE_GONE before
      resumed real-wire service (ADR-0042). The Power-holding admin can
      now request a private manager control event: a forged shared wake
      alone cannot stop the child, but the manager's held Process cap
      force-stops a *live* production child and restores wire service
      (ADR-0043). Root-issued diagnostic Process references now prove
      self/manager/driver and foreign read-only lifecycle refusals,
      forged/empty/wrong-kind/stale-cap refusal, a positive user-child
      reap, and live post-refusal wire service (ADR-0044). The test
      exposed and fixed the distinction between kernel-bootstrapped
      roots and user-child spawn records. Before both initial spawn and
      every restart, a bounded worker now probes the REAL netd MAC and
      rngd device entropy protocols, then reaps its held Process cap.
      Separate opt-in #UD and stalled-driver negative fixtures prove
      fail-closed OFFLINE before respawn; the driver's genuine #UD also
      exposed and closed deferred supervised-driver teardown (ADR-0045).
      These four independently tested areas close the original exit
      gates, not a success string on an unattended boot. ADR-0047 then
      fixed the receiver-side authority gap discovered during 8.0
      review: fault, stall and all poison/shutdown operations require
      a transferred boot-granted diagnostic reference checked by the
      *receiving service*, never just the client's data endpoint.
      Missing and wrong markers are refused in real IPC before the
      existing #UD/stall proofs; the historical driver/FS/console
      shutdown fixtures retain their positive and negative proofs.
      The final checkpoint was freshly requalified after this fix.
      Complete recovery from the fixed kernel trust root with only
      explicit Image/Endpoint/Notification/Process authority;
      kernel alone mints MMIO and keeps driver lifecycle. The manager
      can delegate only actual caps already held, attenuated by
      `SYS_SPAWN`; requested manifest grants are checked against the
      actual inventory and audited against child caps. No typed match,
      no child. Bounded static service graph, missing-dependency/cycle
      refusal, explicit readiness before dependents, restart-on-exit
      with timer backoff and budget, observable OFFLINE state. Add the
      narrow Process-cap-gated reap path needed to keep restart
      records/frames flat; refuse kernel-supervised children. The
      *production* `netstackd` is the first managed service: prove a
      real kill/restart through the same endpoint, stale bearer refusal
      and resumed wire service. This is the required closure of the
      Phase 7 stack-supervision obligation; it is now DONE with the
      manager-owned production restarts and active probes. No
      dynamic permission UI, generic device-cap request, or competing
      kernel/user restart owner (ADR-0037). Existing M1–M7 regressions
      must continue to pass including the no-peer boot.
- [x] **8.1 transactional configuration store**. COMPLETE — accepted
      ADR-0046 selects a bounded single-key, immutable-generation design,
      explicit receiving-service update marker and the existing AFS1
      ordered-write/atomic-sector crash model. The byte-exact `no_std`
      codec and fail-closed generation scanner compile for the bare-metal
      target and match the independent Python reference. A resident
      `configd` scans real fsd-visible records; an isolated ordinary
      reader proves UNSET, exact host-provisioned VALUE, fail-closed
      malformed/newest-checksum rejection and 20 receiver-side SET
      refusals. The **transactional-core checkpoint** adds a distinct
      boot-granted updater: actual marker-authorized
      CREATE/WRITE/CLOSE/rescan, two different guest values, seven
      SIGKILL/reboot recovery gates on the SAME disk, eight byte-exact
      immutable generations and typed ninth/full-disk refusals. The
      trusted raw-FS shell only stages a test-intent file; it never
      receives update authority. Closure measures exact post-EBS-relative
      free-frame consumption / spawn records / process slots `(254,10,10)`
      across no-update, commit, no-op, all eight generations and the ninth
      refusal. Same-boot disk-full forces a second real marked SET to return
      `DEGRADED`, with an exact old-value READ; safe full-table preflight
      remains `NO_SPACE` and does not degrade.
      AFS1 can hide a newer generation if its commit sector suffers
      arbitrary corruption; after the explicit scope decision, 8.1
      rejects **visible malformed config records** rather than pretending
      to detect media corruption or rollback outside that model. The
      existing guest tests exercise CREATE/WRITE-commit/CLOSE/reply kill
      boundaries without silently substituting an older visible record.
      No GC or 8.2 UI yet.
- [x] **8.2 permission manifests + CLI grant workflow**. ADR-0048/0049:
      one mediator endpoint, receiver-checked approval marker, explicit
      request/decision/issued-authority separation, manager-owned broker
      and mediator-only app with exact four/one grants, fixed 32-slot cap
      space, rngd-issued 128-bit bearers and receiver-side invalidation of
      independently copied bytes. The bound no_std codec and exact
      transactional `perm8-*` immutable records survive same-disk reboot
      and real broker Process-cap reap/restart; the old bearer does not.
      ALLOW, DENY and REVOKE each survived seven actual SIGKILL write
      boundaries and independent AFS1 audits. Malformed newest records,
      ninth-generation/full-disk refusal, and absent rngd/real fsd fail
      closed; ADR-0051 reaps an exited kernel-root fsd to orphan its
      endpoint instead of stranding callers. No ambient pid/name grant,
      anti-rollback claim, arbitrary corrupted commit-sector guarantee,
      graphics, signatures, or package authority. The previous fixed-slot
      and volatile checkpoints remain available as historical binaries;
      this completed checkpoint is qualified separately.
- [x] **8.3 standard userspace libraries** (ADR-0052). A separately
      compiled no_std `arena-lib` crate links the existing frozen ABI,
      checked syscall/IPC transport (typed service status versus kernel
      transport, unexpected returned-cap refusal), bounded AFS1 file
      client with explicit Endpoint/LENT caps, and the existing native
      ARP/ICMP/UDP/DNS/TCP client with receiver-issued bearer semantics.
      `fstest` and production `permissiond` are independent linked FS
      consumers; `arptest` and the shell are independent linked native
      network consumers, all crossing the same `ipc`/`sys` boundary.
      Host fake transports test byte-exact wire and negative space;
      QEMU proves genuine AFS1 DMA/durable approval and real-wire ARP,
      plus no-stack SKIP and wrong-kind refusal. No new kernel primitive,
      cap grant, ambient name/identity authority, dynamic runtime or
      POSIX socket compatibility. Prior M5 disk-operation counts and M8
      resource/restart invariants stay exact; the original 42-suite and
      artifact-bound 100/100 qualification are recorded in ADR-0052.
      The corrective 8.3 checkpoint fixes syscall-backed unexpected
      IPC reply-cap disposal (not just parser refusal): the controlled
      M6 service returns an inert cap 40 times to each of two linked
      clients; every landed slot is destroyed, baseline 2/32 occupancy
      remains exact, normal no-cap replies work, and a deliberate
      cleanup-removal mutation fails the guest regression. FS helper
      success words remain caller-validated, not library-certified.
      Fresh corrective historical suite 43/43 and final-image-bound
      100/100; `phase83-corrected` archive supersedes original 8.3.
- [ ] **8.4 package format + signed packages**. ADR-0053 is **Proposed,
      design complete for user/C review**; no 8.4 implementation until their
      approval, and the exact vendored-source audit and independent wire
      vectors remain gates before ADR acceptance/dependent code. There is
      no 8.4 signature code, install authority or qualification yet. Define
      package identity, public test root, cumulative revocation and canonical
      signature domains; test tamper and bounded same-disk crash recovery
      and guest **receiver-side verified staging only**. Installation,
      activation and dynamic linking remain out of scope until 8.5.
- [ ] **8.5 installer/updater**. Artifact-bound install/upgrade and
      power-loss recovery tests with an explicit trust chain and
      rollback rules. No assumption of real-hardware drivers or
      secure-boot integration without separate proof.

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
  green. **Before committing a completed phase/milestone**, build its
  final image with `tools/build.sh --image` and run
  `tools/stability_loop.sh 100` on that image. Require 100/100,
  zero failures and a receipt whose kernel SHA-256 matches the built
  artifact. If the image changes, the qualification must be rerun;
  a past receipt is not evidence for new bits. Releases still require
  the full suite and artifact-bound qualification as separate gates.
- New subsystem ⇒ new `m<N>:test:*` markers + new `tools/test_m<N>.py`;
  old test scripts are never deleted.
- Significant decisions inside a milestone ⇒ ADR before merging the code
  that depends on them.
- A milestone that cannot be demonstrated by a test does not count as done,
  no matter how good the code looks.
