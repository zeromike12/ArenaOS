//! 64-bit Task State Segment (SDM Vol. 3 §8.7).
//!
//! We never do hardware task switching (it does not exist in 64-bit mode).
//! The TSS is here for exactly two things the architecture still routes
//! through it:
//!
//! 1. **IST stacks** — dedicated, always-valid stacks for the exceptions
//!    that must survive a broken stack: #DF (8), NMI (2), #MC (18). Their
//!    IDT gates carry IST=1, so entry switches to `FAULT_STACK` no matter
//!    what state the interrupted stack was in.
//! 2. **RSP0** — the ring-3→ring-0 stack pointer for interrupts that land
//!    in user mode (M3.3, ADR-0014): the scheduler reprograms it to the
//!    incoming thread's kernel stack top on every context switch. It
//!    starts at 0 deliberately: any privilege-level stack switch before
//!    the scheduler wires it up faults loudly instead of silently using a
//!    stale stack.
//!
//! The TSS descriptor is a 16-byte system-segment pair living in GDT slots
//! 3–4 (selector 0x18); `init()` fills it and loads TR with `ltr`.

use super::{SyncCell, gdt};

/// 64-bit TSS layout, exactly as the SDM defines it. Note the 4-byte
/// alignment quirk: the 8-byte RSP/IST fields sit at 4-mod-8 offsets, so
/// the struct is `packed(4)` — read fields by copy, never by reference.
#[repr(C, packed(4))]
pub struct Tss {
    _reserved1: u32,
    /// Stack pointer for ring 3 → ring 0 transitions (wired in M3).
    pub rsp0: u64,
    _rsp1: u64,
    _rsp2: u64,
    _reserved2: u64,
    /// IST1..IST7; `ist[n]` is the IST(n+1) stack top (0 = unused).
    pub ist: [u64; 7],
    _reserved3: u64,
    _reserved4: u16,
    /// Offset of the I/O permission bitmap from the TSS base. We have no
    /// bitmap, so this points one past the end (SDM §19.5.2 legacy
    /// numbering: any value ≥ TSS size denies all user port I/O).
    pub iomap_base: u16,
}
const _: () = assert!(core::mem::size_of::<Tss>() == 104);

/// Dedicated exception stack for IST1 (#DF/NMI/#MC). 16 KiB is generous for
/// a diagnostic path that prints and halts; aligned 16 and sized in whole
/// 16-byte blocks so the stack top the CPU receives is 16-aligned.
const FAULT_STACK_BYTES: usize = 16 * 1024;

#[repr(C, align(16))]
struct FaultStack([u8; FAULT_STACK_BYTES]);

static FAULT_STACK: FaultStack = FaultStack([0; FAULT_STACK_BYTES]);

static TSS: SyncCell<Tss> = SyncCell::new(Tss {
    _reserved1: 0,
    rsp0: 0,
    _rsp1: 0,
    _rsp2: 0,
    _reserved2: u64::MAX, // reserved fields read as all-ones per SDM §8.7
    ist: [0; 7],
    _reserved3: u64::MAX,
    _reserved4: 0,
    iomap_base: 0,
});

/// The stack top programmed into IST1 (one past the end — stacks grow down).
pub fn ist1_stack_top() -> u64 {
    core::ptr::addr_of!(FAULT_STACK) as u64 + FAULT_STACK_BYTES as u64
}

/// Program RSP0 — the stack the CPU switches to for any ring-3 → ring-0
/// privilege-level transition (interrupts/exceptions landing in user mode;
/// `syscall` uses the GS scratch instead, ADR-0014). The scheduler calls
/// this on every switch to a stack-owning thread, together with
/// `syscall::set_cpu_kernel_stack`, so the two always agree.
///
/// # Safety
/// Ring 0, IF=0 (the scheduler's decision phase), `top` = one past the
/// last byte of the current thread's kernel stack, 16-byte aligned.
pub unsafe fn set_rsp0(top: u64) {
    // SAFETY: caller contract; packed(4) field written by copy (E0793).
    unsafe { core::ptr::addr_of_mut!((*TSS.get()).rsp0).write_unaligned(top) };
}

/// RSP0 as currently programmed (test evidence; copy-out of a packed
/// field).
pub fn read_back_rsp0() -> u64 {
    // SAFETY: read-only copy-out; single-CPU.
    unsafe { core::ptr::addr_of!((*TSS.get()).rsp0).read_unaligned() }
}

