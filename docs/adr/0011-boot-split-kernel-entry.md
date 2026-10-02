# ADR-0011: Boot split — ExitBootServices, kernel entry, and the reclaimed timer chain

Status: accepted (M2.7, 2026-09)

## Problem

Milestones 2.1–2.6 ran *inside* the boot stage: UEFI boot services were
alive, the dual-view tables of ADR-0008 (identity for firmware + kernel
direct map) were current, and the interrupt controllers were still in the
state firmware left them in for *its own* use. Milestone 2.7 turns the
boot stage and the kernel proper into separate crates and crosses the
point of no return:

* `ExitBootServices()` invalidates all boot-services memory and firmware
  stops servicing timer interrupts for us. The identity view must die at
  the same instant (ADR-0008 deferred exactly this tear-down to here).
* Our PE image is loaded by firmware at whatever base it chose, but the
  kernel view maps it at the link base `0x140000000` (`+KERNEL_OFFSET`
  direct map). Every absolute address baked into the image — IDT/GDT/TSS
  descriptors, vtables, GOT entries — points at the *loaded* addresses.
* `ResetSystem` is a *runtime* service and survives EBS, but its code and
  data live in firmware regions the kernel view does not map. The
  shutdown path that every milestone harness depends on (the clean-halt
  discriminator `halting via UEFI ResetSystem(shutdown)`) must still work
  from a kernel-only address space.
* The timer chain (PIT → IOAPIC → LAPIC) that M2.2/M2.6 drove must be
  *reclaimed*: firmware reprograms and masks interrupt state at EBS, and
  the kernel inherits a machine that no longer delivers timer ticks.
* The transition must be reboot-stable: one green boot proves
  correctness, 100 green boots prove we are not winning a race.

## Decision

**Two crates, one handoff record.** `kernel/boot` is the UEFI
application (stages 2.1–2.6 plus EBS); `kernel/kernel` is the kernel
proper. Boot fills `BootInfo` (`entry.rs`: magic `ARENK1`, ABI version,
map key, `fw_cr3`/`dual_view_cr3`/`kernel_cr3`, image base/size, TSC and
tick rates, carried M1/M2 tallies, reconciliation counters) and hands
control over through an **address-space switch trampoline**: boot calls
the trampoline at its kernel-view alias while the dual view is still
live; inside, CR3 switches to the kernel-only tables, RSP is lifted to
its direct-map alias, and `kmain` is entered with the kernel-view
`BootInfo` pointer. From that instruction the identity view of RAM is
gone. `kmain` validates the record as a real ABI check (magic, version,
live CR3/CR0/EFER read-backs against the record) — a corrupted handoff
halts with diagnostics instead of limping on.

**Base relocations are replayed at the kernel view.** The PE
`.reloc` directory (DataDirectory[5]) is walked and every DIR64 entry
(101 of them: descriptors, vtables, statics) rewritten with
`+KERNEL_OFFSET` through the identity MMIO/RAM alias, with CR0.WP
toggled around the writes. Interrupts are off (`cli`) across the whole
relocation window — a partially relocated IDT must never take a fault —
and the IDT/GDT/TSS are read back afterwards to prove the switch.

**Shutdown runs from a farewell island.** `halt.rs` keeps a tiny
assembly island (`arena_reset_island`) at an identity address that is
mapped executable in *every* view the kernel may run under (firmware
tables, dual view, kernel view — the kernel view maps the island page by
design). `reset_shutdown()` converts the island to its physical address,
enters it with firmware's CR3 (`BootInfo.fw_cr3`, captured at entry) and
an identity stack, and the island calls the recorded `ResetSystem`
function pointer in firmware's own world — its tables, its mappings, its
stack. If no `fw_cr3` was captured (failure earlier than boot step 2),
a fallback calls `ResetSystem` directly under the still-live firmware or
dual-view tables (the M2.6-proven path).

**The timer chain is reclaimed explicitly, and the PIT lives on IOAPIC
pin 2 — not pin 0.** This was the milestone's hard-won finding:

* EDK2 at EBS leaves: IOAPIC **RTE2** masked and vector-zeroed, the 8259
  IMRs fully masked, LINT0 as ExtINT (legacy PIC path), LAPIC SVR
  enabled, and the **HPET fully disabled** (GEN_CONF reads 0 — it never
  engages legacy-replacement mode, so it is *not* an IRQ suppressor).
