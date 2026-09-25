//! Minimal interrupt descriptor table — installed together with our GDT.
//!
//! WHY THIS EXISTS AT M1 (learned the hard way, see commit history):
//! EDK2 implements UEFI TPLs with the interrupt flag — every boot-service
//! call restores TPL_APPLICATION and executes `sti` on the way out. If we
//! swap the GDT without swapping the IDT, the next firmware timer interrupt
//! enters through a gate that selects the *firmware's* code segment (0x38),
//! which does not exist in our GDT → #GP in the firmware handler → #DF →
//! triple fault. The GDT and IDT must change hands together, atomically with
//! respect to interrupts (they are: we run IF=0 outside firmware calls, and
//! our own gates are in place before the first firmware call).
//!
//! Policy: vectors 0..31 get diagnostic stubs that log the vector, error
//! code, and interrupted context (plus CR2 for #PF) over serial, then halt
//! the machine (ADR-0005: crashes must produce useful diagnostics). Since
//! M2.1 a fault *armed as expected* by the test suite (`faults` module) is
//! instead recorded and recovered from — that is how the M2 suite proves
//! delivery, diagnosis, and resumption without dying. Vectors 32..255 share one absorb-and-EOI stub: external
//! interrupts arriving while we still borrow the firmware's devices are
//! acknowledged (8259 PIC EOI *and* LAPIC EOI — EDK2 runs this platform in
//! IOAPIC→LAPIC mode with the 8259s masked and unprogrammed; see the stub
//! comments) and ignored. The stub counts them so the M1 test suite can
//! *prove* real hardware interrupts flowed through our IDT.
//!
//! References: SDM Vol. 3 §6.10 (IDT, gate format), §6.12.1 (error codes),
//! §6.14.2 (LIDT/SIDT), §11 (APIC architecture; local-APIC EOI register),
//! Intel 8259A datasheet (EOI command).
//!
//! ABI notes: `x86-interrupt` is not stable Rust, so the stubs are written
//! with `global_asm!` and hand off to Rust via the target's win64/"C" ABI.
//! The interrupted RSP has *no* guaranteed alignment (the fault can hit any
//! instruction), so `exception_common` frames itself with RBP, saves all
//! caller-saved registers (the interrupted code may hold live values in
//! them — the handler is free to clobber them per ABI), and forces
//! RSP ≡ 0 mod 16 (`and rsp, -16`) before the `call`. The handler is an
//! ordinary `extern "C"` fn and may return (fault-recovery path), so the
//! stub unwinds exactly (`lea rsp, [rbp-72]`), restores the registers, and
//! `iretq`s whatever the (possibly rewritten) frame says.

use core::arch::global_asm;
use core::sync::atomic::{AtomicU64, Ordering};

use super::SyncCell;

/// Count of external interrupts absorbed by the ignore stub. The M1 test
/// suite enables IF briefly and requires this to advance — proof that real
/// device interrupts traverse our IDT, our stub, and an `iretq` intact.
static TIMER_IRQ_COUNT: AtomicU64 = AtomicU64::new(0);

pub fn absorbed_irq_count() -> u64 {
    TIMER_IRQ_COUNT.load(Ordering::Relaxed)
}

