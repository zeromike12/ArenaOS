//! Interrupt controllers: the kernel-owned IOAPIC/LAPIC register window.
//!
//! Through M2.6 the *firmware* owned this chain: OVMF routes the 8254 PIT
//! through IOAPIC pin 0 → LAPIC vector 32 and leaves the legacy 8259 pair
//! masked. `ExitBootServices` ends that ownership — measured in the M2.7
//! bring-up: 160 PIT deliveries before the handover, zero after, even with
//! the PIT itself re-armed (EDK2's timer stack tears its IOAPIC routing
//! down at EBS). The kernel therefore re-establishes the full chain here:
//! IOAPIC RTE0 → vector 32 → LAPIC (software-enabled) → IDT.
//!
//! All accessors take the register-window base as a *virtual* address: the
//! caller picks the alias that matches the live page tables (identity
//! pre-handover; under the kernel view, `paging::mmio_alias_va(phys)` for
//! the above-2 GiB windows — the kernel-half alias rule every address
//! space inherits; see that function for why `phys + KERNEL_OFFSET` wraps
//! out of the kernel half there). Register-window protocol: write the index to REGSEL, then
//! read/write REGWIN; both accesses must not be interleaved with other
//! IOAPIC users (single CPU, IF=0 — boot contract).

/// IOAPIC MMIO base on this platform (q35-fixed; GCD MMIO, absent from
/// the memory map — build_kernel_view aliases it explicitly). The HPET
/// below shares the IOAPIC's 2 MiB alias page by q35 layout luck.
pub const IOAPIC_PHYS: u64 = 0xFEC0_0000;

/// HPET MMIO base (q35-fixed; ACPI-described, memory-map-absent like the
/// other interrupt fabric).
pub const HPET_PHYS: u64 = 0xFED0_0000;
/// HPET general configuration register.
pub const HPET_GEN_CONF: u64 = 0x10;
/// GEN_CONF bit 1: legacy replacement mode — HPET timers 0/1 take over
/// IRQ0/IRQ8 and QEMU *suppresses the 8254's IRQ output entirely*
/// (i8254.c `pit_irq_control`). The kernel runs the PIT as its tick
/// source, so this bit must be off.
pub const HPET_LEGACY_ENABLE: u32 = 1 << 1;

/// The vector the PIT is delivered on — the first slot of the IDT's
/// absorbed-IRQ range (32..255), matching the firmware's pre-handover
/// routing so both eras share one chain.
pub const PIT_VECTOR: u8 = 32;

/// IOAPIC register-window select (MMIO offset 0x00).
const REGSEL: u64 = 0x00;
/// IOAPIC register window (MMIO offset 0x10).
const REGWIN: u64 = 0x10;

/// The IOAPIC pin carrying ISA IRQ0 (the 8254 PIT) on PC platforms: pin
/// **2**, per the ACPI interrupt-source-override convention — real
/// chipsets wire the PIT there, QEMU's ioapic_set_irq hard-remaps
/// "ISA IRQ0 → pin 2" to match, and firmware used this pin pre-handover
/// (measured at EBS: RTE2 held vector 32 masked; RTE0 was never used).
/// M3+ reads the MADT override table instead of hardcoding.
pub const PIT_IOAPIC_PIN: u8 = 2;

/// LAPIC spurious-vector register (MMIO offset 0xF0); bit 8 is the
/// software-enable flag.
const LAPIC_SVR: u64 = 0xF0;
const LAPIC_SVR_ENABLE: u32 = 1 << 8;

/// LAPIC task-priority register — blocks delivery below its class.
pub const LAPIC_TPR: u64 = 0x80;
/// LAPIC processor-priority (read-only mirror of the effective floor:
/// max(TPR, highest in-service vector)).
pub const LAPIC_PPR: u64 = 0xA0;
/// LAPIC EOI register (write 0; clears the highest in-service bit).
pub const LAPIC_EOI_REG: u64 = 0xB0;
/// LAPIC ID register (bits 31:24 hold the APIC ID in xAPIC mode).
pub const LAPIC_ID: u64 = 0x20;
/// LAPIC version register (bits 7:0 = version, 23:16 = max LVT entry).
pub const LAPIC_VERSION: u64 = 0x30;
/// LVT LINT0 pin — how local interrupt pin 0 delivers (ExtINT vs fixed
/// vs masked). Firmware timer stacks sometimes wire the 8254 through it.
pub const LAPIC_LVT_LINT0: u64 = 0x350;
/// LVT LINT1 pin (NMI on PC platforms).
pub const LAPIC_LVT_LINT1: u64 = 0x360;
/// LVT timer entry (local APIC timer, if firmware used one).
pub const LAPIC_LVT_TIMER: u64 = 0x320;
/// Interrupt-command register (low): fire an IPI per the encoded fields.
pub const LAPIC_ICR_LO: u64 = 0x300;
/// Interrupt-command register (high): destination field for the ICR.
pub const LAPIC_ICR_HI: u64 = 0x310;
/// In-service bits for vectors 32..63 (our absorbed-IRQ range starts here).
pub const LAPIC_ISR1: u64 = 0x110;
/// Interrupt-request bits for vectors 32..63 (pending, not yet delivered).
pub const LAPIC_IRR1: u64 = 0x210;

