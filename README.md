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
| Milestone 2 — Kernel foundations (exceptions, timers, frames, paging, heap, locks, boot split) | 🔨 in progress — 2.1 (TSS/IST, exception recovery) + 2.2 (PIT/TSC calibration, monotonic clock, 100 Hz tick) done | `tools/test_m2.py` (7/7, same boot re-proves M1 8/8) |
| Milestone 3 — Multitasking · 4 — Userspace & first program | ⬜ | — |
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

## What just booted (Milestone 1)

QEMU/OVMF loads `EFI/BOOT/BOOTX64.EFI` (our Rust boot stage). It disables
interrupts, installs its own GDT (verified by `sgdt` read-back), initializes
the 16550 UART (verified by hardware loopback), inspects CPU state (long
mode, CR0/CR3/CR4/EFER, CPUID), parses and classifies the real UEFI memory
map, runs six self-tests, reports `m1: RESULT PASS (6/6)` over serial, and
halts the machine safely via UEFI `ResetSystem`. The test harness asserts
every one of those facts and that QEMU exits cleanly. No fake output: each
test fails loudly if the underlying mechanism is broken.

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