/// What the live TSS holds in IST1, read back out (test evidence). Copy-out
/// is required: the field lives in a `packed(4)` struct (E0793 otherwise).
pub fn read_back_ist1() -> u64 {
    // SAFETY: single-CPU boot context; plain copy of one field.
    unsafe { core::ptr::addr_of!((*TSS.get()).ist).read_unaligned()[0] }
}

/// Build the TSS, install its GDT descriptor, and load the task register.
///
/// # Safety
/// Ring 0, interrupts disabled, our GDT already live (the descriptor is
/// written into GDT slots 3–4, which `gdt::load()` left present-but-zero).
pub unsafe fn init() {
    unsafe {
        let tss = TSS.get();
        // RSP0 stays 0 until M3 (see module docs).
        (*tss).ist[0] = ist1_stack_top();
        (*tss).iomap_base = core::mem::size_of::<Tss>() as u16;

        // 16-byte system-segment descriptor (SDM Vol. 3 §8.7, Figure 8-4;
        // type values per Vol. 3 Table 3-2): type=0x9 (*available* 64-bit
        // TSS — LTR itself sets the busy bit, turning it into 0xB; encoding
        // 0xB up front is rejected with #GP), DPL=0, P=1, G=0 (byte limit).
        let base = tss as u64;
        let limit = (core::mem::size_of::<Tss>() - 1) as u64;
        let lo = (limit & 0xFFFF)
            | ((base & 0xFF_FFFF) << 16)
            | (0x9_u64 << 40) // type: 64-bit TSS, *available* (Table 3-2)
            | (1_u64 << 47) // P
            | (((limit >> 16) & 0xF) << 48)
            | (((base >> 24) & 0xFF) << 56);
        let hi = base >> 32;
        gdt::set_tss_descriptor(lo, hi);

        // Load the task register from our new descriptor.
        core::arch::asm!("ltr ax", in("ax") gdt::TSS_SELECTOR,
            options(nostack, preserves_flags));
    }
}

/// M2.7 kernel-view switch: move IST1 and the TSS descriptor base to
/// their `+offset` (kernel-view) aliases, reload GDTR at the GDT's own
/// `+offset` alias, and re-LTR. The descriptor must be re-encoded as
/// type 0x9 (*available*): the live one carries busy (0xB) since the
/// first LTR, and LTR rejects 0xB with #GP — the M2.1b lesson, applied
/// deliberately this time.
///
/// # Safety
/// Ring 0, IF=0, called while the dual-view tables are still live (both
/// aliases valid), `offset` = KERNEL_OFFSET, GDT already prepared for
/// reload (its content is position-independent).
pub unsafe fn relocate_for_kernel(offset: u64) {
    // SAFETY: caller contract; single-writer boot sequence.
    unsafe {
        let tss = TSS.get();
        // ist lives in a packed(4) struct: copy the array out, adjust,
        // copy it back (never borrow packed fields — E0793).
        let mut ist = core::ptr::addr_of!((*tss).ist).read_unaligned();
        ist[0] += offset;
        core::ptr::addr_of_mut!((*tss).ist).write_unaligned(ist);

        let base = (tss as u64) + offset;
        let limit = (core::mem::size_of::<Tss>() - 1) as u64;
        let lo = (limit & 0xFFFF)
            | ((base & 0xFF_FFFF) << 16)
            | (0x9_u64 << 40) // type: 64-bit TSS, *available* (LTR sets busy)
            | (1_u64 << 47) // P
            | (((limit >> 16) & 0xF) << 48)
            | (((base >> 24) & 0xFF) << 56);
        gdt::set_tss_descriptor(lo, base >> 32);
        gdt::reload_at(gdt::table_addr() + offset);
        core::arch::asm!("ltr ax", in("ax") gdt::TSS_SELECTOR,
            options(nostack, preserves_flags));
    }
}

/// Read back the task register selector (`str`) — M2 test evidence that the
/// CPU accepted our TSS.
pub fn read_tr() -> u16 {
    let tr: u16;
    // SAFETY: STR is a ring-0-readable store of the TR selector; no side
    // effects beyond writing our local.
    unsafe {
        core::arch::asm!("str ax", out("ax") tr, options(nostack, preserves_flags, nomem));
    }
    tr
}