/// Read a 32-bit HPET register (the 64-bit-wide registers' low half —
/// all registers this module touches live below 2^32).
///
/// # Safety
/// `hpet_va` must be a mapped alias of the HPET MMIO page.
pub unsafe fn hpet_read(hpet_va: u64, reg: u64) -> u32 {
    // SAFETY: caller contract; volatile MMIO read.
    unsafe { ((hpet_va + reg) as *const u32).read_volatile() }
}

/// Write a 32-bit HPET register (low half; preserves the untouched high
/// half by definition of a 32-bit access).
///
/// # Safety
/// As [`hpet_read`].
pub unsafe fn hpet_write(hpet_va: u64, reg: u64, value: u32) {
    // SAFETY: caller contract; volatile MMIO write.
    unsafe { ((hpet_va + reg) as *mut u32).write_volatile(value) };
}

/// Read a 32-bit LAPIC register.
///
/// # Safety
/// `lapic_va` must be a mapped alias of the LAPIC MMIO page.
pub unsafe fn lapic_read(lapic_va: u64, reg: u64) -> u32 {
    // SAFETY: caller contract; volatile MMIO read.
    unsafe { ((lapic_va + reg) as *const u32).read_volatile() }
}

/// Write a 32-bit LAPIC register.
///
/// # Safety
/// As [`lapic_read`].
pub unsafe fn lapic_write(lapic_va: u64, reg: u64, value: u32) {
    // SAFETY: caller contract; volatile MMIO write.
    unsafe { ((lapic_va + reg) as *mut u32).write_volatile(value) };
}

/// Read one IOAPIC register through the window at `ioapic_va`.
///
/// # Safety
/// `ioapic_va` must be a mapped alias of the IOAPIC MMIO page; single
/// CPU with IF=0 (no interleaved window users).
pub unsafe fn ioapic_read(ioapic_va: u64, index: u32) -> u32 {
    // SAFETY: caller contract; volatile MMIO accesses, register-only side
    // effects.
    unsafe {
        ((ioapic_va + REGSEL) as *mut u32).write_volatile(index);
        ((ioapic_va + REGWIN) as *mut u32).read_volatile()
    }
}

/// Write one IOAPIC register through the window at `ioapic_va`.
///
/// # Safety
/// As [`ioapic_read`].
pub unsafe fn ioapic_write(ioapic_va: u64, index: u32, value: u32) {
    // SAFETY: caller contract; volatile MMIO accesses.
    unsafe {
        ((ioapic_va + REGSEL) as *mut u32).write_volatile(index);
        ((ioapic_va + REGWIN) as *mut u32).write_volatile(value);
    }
}

/// Redirection-table register indices for `pin` (RTEs start at 0x10,
/// two 32-bit registers each).
pub const fn rte_lo_index(pin: u8) -> u32 {
    0x10 + pin as u32 * 2
}
pub const fn rte_hi_index(pin: u8) -> u32 {
    0x10 + pin as u32 * 2 + 1
}

/// Route IOAPIC `pin` to LAPIC `vector` on the BSP: fixed delivery,
/// physical destination, edge-triggered, unmasked. High word first so
/// the entry is never transiently unmasked with a stale low word (the
/// low word carries the mask bit).
///
/// # Safety
/// As [`ioapic_read`]; `vector` must be a slot the IDT owns (32..255).
pub unsafe fn route_pin_to_vector(ioapic_va: u64, pin: u8, vector: u8) {
    debug_assert!(vector >= 32);
    // SAFETY: caller contract.
    unsafe {
        ioapic_write(ioapic_va, rte_hi_index(pin), 0); // destination = APIC ID 0 (BSP)
        ioapic_write(ioapic_va, rte_lo_index(pin), u32::from(vector)); // fixed/phys/edge, mask=0
    }
}

/// Read the LAPIC spurious-vector register (diagnostics: software-enable
/// bit, spurious vector).
///
/// # Safety
/// `lapic_va` must be a mapped alias of the LAPIC MMIO page.
pub unsafe fn lapic_svr(lapic_va: u64) -> u32 {
    // SAFETY: caller contract; volatile MMIO read.
    unsafe { ((lapic_va + LAPIC_SVR) as *mut u32).read_volatile() }
}

/// Ensure the LAPIC is software-enabled (SVR bit 8), preserving the
/// spurious-vector field. Idempotent; firmware normally leaves this set,
/// but the kernel verifies rather than assumes at the handover.
///
/// # Safety
/// As [`lapic_svr`].
pub unsafe fn lapic_enable(lapic_va: u64) -> u32 {
    // SAFETY: caller contract.
    unsafe {
        let svr = lapic_svr(lapic_va);
        if svr & LAPIC_SVR_ENABLE == 0 {
            ((lapic_va + LAPIC_SVR) as *mut u32).write_volatile(svr | LAPIC_SVR_ENABLE);
            lapic_svr(lapic_va)
        } else {
            svr
        }
    }
}