* QEMU's `hw/intc/ioapic.c` (`ioapic_set_irq`) applies the classic PC
  convention — the same one MADT interrupt-source-override entries
  encode: *ISA IRQs map to GSI 1:1 except IRQ0, which maps to GSI 2.*
  The PIT's ISA IRQ0 therefore arrives on **IOAPIC pin 2**. Routing
  RTE0 hits a pin nothing is wired to; masked pins on edge-triggered
  RTEs drop edges entirely, which is why firmware's masked RTE2
  silently ate every tick.
* Reclaim sequence in `kmain` (`drivers/intc.rs` + `drivers/pit.rs`):
  re-arm the PIT channel 0 at 100 Hz mode 3 → clear HPET
  `GEN_CONF.LEGACY_ENABLE` (cheap insurance, read-back logged) → write
  IOAPIC **pin 2** RTE to `PIT_VECTOR` (32), fixed delivery, physical
  destination, edge-triggered, unmasked (old RTE read back and logged
  first) → LAPIC TPR=0, spurious EOI, SVR enabled → `sti` and absorb.
  `kernel_irq_live` then requires **2 real ticks** through the relocated
  IDT with EOI via the kernel-alias LAPIC MMIO (IRR/ISR/PPR read-backs
  prove the delivery path, tick counter proves re-delivery), plus a PIT
  count-delta check proving channel 0 is actually counting.

**Stability gate.** `tools/stability_loop.sh` boots the prebuilt ESP
image N times (default 100), fresh NVRAM each run, requiring on every
boot: `m2: RESULT PASS (21/21)`, the canonical halt line, no `PANIC`,
QEMU exit 0, under a per-boot timeout. Milestone 2.7 is not done until
the loop is green.

## Approaches considered

1. **Full split at EBS with trampoline + relocation replay + farewell
   island (chosen).** Pays real complexity once, and lands exactly the
   end state every later milestone needs: kernel-only address space,
   higher-half image, firmware reachable only through a deliberate,
   documented bridge.
2. **Keep the dual view forever (never tear down identity).** Zero
   trampoline work, but the RW+X firmware carve-out of ADR-0008 would
   outlive its justification, W^X enforcement stays incomplete, and user
   space (M3+) could never be separated from a kernel that is also
   identity-mapped. Rejected: violates the address-space plan.
3. **Link the kernel at its loaded base / identity-linked kernel.**
   Removes relocation replay, but firmware chooses the load base — the
   kernel would have no stable higher-half layout, and every future
   component (modules, user mappings) would inherit that instability.
   Rejected.
4. **Call `ResetSystem` directly under the kernel view.** Impossible:
   the runtime services' code/data are unmapped by design after the
   identity tear-down. The island is the minimal bridge (a few dozen
   bytes of assembly, one deliberately mapped page).
5. **Switch to the LAPIC timer instead of reclaiming the PIT chain.**
   Attractive long-term (it is the scheduler-tick source for M3 on real
   hardware), but it would have *masked* the pin-2 finding rather than
   explained it, and M2.2's calibrated PIT path is the tested one.
   Reclaimed the PIT now; LAPIC timer is an M3 candidate with this ADR
   as the map of the terrain.
6. **Debugging theories eliminated by live evidence** (recorded so they
   are never re-walked): HPET legacy replacement as IRQ suppressor
   (GEN_CONF=0); an IOAPIC "global enable" register (does not exist —
   index 1 is the VERSION register; only per-RTE masking); port 0x61
   gate bit (QEMU mode-3 OUT ignores the gate); stuck TPR/ISR/PPR
   (read-backs clean); dead LAPIC receive path (self-IPI lands in IRR).

## Consequences

* Accepted downsides: the handoff has moving parts (trampoline,
  relocation replay, island) that must stay correct across toolchain and
  linker changes — guarded by `kmain`'s ABI/state validation, IDT/GDT
  read-backs, the 21-test suite, and the 100-boot loop. The farewell
  island is assembly in a repo that prefers Rust; it is deliberately
  tiny, commented, and the only way back into firmware's world.
* The identity-view carve-out of ADR-0008 is now fully retired at EBS,
  as promised there ("documented deviation, tracked in ROADMAP").
* The boot stack's physical pages stay reserved (firmware region) and
  serve as the entry stack until M3 gives tasks their own.
* Future multiprocessor entry (M5+) will replay a subset of this: no
  firmware involvement, but the same IOAPIC pin-2 map, LAPIC setup, and
  vector allocation (`PIT_VECTOR` = 32 is now the first allocated
  vector — the interrupt-vector budget grows from here).
* Every milestone from here on ships as a **GitHub release with QEMU
  artifacts** (ESP image + tested EDK2 firmware pair + run instructions)
  so the exact tested build can be run outside this repo — see
  `docs/RUNNING.md` and `tools/release.sh`.
