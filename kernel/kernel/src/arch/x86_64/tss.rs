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
//! 2. **RSP0** — the ring-3→ring-0 stack pointer, consumed by M3's first
//!    syscall/user-mode transition. Deliberately left 0 until then: if
//!    anything ever tries a privilege-level stack switch before M3 wires
//!    this up, we want a loud fault, not silent use of a stale stack.
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