global_asm!(
    // ---- exception stubs: 32 slots at a fixed 16-byte stride -------------
    // Each stub body is at most 9 bytes (push imm8 ×2 = 4, jmp rel32 = 5),
    // so `.p2align 4` guarantees stub N lives exactly at base + 16*N. The
    // Rust side (`exc_stub_addr`) relies on this stride — audited above.
    ".section .text",
    ".p2align 4",
    ".globl arena_exc_stubs",
    "arena_exc_stubs:",
    ".macro EXC_STUB vec, has_err",
    ".p2align 4",
    ".if \\has_err == 0",
    "push 0", // dummy error code: uniform stack shape for the common path
    ".endif",
    "push \\vec",
    "jmp exception_common",
    ".endm",
    "EXC_STUB 0, 0",  // #DE Divide Error
    "EXC_STUB 1, 0",  // #DB Debug
    "EXC_STUB 2, 0",  // NMI
    "EXC_STUB 3, 0",  // #BP Breakpoint
    "EXC_STUB 4, 0",  // #OF Overflow
    "EXC_STUB 5, 0",  // #BR Bound Range
    "EXC_STUB 6, 0",  // #UD Invalid Opcode
    "EXC_STUB 7, 0",  // #NM Device Not Available
    "EXC_STUB 8, 1",  // #DF Double Fault (error code always 0)
    "EXC_STUB 9, 0",  // (reserved; legacy #CS)
    "EXC_STUB 10, 1", // #TS Invalid TSS
    "EXC_STUB 11, 1", // #NP Segment Not Present
    "EXC_STUB 12, 1", // #SS Stack-Segment Fault
    "EXC_STUB 13, 1", // #GP General Protection
    "EXC_STUB 14, 1", // #PF Page Fault (CR2 holds the address)
    "EXC_STUB 15, 0", // (reserved)
    "EXC_STUB 16, 0", // #MF x87 FPU Error
    "EXC_STUB 17, 1", // #AC Alignment Check
    "EXC_STUB 18, 0", // #MC Machine Check
    "EXC_STUB 19, 0", // #XF SIMD Floating Point
    "EXC_STUB 20, 0", // #CP Control Protection
    "EXC_STUB 21, 1", // (reserved by SDM but documented with EC in some refs — keep shape)
    "EXC_STUB 22, 0", // (reserved)
    "EXC_STUB 23, 0", // (reserved)
    "EXC_STUB 24, 0", // (reserved)
    "EXC_STUB 25, 0", // (reserved)
    "EXC_STUB 26, 0", // (reserved)
    "EXC_STUB 27, 0", // (reserved)
    "EXC_STUB 28, 0", // #HV Hypervisor Injection
    "EXC_STUB 29, 0", // #VC VMM Communication
    "EXC_STUB 30, 1", // #SX Security Exception
    "EXC_STUB 31, 0", // (reserved)
    // ---- external interrupt stub (vectors 32..255) ------------------------
    // Absorb and acknowledge BOTH interrupt controllers, then bump the
    // absorbed-counter and iretq. No registers touched besides rax (saved).
    //
    // On this platform (QEMU q35 + EDK2/OVMF) the firmware routes the 8254
    // PIT through IOAPIC → LAPIC (vector 32) and leaves the legacy 8259 pair
    // fully masked (IMR=0xFF/0xFF) with its vector base UNPROGRAMMED
    // (irq_base=0). So:
    //   - LAPIC EOI (32-bit zero store to the EOI register, offset 0xB0, at
    //     the architectural default APIC base 0xFEE00000) clears the
    //     in-service bit for the firmware-delivered vector; without it the
    //     LAPIC blocks every later vector at that priority and at most ONE
    //     tick is ever absorbed. A no-op when nothing is in service.
    //   - 8259 cascade EOI (OCW2 0x20 to slave port 0xA0 then master 0x20)
    //     covers platforms/handoffs where the tick arrives through the legacy
    //     PIC; a no-op when the 8259 has nothing in service. Do NOT unmask
    //     8259 IRQ0 to "help": with irq_base=0 the ExtINT acknowledge via
    //     LINT0 delivers vector 0x00 — a #DE through our gate 0 (observed
    //     live; see docs/TESTING.md).
    // The post-M2 interrupt layer will read IA32_APIC_BASE and own both
    // controllers properly after ExitBootServices; M1 keeps the default base.
    ".p2align 4",
    ".globl arena_irq_ignore_stub",
    "arena_irq_ignore_stub:",
    "push rax",
    "mov al, 0x20",
    "out 0xa0, al",
    "out 0x20, al",
    "mov rax, 0xfee000b0",
    "mov dword ptr [rax], 0",
    "lock inc qword ptr [rip + {timer_count}]",
    "pop rax",
    "iretq",
    // ---- common exception path --------------------------------------------
    // Stack here: [vector][error_code][RIP][CS][RFLAGS][RSP][SS]
    // Hand (vector, error_code, &mut frame) to Rust in rcx/rdx/r8 (win64
    // ABI). The handler is an ordinary Rust fn, so it may clobber ALL
    // caller-saved registers — and the interrupted code may legitimately
    // hold live values in any of them (an exception can land on any
    // instruction). So the stub saves/restores rax,rcx,rdx,rsi,rdi,r8-r11
    // around the call; callee-saved registers are the handler's job per
    // ABI. RBP frames the stub: exact unwind via `lea rsp,[rbp-72]` (skips
    // the alignment slack), and the handler can rewrite frame.rip (fault
    // recovery) without losing our way back. Alignment audit in header.
    ".p2align 4",
    "exception_common:",
    "push rbp",
    "mov rbp, rsp",
    "push rax",
    "push rcx",
    "push rdx",
    "push rsi",
    "push rdi",
    "push r8",
    "push r9",
    "push r10",
    "push r11",
    "and rsp, -16",
    "sub rsp, 32",
    "mov rcx, [rbp + 8]",
    "mov rdx, [rbp + 16]",
    "lea r8, [rbp + 24]",
    "call {handler}",
    "lea rsp, [rbp - 72]",
    "pop r11",
    "pop r10",
    "pop r9",
    "pop r8",
    "pop rdi",
    "pop rsi",
    "pop rdx",
    "pop rcx",
    "pop rax",
    "pop rbp",
    "add rsp, 16",
    "iretq",
    timer_count = sym TIMER_IRQ_COUNT,
    handler = sym arena_exception_handler,
);

