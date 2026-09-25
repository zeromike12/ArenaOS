# ADR-0003: Boot strategy — the boot stage is our own UEFI application

Status: Accepted
Date: 2026-09-25
Milestone context: Phase 0 — Architecture, delivered in Milestone 1

## Problem

Get our kernel from power-on to running our code in 64-bit long mode, with a
memory map and console, on x86-64 PCs — without inheriting another project's
boot chain, and keeping the eventual secure-boot story ours to control.

## Options considered

**GRUB (multiboot2):** mature, ubiquitous on BIOS+UEFI. Rejected: large
external runtime dependency in the most security-critical link of the chain;
multiboot2's information model is dated; GRUB's own CVE history sits directly
under our kernel; and we would debug two projects' boot paths.

**Limine:** modern, pleasant, maintained. Rejected for the same structural
reason — the boot chain is part of *our* product (VISION: own boot process;
secure boot later). Also a smaller external project to bet two decades on.

**Direct UEFI application (chosen):** UEFI firmware already loads PE32+
images from the ESP and hands us long mode, paging, and a memory map. We write
the EFI-application entry ourselves in Rust (`x86_64-unknown-uefi`), parse the
UEFI system table with our own bindings (ADR-0004), then transition to the
kernel proper.

**Direct kernel + own firmware:** (writing our own UEFI-class firmware) —
rejected: absurd scope; firmware is the one layer where standing on the
industry standard is free and correct.

## Decision

- The boot stage is a UEFI application: `EFI/BOOT/BOOTX64.EFI` on a FAT ESP,
  built from `kernel/boot/` with the Rust `x86_64-unknown-uefi` target.
- The boot stage's job list (grows across milestones, fixed contract):
  1. Verify/establish CPU state: long mode confirmed, own GDT installed,
     interrupts disabled until the kernel's IDT exists. (M1)
  2. Bring up a polled serial console. (M1)
  3. Capture firmware info: memory map (with map key), ACPI RSDP and other
     configuration tables, GOP framebuffer parameters when present. (M1
     captures memory map; rest in M2)
  4. [M2] Build kernel page tables: higher-half kernel, W^X, NX; allocate
     kernel structures; set up IDT and switch to the kernel's own runtime.
  5. [M2] Call `ExitBootServices()` with the current map key, then hand off
     to the kernel entry point with a **boot-info record** — the single
     arch-neutral contract between "whoever booted us" and "the kernel".
- QEMU/OVMF (EDK2) is the reference firmware for development and CI-style
  tests. Physical firmware diversity is a Phase-8 hardening problem, not a
  Phase-1 one.
- Shutdown/reset for testing and "halt safely" uses the UEFI
  `ResetSystem()` runtime service (verified working in QEMU at M1) — no
  dependency on emulator-only devices like isa-debug-exit in the kernel's
  normal path.

## Reasoning

Owning the boot chain is a requirement of the security vision (measured/signed
boot later), removes an entire external dependency class, and UEFI does the
two genuinely annoying parts for us (mode transition, firmware quirks). The
cost — writing UEFI table bindings by hand — is small, mechanical, and
well-specified, and we need deep familiarity with the memory map anyway.

## Downsides accepted

- No BIOS/CSM support: machines without UEFI cannot boot ArenaOS. Accepted —
  UEFI has been universal on x86-64 PCs for over a decade.
- We inherit UEFI's warts (calling conventions, memory map mutation rules,
  firmware bugs). Contained: all firmware interaction is quarantined in the
  boot stage; the kernel proper sees only the boot-info record.
- Hand-written UEFI bindings must track spec offsets precisely; mitigated by
  static assertions and the M1 boot test running against real EDK2 on every
  change.

## Future implications

- The boot-info record is the portability seam: an ARM64 boot stage (later)
  produces the same record; a fallback multiboot2 shim (only if ever needed)
  would too — kernel code never learns who booted it.
- Secure boot (Phase 8+): we sign *our* PE image; no shim ecosystem required.
- KASLR (Phase 2+): the boot stage owns physical placement, so kernel
  randomization is a boot-stage feature, not a kernel-internal one.

## Addendum (M1 implementation, 2026-09): measured handoff facts

The boot-strategy decision survived first contact with EDK2, with three
measured facts now constraining all boot-stage code (details and post-mortem
in `docs/ARCHITECTURE.md` §interrupt-controller handoff and
`docs/TESTING.md` §case study):

- UEFI boot-service calls execute `sti` on return (TPL restore). Our GDT and
  IDT are therefore installed together, before the first firmware call, and
  our own code runs IF=0.
- Firmware hands off in IOAPIC→LAPIC mode with the legacy 8259s masked *and
  unprogrammed* (irq_base=0). Interrupt stubs must EOI the LAPIC; touching
  8259 masks before reprogramming ICW2 delivers vector 0 (#DE).
- Firmware memory-map descriptors include multi-GiB reserved MMIO spans;
  RAM accounting must classify descriptors, not sum them.
