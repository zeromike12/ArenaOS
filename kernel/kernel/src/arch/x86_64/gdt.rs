//! Our own GDT, replacing the firmware's (ADR-0003 step 1).
//!
//! UEFI leaves us with *its* GDT, whose layout is firmware-specific. We
//! install a known-good 64-bit GDT so the kernel controls its own segment
//! selectors — a prerequisite for the ring-0/ring-3 split in Milestone 3
//! (user entries join this table then; the TSS descriptor already does in
//! M2.1, slots 3–4, selector 0x18 — the kernel selectors below are chosen
//! so they stay stable as the table grows).
//!
//! References: SDM Vol. 3 §3.4.5 (segment descriptors), §6.2.3 (GDT),
//! §6.8 (loading GDTR), §6.14 (64-bit mode segment behavior: base/limit are
//! ignored for data segments; CS.L must be 1, CS.D must be 0).

use super::SyncCell;

/// Segment descriptor entries (raw 64-bit encodings, SDM Vol. 3 §3.4.5).
///
/// Layout (bits): 0-15 limit[0:15], 16-39 base[0:23], 40-47 type/attr,
/// 48-51 limit[16:19], 52-55 flags, 56-63 base[24:31].
/// Long-mode code: L(bit 21 of flags field)=1, D=0 → flags nibble 0xA.
const GDT_NULL: u64 = 0;
/// P=1, DPL=0, S=1, type=execute/read/accessed (0xA), L=1, D=0, G=1, limit=0xFFFFF.
const GDT_KERNEL_CODE: u64 = 0x00AF_9A00_0000_FFFF;
/// P=1, DPL=0, S=1, type=read/write/accessed (0x2), D/B=1, G=1, limit=0xFFFFF.
const GDT_KERNEL_DATA: u64 = 0x00CF_9200_0000_FFFF;

pub const KERNEL_CODE_SELECTOR: u16 = 0x08;
pub const KERNEL_DATA_SELECTOR: u16 = 0x10;
/// 16-byte TSS system-segment descriptor (GDT slots 3–4), filled and loaded
/// by `tss::init()` in M2.1.
pub const TSS_SELECTOR: u16 = 0x18;

/// The boot GDT itself. Slots 0–2 are fixed at build time; slots 3–4 are
/// the TSS descriptor pair, written by `set_tss_descriptor` during
/// `tss::init()` (live-table write is safe: slot unused until `ltr`, IF=0,
/// single CPU). `sgdt` read-back in the M1 test suite verifies the CPU
/// actually accepted the table.
#[repr(C, align(16))]
struct Gdt {
    entries: [u64; 5],
}

static GDT: SyncCell<Gdt> = SyncCell::new(Gdt {
    entries: [GDT_NULL, GDT_KERNEL_CODE, GDT_KERNEL_DATA, 0, 0],
});

/// Write the 16-byte TSS system-segment descriptor into GDT slots 3–4.
///
/// # Safety
/// Our GDT must be live, interrupts disabled, single CPU (the only caller
/// is `tss::init()` before any IST gate can fire).
pub unsafe fn set_tss_descriptor(lo: u64, hi: u64) {
    unsafe {
        let gdt = GDT.get();
        core::ptr::addr_of_mut!((*gdt).entries[3]).write(lo);
        core::ptr::addr_of_mut!((*gdt).entries[4]).write(hi);
    }
}

/// GDTR image: 16-bit limit + 64-bit base, packed to exactly 10 bytes
/// (SDM Vol. 3 §6.14.1 — SGDT/LGDT operand format).
#[repr(C, packed(2))]
struct GdtDescriptor {
    limit: u16,
    base: u64,
}
const _: () = assert!(core::mem::size_of::<GdtDescriptor>() == 10);

static GDT_DESCRIPTOR: SyncCell<GdtDescriptor> = SyncCell::new(GdtDescriptor { limit: 0, base: 0 });

/// Install our GDT and reload all segment registers.
///
/// After `lgdt`, CS still *references the firmware's* descriptor slot (the
/// descriptor cache was loaded from their table). Even though the values look
/// compatible, we reload CS through a far return to our own 64-bit code
/// selector before anything can alias the old table. Data segments are then
/// reloaded with plain `mov`s.
///
/// # Safety
/// Must run at ring 0 with interrupts disabled (guaranteed: `cli()` runs at
/// kernel entry) and with the image identity-mapped (UEFI guarantees this for
/// loaded images before ExitBootServices).
pub unsafe fn load() {
    unsafe {
        // Write the descriptor image (base of our static GDT, byte limit).
        // Single writer, pre-concurrency — see SyncCell contract.
        GDT_DESCRIPTOR.get().write(GdtDescriptor {
            limit: (core::mem::size_of::<Gdt>() - 1) as u16,
            base: GDT.get() as u64,
        });

        // LGDT from the 10-byte descriptor image.
        core::arch::asm!("lgdt [{}]", in(reg) GDT_DESCRIPTOR.get(),
            options(readonly, nostack, preserves_flags));

        // Far-return to reload CS: `retfq` pops RIP (top of stack) then CS,
        // so CS is pushed first, then the address of local label `2:`.
        core::arch::asm!(
            "push {cs_sel}",
            "lea {tmp}, [rip + 2f]",
            "push {tmp}",
            "retfq",
            "2:",
            cs_sel = in(reg) KERNEL_CODE_SELECTOR as u64,
            tmp = lateout(reg) _,
            options(preserves_flags),
        );

        // Reload data segments via rax (ax is what segment movs consume).
        core::arch::asm!(
            "mov rax, {sel}",
            "mov ds, ax",
            "mov es, ax",
            "mov fs, ax",
            "mov gs, ax",
            "mov ss, ax",
            sel = in(reg) KERNEL_DATA_SELECTOR as u64,
            out("rax") _,
            options(nostack, preserves_flags),
        );
    }
}

/// Read back the GDTR (base, limit) with `sgdt` — used by the M1 test to
/// prove the CPU accepted *our* table, not the firmware's.
pub fn read_gdtr() -> (u64 /*base*/, u16 /*limit*/) {
    let mut desc = GdtDescriptor { limit: 0, base: 0 };
    // SAFETY: SGDT writes exactly 10 bytes to a valid, aligned-enough stack
    // object (packed(2) struct); the instruction has no other side effects
    // and is legal at ring 0.
    unsafe {
        core::arch::asm!("sgdt [{}]", in(reg) &mut desc, options(nostack, preserves_flags));
    }
    (desc.base, desc.limit)
}

/// What `sgdt` should report after `load()` — the M1 test compares against
/// this instead of trusting `load()` blindly.
pub fn expected_gdt() -> (u64, u16) {
    (GDT.get() as u64, (core::mem::size_of::<Gdt>() - 1) as u16)
}