/// Hardware-pushed interrupt frame (SDM Vol. 3 §6.12.1, Figure 6-9): the
/// 64-bit form always pushes SS/RSP even for same-privilege faults.
#[repr(C)]
pub struct InterruptStackFrame {
    pub rip: u64,
    pub cs: u64,
    pub rflags: u64,
    pub rsp: u64,
    pub ss: u64,
}

/// Rust half of the common exception path (see asm above). Either recovers a
/// fault the test suite armed as expected (`faults` module: record what was
/// observed, resume the test at its recorded address) or — the production
/// path — prints full diagnostics and halts the machine (ADR-0005).
extern "C" fn arena_exception_handler(
    vector: u64,
    error_code: u64,
    frame: *mut InterruptStackFrame,
) {
    // #PF puts the faulting linear address in CR2; capture it before
    // anything (logging, recovery bookkeeping) can fault again and clobber.
    let cr2 = if vector == 14 { super::read_cr2() } else { 0 };

    // Controlled fault injection (M2.1): armed expectation matching this
    // vector → record the measured delivery and resume the test. Everything
    // else falls through to diagnostics + halt.
    if let Some(resume) = super::faults::take_expected(vector) {
        super::faults::record(vector, error_code, cr2);
        // SAFETY: `frame` points at the hardware-pushed values on the live
        // stub stack; rewriting RIP is exactly the documented recovery
        // contract, and `resume` came from the fault site itself.
        unsafe { (*frame).rip = resume };
        return;
    }

    // SAFETY: `frame` points at the hardware-pushed values directly above
    // the still-live stub stack; we are on the interrupted stack, single CPU,
    // and the values are plain reads. Stack faults (#DF/#SS-class) re-faulting
    // here are covered by the IST1 stacks installed in M2.1 for #DF/NMI/#MC.
    let (rip, cs, rflags, rsp, ss) = unsafe {
        let f = &*frame;
        (f.rip, f.cs, f.rflags, f.rsp, f.ss)
    };
    crate::log::write_marker(format_args!(
        "[arena PANIC fault] vector={vector:#04x} ({}) error_code={error_code:#x} rip={rip:#x} cs={cs:#x} rflags={rflags:#x} rsp={rsp:#x} ss={ss:#x} cr2={cr2:#x}",
        vector_name(vector),
    ));
    crate::halt::halt_machine("cpu exception")
}

