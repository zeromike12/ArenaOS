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
| Milestone 3 — Multitasking (threads, scheduler, processes, capabilities) | 🔨 in progress — 3.1 (kernel threads + context switch: callee-saved frame, no-FPU invariant build-enforced, canaried 32 KiB stacks, exact-accounting reap, ADR-0012) done | `tools/test_m3.py` (5/5) + m1/m2 regressions green |
| Milestone 4 — Userspace & first program | ⬜ | — |
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
`qemu-system-x86_64` command, serial is the console, and the VM shuts
itself down cleanly when the milestone suite finishes.

## What just booted (Milestone 2)

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
and heap accounting returns exactly to baseline (`m3: RESULT PASS
(5/5)`). Finally the machine shuts down through a farewell island that
hands control back to firmware's `ResetSystem` in firmware's own address
space. Every claim above is a machine-checked serial marker; nothing is
decorative.

## Documentation map

- [docs/VISION.md](docs/VISION.md) — what ArenaOS is and what it refuses to be
- [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) — system architecture, kernel
  architecture comparison & choice, memory/process/IPC/driver/security/FS/API
  philosophy
- [docs/adr/](docs/adr/README.md) — architecture decision records
  (language, kernel architecture, boot strategy, dependency policy, testing,
  ABI philosophy)
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
