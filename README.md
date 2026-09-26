# ArenaOS

A general-purpose operating system for x86-64 UEFI computers, written from
scratch in Rust. Not based on Linux. Not a Unix clone. A capability-based
hybrid microkernel with userspace drivers, explicit permissions, and its own
APIs — built milestone by milestone, where every milestone is demonstrated by
automated tests that boot the real system in QEMU.

> This is a serious long-term engineering project. The architecture is
> documented before it is built, every decision that matters has an ADR, and
> the tree is bootable at every commit.

## Status board

| Phase / Milestone | State | Evidence |
|---|---|---|
| Phase 0 — Architecture (vision, ADRs, toolchain) | ✅ complete | `docs/` |
| **Milestone 1 — First boot** (UEFI → kernel → verified diagnostics → safe halt) | ✅ **complete** | `tools/test_m1.py` (8/8 in-guest self-tests, clean QEMU exit) |
| **Milestone 2 — Kernel foundations** (exceptions, timers, frames, paging, heap, locks, boot split) | ✅ **complete** — 2.1 (TSS/IST, exception recovery) · 2.2 (PIT/TSC, monotonic clock, 100 Hz tick) · 2.3 (frame allocator, ADR-0007) · 2.4 (own page tables, higher-half, W^X/WP/NX enforced, ADR-0008) · 2.5 (kernel heap, ADR-0009) · 2.6 (spinlocks, irqsave critical sections, ADR-0010) · 2.7 (boot split: ExitBootServices → kernel proper, reclaimed timer chain, farewell-island shutdown, ADR-0011) | `tools/test_m2.py` (21/21) + 100/100-boot stability loop + host suites (arena-heap 12/12, arena-sync 6/6) |
| **Milestone 3 — Multitasking** (threads, scheduler, processes, capabilities) | ✅ **complete** — 3.1 (kernel threads + context switch: callee-saved frame, no-FPU invariant build-enforced, canaried 32 KiB stacks, exact-accounting reap, ADR-0012) · 3.2 (timer-driven preemption: tick hook, nested cooperative switch, per-CPU run queues, exact-RR proof on yield-free threads, ADR-0013) · 3.3 (ring-3 threads, syscall/sysret boundary, TSS RSP0, SMEP/SMAP armed and fault-tested; processes = address-space objects: private PML4, cloned kernel half, exact teardown, ADR-0014) · 3.4 (capability spaces: per-process slot tables, attenuation-only delegation, right-gated destroy, gated `process_root`/`map_memory` invokes, ADR-0015) | `tools/test_m3.py` (13/13) + m1/m2 regressions green + 100/100-boot stability + release [v0.3.0](https://github.com/zeromike12/ArenaOS/releases/tag/v0.3.0) |
| Milestone 4 — Userspace & first program | ✅ complete — 4.1 (executable format + image loader: ELF64 container with ArenaOS strict-subset semantics — ET_EXEC-only validator, W^X segments, zero-filled NOLOAD bss, exact-accounting load into a process space; test image is a genuine cargo/rust-lld artifact in `userspace/payload`, ADR-0016) + 4.2 (syscall ABI v1: six argument registers, typed i64 status codes, frozen call registry — `debug_write`/`thread_exit` — with the marshalling and callee-saved promises proven from ring 3, ADR-0017) + 4.3 (first user process: the real rust-lld image runs at ring 3 in its own address space — writes its pinned message via debug_write byte-identical to the file, stamps bss from ring 3, exits via thread_exit with META's own success code) + 4.4 (IPC v1: endpoint rendezvous with blocking call/reply, badged merged notifications, capability transfer in messages — an echo-server demo across two real processes, ADR-0018) + 4.5 (spawn protocol v1: SYS_SPAWN from image capabilities with explicit attenuating inheritance, Process handles, exit-badge notifications — a ring-3 supervisor spawns the real image twice and the restart is visible as its console message appearing twice, ADR-0019) + 4.6 (minimal shell: interrupt-driven console input with a kernel line discipline, a real second userspace image spawned at boot as the initial service, builtins help/ps/echo/spawn/shutdown, the machine halt gated on a Power capability — every test boot now ends by typing `shutdown` into the running shell, ADR-0020) done | `tools/test_m4.py` (9/9) + `tools/test_m4_shell.py` (interactive session, 26 checks) + m1/m2/m3 regressions green |
| Phases 5–10 — Storage, drivers, net, userspace maturity, graphics, desktop | ⬜ | `docs/ROADMAP.md` |

## Quickstart (this sandbox)

```bash
tools/dev-env/bootstrap.sh     # one-time: Rust 1.97 + bare-metal sysroots + QEMU/EDK2 + musl (idempotent)
source tools/dev-env/env.sh    # shell environment
tools/run_tests.sh             # build → boot in QEMU → assert milestone markers → verdict
tools/run.sh                   # interactive: watch the system boot on serial (Ctrl-A X to exit)
```

On a normal workstation with system packages (`qemu-system-x86`, `ovmf`,
Rust with the `x86_64-unknown-uefi` target), the same scripts work
unmodified — see `docs/DEV-ENV.md` for resolution order and overrides.

## Run a released build in your own QEMU

Every completed milestone ships as a GitHub release: a prebuilt boot
image plus the exact EDK2 firmware pair it was tested against. See
**[docs/RUNNING.md](docs/RUNNING.md)** — one `cp`, one
`qemu-system-x86_64` command, serial is the console in BOTH directions:
after the boot-time test suites pass, the kernel spawns the shell and
the machine waits for you at the `arena> ` prompt — type `help`, and
`shutdown` when you are done.

## What just booted (current: Milestone 4 complete — the system boots into an interactive shell)

QEMU/OVMF loads `EFI/BOOT/BOOTX64.EFI` (our Rust boot stage). It brings
up serial, GDT/IDT/TSS, the 16550 UART, and the real UEFI memory map;
calibrates the TSC against the PIT oscillator; installs its own page
tables (higher-half direct map, per-section W^X, WP+NXE enforced and
fault-tested); grows a guarded kernel heap from the frame allocator; and
proves spinlock/irqsave critical sections against live PIT hardware —
re-running every Milestone-1 self-test on the way (`m1: RESULT PASS
(8/8)`).

Then the M2.7 handoff (ADR-0011): `ExitBootServices()`, CR3 switches to
the kernel-only view through a trampoline (identity mapping torn down,
verified by a recovered fault), PE base relocations are replayed at
`+KERNEL_OFFSET`, and `kmain` validates the `BootInfo` ABI record. The
kernel reclaims the timer chain firmware left behind — the PIT arrives on
IOAPIC **pin 2** (the classic ISA-IRQ0→GSI-2 override), re-routed to our
vector — and proves two live ticks through the relocated IDT with
kernel-alias LAPIC EOI (`m2: RESULT PASS (21/21)`).

Then Milestone 3 begins: `kmain` registers itself as the bootstrap
thread and the scheduler runs real kernel threads (ADR-0012) — exact
round-robin interleave order, callee-saved registers round-tripped
*through* a live context switch, canaried 32 KiB stacks that stay
disjoint under depth-200 recursion, and a 127-thread churn whose frame
and heap accounting returns exactly to baseline. Then the 100 Hz PIT
tick starts driving the scheduler itself (ADR-0013): three threads that
contain *no yield call at all* are rotated in exact round-robin order
by timer preemption (~200k loop iterations each), and cooperative
yields provably compose with 10 ms quanta. Then the ring-3 boundary
comes up (ADR-0014): hand-assembled user payloads execute at CPL 3 and
cross it through real `syscall`s with SMEP/SMAP armed — a privileged
instruction in user mode faults and resumes, a bare kernel read of a
user page takes the SMAP #PF — and processes become address-space
objects: private PML4s with cloned kernel halves, same VA over distinct
frames per process, exact create/destroy accounting. Every process also
anchors a 16-slot capability space (ADR-0015): rights attenuate on
copy (amplification is a loud refusal), destroy is right-gated and
removes references only — dangling caps refuse cleanly — and two
gated invokes (`process_root`, `map_memory`) bind untyped frames into
a target's address space under WRITE rights on both caps
(`m3: RESULT PASS (13/13)`). Milestone 4 begins with the executable
format (ADR-0016): a genuine cargo/rust-lld ELF artifact
(`userspace/payload`, embedded at compile time) is validated field by
field against the ArenaOS strict ELF subset, 25 mutation classes of it
are refused, and a real process loads it — 3 pages, 7 frames, PTEs
W^X-exact, entry stub + META manifest + 4 KiB of zeroed NOLOAD bss
read back under the *target's* CR3 through STAC-bracketed accesses,
double-load refused at zero cost, teardown exact. The syscall ABI v1
(ADR-0017) then proves six-register marshalling and typed status codes
from ring 3; the payload image RUNS as the first user process (M4.3);
IPC v1 (ADR-0018) lands endpoints, blocking call/reply, badged
notifications and capability transfer with an echo-server demo across
two processes; the spawn protocol (ADR-0019) lets a ring-3 supervisor
create processes from image capabilities with explicit attenuating
inheritance and restart a worker through its exit badge; and the
console learns to LISTEN (ADR-0020): COM1 RX on IRQ4/vector 33, a
kernel line discipline, and a real second userspace image — the shell —
spawned at boot as the initial service (`m4: RESULT PASS (9/9)`).

The machine no longer shuts itself down: it ends the suites by handing
the console to the shell and waits at the `arena> ` prompt. It stops
only when a process holding the Power capability asks — `shutdown` in
the shell routes through the farewell island that hands control back to
firmware's `ResetSystem` in firmware's own address space (the test
harnesses type `shutdown` for you, marker-paced, so every automated
boot proves the whole chain). Every claim above is a machine-checked
serial marker; nothing is decorative.

## Documentation map

- [docs/VISION.md](docs/VISION.md) — what ArenaOS is and what it refuses to be
- [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) — system architecture, kernel
  architecture comparison & choice, memory/process/IPC/driver/security/FS/API
  philosophy
- [docs/adr/](docs/adr/README.md) — architecture decision records
  (language, kernel architecture, boot strategy, dependency policy, testing,
  ABI philosophy, allocator, address space, heap, sync, boot split, threads,
  preemption, processes/ring-3, capabilities, executable format)
- [docs/ROADMAP.md](docs/ROADMAP.md) — milestones with exit criteria + the
  "do not build yet" firewall
- [docs/RISKS.md](docs/RISKS.md) — risk register
- [docs/DEV-ENV.md](docs/DEV-ENV.md) — toolchain bootstrap, build/boot/debug
- [docs/RUNNING.md](docs/RUNNING.md) — running a release build in your own QEMU
- [docs/TESTING.md](docs/TESTING.md) — testing doctrine and marker grammar
- [docs/CODING-CONVENTIONS.md](docs/CODING-CONVENTIONS.md),
  [docs/REPO-LAYOUT.md](docs/REPO-LAYOUT.md)

## Ground rules (enforced, not aspirational)

1. **Always bootable.** Every commit builds and passes all milestone tests.
2. **Never fake functionality.** A subsystem is done when a test would fail
   if it were faked (ADR-0005).
3. **Control scope.** Smallest useful version of everything; the firewall
   list in ROADMAP.md is binding.
4. **Decisions get ADRs.** Architecture does not live in chat history.