fn vector_name(v: u64) -> &'static str {
    match v {
        0 => "#DE Divide Error",
        1 => "#DB Debug",
        2 => "NMI",
        3 => "#BP Breakpoint",
        4 => "#OF Overflow",
        5 => "#BR Bound Range",
        6 => "#UD Invalid Opcode",
        7 => "#NM Device Not Available",
        8 => "#DF Double Fault",
        10 => "#TS Invalid TSS",
        11 => "#NP Segment Not Present",
        12 => "#SS Stack-Segment Fault",
        13 => "#GP General Protection",
        14 => "#PF Page Fault",
        16 => "#MF x87 FPU Error",
        17 => "#AC Alignment Check",
        18 => "#MC Machine Check",
        19 => "#XF SIMD Floating Point",
        20 => "#CP Control Protection",
        28 => "#HV Hypervisor Injection",
        29 => "#VC VMM Communication",
        30 => "#SX Security Exception",
        _ => "unknown/reserved",
    }
}

// ---------------------------------------------------------------------------
// IDT construction
// ---------------------------------------------------------------------------

/// 64-bit interrupt gate (SDM Vol. 3 §6.11, Table 3-2): P=1, DPL=0,
/// type=1110. Interrupt gates clear IF on entry — exactly our invariant
/// (handlers re-enable nothing; M2 decides policy per vector).
const GATE_INT64_PRESENT: u8 = 0x8E;

#[derive(Clone, Copy)]
#[repr(C, packed(2))]
struct IdtGate {
    /// bits 0..15 of the handler offset.
    offset_low: u16,
    /// Code-segment selector for the handler (must be a 64-bit CS).
    selector: u16,
    /// Byte 4 (SDM Vol. 3 Fig. 6-7): bits 0..2 = IST index (0 = no stack
    /// switch), bits 3..7 reserved/zero.
    ist: u8,
    /// Byte 5: bits 0..3 = gate type (0xE = 64-bit interrupt gate), bit 4 =
    /// S (must be 0), bits 5..6 = DPL, bit 7 = P.
    type_attr: u8,
    /// bits 16..31 of the handler offset.
    offset_mid: u16,
    /// bits 32..63 of the handler offset.
    offset_high: u32,
    /// Must be zero.
    reserved: u32,
}
const _: () = assert!(core::mem::size_of::<IdtGate>() == 16);

#[repr(C, packed(2))]
struct IdtDescriptor {
    limit: u16,
    base: u64,
}
const _: () = assert!(core::mem::size_of::<IdtDescriptor>() == 10);

const NUM_VECTORS: usize = 256;

static IDT: SyncCell<[IdtGate; NUM_VECTORS]> = SyncCell::new(
    [IdtGate {
        offset_low: 0,
        selector: 0,
        ist: 0,
        type_attr: 0,
        offset_mid: 0,
        offset_high: 0,
        reserved: 0,
    }; NUM_VECTORS],
);

static IDT_DESCRIPTOR: SyncCell<IdtDescriptor> = SyncCell::new(IdtDescriptor { limit: 0, base: 0 });

// Assembly stub symbols (defined in the global_asm! block above).
unsafe extern "C" {
    /// First of the 32 per-vector exception stubs, EXC_STUB_STRIDE apart.
    static arena_exc_stubs: u8;
    /// Shared absorb-and-EOI stub for vectors 32..255.
    static arena_irq_ignore_stub: u8;
}

/// Stride between consecutive exception stubs (asm guarantees `.p2align 4`
/// and bodies ≤ 9 bytes, so each stub fits its 16-byte slot).
const EXC_STUB_STRIDE: usize = 16;

fn exc_stub_addr(vector: usize) -> u64 {
    debug_assert!(vector < 32);
    // Address arithmetic within the 32-stub block is in-bounds by the
    // stride audit at the asm site (`.p2align 4`, ≤9-byte bodies).
    core::ptr::addr_of!(arena_exc_stubs) as u64 + (vector * EXC_STUB_STRIDE) as u64
}

fn irq_stub_addr() -> u64 {
    core::ptr::addr_of!(arena_irq_ignore_stub) as u64
}

/// IST index per vector (SDM Vol. 3 §6.14.5): the exceptions that must
/// survive a corrupted/unmapped stack run on the dedicated IST1 stack from
/// `tss.rs`. 0 = no stack switch (normal behavior).
pub const fn ist_for(vector: usize) -> u8 {
    match vector {
        2 | 8 | 18 => 1, // NMI, #DF, #MC
        _ => 0,
    }
}

