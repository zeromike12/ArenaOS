# ADR-0008: Kernel address space — dual-view paging with per-section W^X

Status: accepted (M2.4, 2026-09)

## Problem

The boot stage runs on the firmware's page tables: identity-mapped,
everything writable+executable, CR0.WP off (ring 0 can write "read-only"
pages), and no kernel/user separation to prepare for. Milestone 2.4 must
install *our own* address space with enforced permissions — while firmware
services (including the final `ResetSystem`) keep working, because we do not
call ExitBootServices until M2.7.

## Decision

`arch/x86_64/paging.rs` builds one PML4 with two views and switches CR3 in
place:

```text
0x0000_0000_0000_0000 .. 0x0000_0000_FFFF_FFFF   identity view
    every firmware-described region, RW+X (see carve-out below)
0xFFFF_FFFF_8000_0000 .. 0xFFFF_FFFF_FFFF_FFFF   kernel direct map
    VA = phys + KERNEL_OFFSET, first 2 GiB, RW+NX
```

* **Our image window is mapped per PE section in BOTH views** (4 KiB pages,
  huge-page splits where needed): `.text` → R+X, `.rdata` → RO+NX, rest →
  RW+NX. Image base/size come from the UEFI Loaded Image Protocol; section
  bounds from parsing our own PE headers.
* **CR0.WP is set** — RO pages bind ring 0 itself; **EFER.NXE** is
  asserted on.
* **The identity view is RW+X outside our image window.** This is a
  deliberate, temporary firmware-compatibility carve-out: EDK2 executes
  from memory the UEFI map labels Reserved/BootServicesData (decompressed
  DXE volumes), so type-based NX there faulted live firmware code on first
  boot. We own nothing pre-EBS; W^X is fully enforced in the direct map
  from day one, and the identity view is torn down at M2.7 (roadmap 2.4's
  "tear down identity mapping" happens at ExitBootServices, when firmware
  no longer needs it — documented deviation, tracked in ROADMAP).
* **The LAPIC MMIO page is mapped explicitly** from IA32_APIC_BASE: it is
  GCD MMIO, absent from the UEFI memory map, and the interrupt EOI path
  writes it.
* Bulk ranges use 2 MiB pages; described regions above 4 GiB are skipped
  and counted (`skipped_high_regions`, logged — 1 on the reference VM,
  the 64-bit PCI window reservation).
* The CR3 switch needs no trampoline: RIP, stack, and all live data are
  identity addresses that the new tables keep valid, so init simply
  continues. Verified by read-back and by the M1/M2 suites passing
  *after* the switch in the same boot.

## Approaches considered

1. **Dual view (chosen)** — identity for firmware + direct map for the
   kernel, both live until EBS.
2. **Higher-half-only with early trampoline** — jump to the higher-half
   alias immediately and drop identity: cleaner end state, but breaks
   every firmware call (ResetSystem!) until M2.7. Rejected for now; the
   end state is exactly this, reached at EBS.
3. **Keep firmware tables until EBS** — defers all enforcement; violates
   milestone scope (WP/NX/W^X are testable now and shape the kernel's
   memory habits early). Rejected.

## Evidence (reference VM, QEMU TCG)

* `paging: address space live: cr3=0x100000` — 8 frames of tables total.
* Image window `[0x1dd0c000,0x1dd4b000)`, 5 sections.
* Higher-half call at `0xffffffff9dd11130` executed and returned the probe
  magic (`m2:test:vm_address_space`).
* Ring-0 write to RO `.text` alias: #PF `ec=0x3` (present+write),
  `cr2` = target (`vm_write_protect`).
* Fetch from NX data alias: #PF `ec=0x11` (present+instruction-fetch)
  (`vm_nx`).
* Firmware table facts learned the hard way: boot services HeaderSize 376
  = 24 + 44 slots, revision 0x20046 — **CloseEvent/CheckEvent occupy slots
  11/12, so HandleProtocol is slot 16**; a struct omitting them lands on
  ReinstallProtocolInterface (slot 14), which answers every protocol query
  with a plausible EFI_NOT_FOUND.

## Disadvantages accepted

* WB cache attributes everywhere (the devices we touch are port I/O;
  PAT-driven UC/WC arrives with real MMIO drivers).
* No global pages (CR4.PGE off), no 1 GiB pages, 2 GiB direct map,
  identity over-maps region edges ≤2 MiB (same described regions, RW+X).
* Single address space, single CPU — no kernel/user split yet (M3) and no
  ASIDs.

## Compiler/linker hazard (normative — see CODING-CONVENTIONS)

The boot image is a **non-PIE PE** (link base 0x140000000, loaded and
relocated elsewhere). `symbol + KERNEL_OFFSET` is a link-time constant, and
LLVM materializes such constants as 32-bit RIP-relative `lea` — the addend
wraps and the upper half of the higher-half address is silently lost (a
call through such a folded pointer landed at 0x9dd11120 → #PF). Rule:
compute higher-half addresses as `black_box(symbol_addr) + KERNEL_OFFSET`
so the offset is added by a runtime 64-bit instruction. This cost one boot
crash to find; the regression tests (`vm_*`) now cover it.

## Revisit triggers

* M2.7 ExitBootServices: identity view torn down; direct map becomes the
  only view; reclaim folds boot-services memory into the frame allocator
  and the mapping.
* Physical RAM beyond the 2 GiB direct-map window, or memory above 4 GiB
  on a target (grow the map or add a sparse VM region).
* M3 kernel/user split: user address space gets its own PML4 half
  (canonical low), PCID/ASID decision, and KPTI-style isolation review.