fn make_gate(handler: u64, ist: u8) -> IdtGate {
    IdtGate {
        offset_low: (handler & 0xFFFF) as u16,
        selector: super::gdt::KERNEL_CODE_SELECTOR,
        ist,
        type_attr: GATE_INT64_PRESENT,
        offset_mid: ((handler >> 16) & 0xFFFF) as u16,
        offset_high: (handler >> 32) as u32,
        reserved: 0,
    }
}

/// Build the 256-entry IDT and load it with LIDT.
///
/// # Safety
/// Ring 0, IF=0 (caller guarantees: installed right after the GDT swap in
/// `main`, which runs with interrupts masked), stubs reachable in the loaded
/// image.
pub unsafe fn init() {
    // SAFETY: single-writer boot-time init (SyncCell contract), caller
    // guarantees IF=0 so no interrupt can race table construction.
    unsafe {
        let idt = IDT.get();
        for vector in 0..NUM_VECTORS {
            let handler = if vector < 32 {
                exc_stub_addr(vector)
            } else {
                irq_stub_addr()
            };
            // Write through the raw pointer; IdtGate is packed, so assign
            // the whole struct rather than borrowing fields.
            core::ptr::write(&mut (*idt)[vector], make_gate(handler, ist_for(vector)));
        }
        IDT_DESCRIPTOR.get().write(IdtDescriptor {
            limit: (core::mem::size_of::<[IdtGate; NUM_VECTORS]>() - 1) as u16,
            base: idt as u64,
        });
        core::arch::asm!("lidt [{}]", in(reg) IDT_DESCRIPTOR.get(),
            options(readonly, nostack, preserves_flags));
    }
}

/// Audit the live IDT (located via `sidt`): count gates whose selector,
/// attribute/IST bytes, or handler offset deviate from the intended encoding
/// (KERNEL_CODE_SELECTOR, GATE_INT64_PRESENT, IST = `ist_for(vector)`,
/// offset = the stub for that vector). Returns (bad_selectors, bad_attrs, bad_offsets). Used by the
/// M1 suite as independent evidence that the table the CPU will walk is the
/// table we meant to build.
pub fn audit_gates() -> (usize, usize, usize) {
    let (base_addr, _) = read_idtr();
    let base = base_addr as *const u64;
    let mut bad_sel = 0usize;
    let mut bad_attr = 0usize;
    let mut bad_off = 0usize;
    for v in 0..NUM_VECTORS {
        // SAFETY: base is the live IDT (256 16-byte gates) per `sidt`;
        // read-only, IF-agnostic (table reads are not interruptible state).
        let lo = unsafe { base.add(v * 2).read_unaligned() };
        let hi = unsafe { base.add(v * 2 + 1).read_unaligned() };
        let sel = (lo >> 16) & 0xFFFF;
        let ist_byte = (lo >> 32) & 0xFF;
        let attrs = (lo >> 40) & 0xFF;
        let offset = (lo & 0xFFFF) | (((lo >> 48) & 0xFFFF) << 16) | (hi << 32);
        let want = if v < 32 {
            exc_stub_addr(v)
        } else {
            irq_stub_addr()
        };
        if sel != super::gdt::KERNEL_CODE_SELECTOR as u64 {
            bad_sel += 1;
        }
        if attrs != GATE_INT64_PRESENT as u64 || ist_byte != u64::from(ist_for(v)) {
            bad_attr += 1;
        }
        if offset != want {
            bad_off += 1;
        }
    }
    (bad_sel, bad_attr, bad_off)
}

/// Read back the IDTR (base, limit) with `sidt` — M1 test evidence that the
/// CPU accepted our table.
pub fn read_idtr() -> (u64, u16) {
    let mut desc = IdtDescriptor { limit: 0, base: 0 };
    // SAFETY: SIDT writes 10 bytes to a valid stack object; no side effects;
    // legal at ring 0.
    unsafe {
        core::arch::asm!("sidt [{}]", in(reg) &mut desc, options(nostack, preserves_flags));
    }
    (desc.base, desc.limit)
}

/// What `sidt` must report after [`init`].
pub fn expected_idt() -> (u64, u16) {
    (
        IDT.get() as u64,
        (core::mem::size_of::<[IdtGate; NUM_VECTORS]>() - 1) as u16,
    )
}
