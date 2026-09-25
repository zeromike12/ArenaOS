//! Architecture quarantine (CODING-CONVENTIONS: arch-specific code lives only
//! under `arch/`). x86-64 CPU state access: control registers, MSRs, CPUID,
//! port I/O.

pub mod faults;
pub mod gdt;
pub mod idt;
pub mod paging;
pub mod tss;

use core::arch::x86_64::__cpuid;
use core::fmt;

pub(crate) use crate::sync::SyncCell;

/// Disable maskable interrupts (CLI). Safe: at boot we own the CPU; UEFI
/// requires single-CPU execution and we keep interrupts off until the kernel
/// installs an IDT (M2).
pub fn cli() {
    // SAFETY: CLI is safe to execute at any privilege ≥ ring 3 in our boot
    // context; it cannot fault and only affects the interrupt flag.
    unsafe { core::arch::asm!("cli", options(nostack, preserves_flags, nomem)) };
}

/// Enable maskable interrupts (STI). Used by exactly one M1 test (proving
/// hardware interrupts traverse our IDT) and never by production boot paths.
pub fn sti() {
    // SAFETY: legal at ring 0; effect is intended and bounded by the caller
    // (the test re-masks immediately after).
    unsafe { core::arch::asm!("sti", options(nostack, preserves_flags, nomem)) };
}

/// x86 I/O port write (8-bit).
///
/// # Safety
/// Writing to an arbitrary port can have arbitrary device side effects; the
/// caller must know the port's semantics (our only user is the 16550 UART
/// driver, which owns COM1's register block).
pub unsafe fn outb(port: u16, value: u8) {
    // SAFETY: forwarded to caller contract.
    unsafe {
        core::arch::asm!("out dx, al", in("dx") port, in("al") value, options(nostack, preserves_flags, nomem))
    };
}

/// x86 I/O port read (8-bit).
///
/// # Safety
/// As [`outb`]: reading some ports has side effects (clear-on-read status
/// registers); the caller must own the port's semantics.
pub unsafe fn inb(port: u16) -> u8 {
    let value: u8;
    // SAFETY: forwarded to caller contract.
    unsafe {
        core::arch::asm!("in al, dx", out("al") value, in("dx") port, options(nostack, preserves_flags, nomem));
    }
    value
}

pub fn read_cr0() -> u64 {
    let v: u64;
    // SAFETY: reading CR0 has no side effects and we are in ring 0 long mode.
    unsafe {
        core::arch::asm!("mov {}, cr0", out(reg) v, options(nostack, preserves_flags, nomem))
    };
    v
}

/// Time Stamp Counter (`rdtsc`). Monotonic per-socket counter of unknown
/// rate until calibrated against a real time source — see `timekeeping`
/// (M2.2), which measures TSC-per-PIT-oscillator-tick.
pub fn read_tsc() -> u64 {
    let lo: u32;
    let hi: u32;
    // SAFETY: RDTSC is unprivileged, side-effect-free beyond the two
    // output registers.
    unsafe {
        core::arch::asm!("rdtsc", out("eax") lo, out("edx") hi,
            options(nostack, preserves_flags, nomem));
    }
    ((hi as u64) << 32) | lo as u64
}

/// CR2 — last linear address that caused a #PF (SDM Vol. 3 §4.7). Read by
/// the exception handler so page-fault diagnostics can name the address.
pub fn read_cr2() -> u64 {
    let v: u64;
    // SAFETY: reading CR2 has no side effects; ring 0 long mode.
    unsafe {
        core::arch::asm!("mov {}, cr2", out(reg) v, options(nostack, nomem, preserves_flags));
    }
    v
}

pub fn read_cr3() -> u64 {
    let v: u64;
    // SAFETY: as read_cr0.
    unsafe {
        core::arch::asm!("mov {}, cr3", out(reg) v, options(nostack, preserves_flags, nomem))
    };
    v
}

pub fn read_cr4() -> u64 {
    let v: u64;
    // SAFETY: as read_cr0.
    unsafe {
        core::arch::asm!("mov {}, cr4", out(reg) v, options(nostack, preserves_flags, nomem))
    };
    v
}

/// IA32_EFER MSR (0xC0000080) — SDM Vol. 4 §2.2.1.
pub fn read_efer() -> u64 {
    let (lo, hi): (u32, u32);
    // SAFETY: RDMSR of EFER is side-effect-free; we are in ring 0.
    unsafe {
        core::arch::asm!("rdmsr", out("eax") lo, out("edx") hi, in("ecx") 0xC000_0080_u32,
            options(nostack, preserves_flags, nomem));
    }
    ((hi as u64) << 32) | (lo as u64)
}

/// CR0 bit masks — SDM Vol. 3 §2.5. (Only the bits M1 verifies; the rest
/// join when a subsystem reads them.)
/// Write CR0 (SDM Vol. 3 §2.5).
///
/// # Safety
/// Caller must preserve required long-mode bits (PG, PE, NE, ET, MP) and
/// understand the effect of every changed bit — paging-mode transitions are
/// serialization points.
pub unsafe fn write_cr0(v: u64) {
    // SAFETY: forwarded to caller contract.
    unsafe { core::arch::asm!("mov cr0, {}", in(reg) v, options(nostack, preserves_flags)) };
}

/// Write CR3 (SDM Vol. 3 §4.1.2) — switches the active page-table root and
/// invalidates non-global TLB entries.
///
/// # Safety
/// `v` must point at a valid, fully populated PML4 for the currently
/// executing address space, or the next memory access faults.
pub unsafe fn write_cr3(v: u64) {
    // SAFETY: forwarded to caller contract.
    unsafe { core::arch::asm!("mov cr3, {}", in(reg) v, options(nostack, preserves_flags)) };
}

/// Write EFER (SDM Vol. 4 §2.2.1).
///
/// # Safety
/// Caller must preserve LME/LMA/SCE and understand every changed bit (NXE
/// changes the meaning of PTE bit 63 for all subsequent translations).
pub unsafe fn write_efer(v: u64) {
    // SAFETY: EFER MSR 0xC0000080; WRMSR consumes EDX:EAX with the MSR in
    // ECX; forwarded to caller contract.
    unsafe {
        core::arch::asm!(
            "mov ecx, 0xC0000080",
            "wrmsr",
            in("eax") v as u32,
            in("edx") (v >> 32) as u32,
            out("ecx") _,
            options(nostack, preserves_flags),
        );
    }
}

pub mod cr0 {
    pub const PE: u64 = 1 << 0; // Protection Enable
    pub const WP: u64 = 1 << 16; // Write Protect (ring-0 writes honor RO pages)
    pub const PG: u64 = 1 << 31; // Paging
}

/// CR4 bit masks — SDM Vol. 3 §2.6.
pub mod cr4 {
    pub const PAE: u64 = 1 << 5; // Physical Address Extension (required in long mode)
}

/// IA32_EFER bit masks — SDM Vol. 4 §2.2.1.
pub mod efer {
    pub const LME: u64 = 1 << 8; // Long Mode Enable
    pub const LMA: u64 = 1 << 10; // Long Mode Active
    pub const NXE: u64 = 1 << 11; // No-Execute Enable (PTE bit 63)
}

/// Raw CPUID leaf result.
pub struct CpuidResult {
    pub eax: u32,
    pub ebx: u32,
    pub ecx: u32,
    pub edx: u32,
}

pub fn cpuid(leaf: u32) -> CpuidResult {
    // CPUID never faults; unsupported leaves are checked via max-leaf values
    // by callers before interpretation.
    let r = __cpuid(leaf);
    CpuidResult {
        eax: r.eax,
        ebx: r.ebx,
        ecx: r.ecx,
        edx: r.edx,
    }
}

/// Decoded CPU identity we care about at boot (SDM Vol. 2A §3.2 "CPUID").
pub struct CpuInfo {
    pub vendor: [u8; 12],
    pub family: u32,
    pub model: u32,
    pub stepping: u32,
    /// CPUID.1:EDX[26] — SSE2 (architecturally required on x86-64; we log it
    /// anyway because *verification* is the point).
    pub has_sse2: bool,
    /// CPUID.8000_0001h:EDX[20] — NX.
    pub has_nx: bool,
    /// CPUID.8000_0001h:EDX[29] — long mode (EM64T/AMD64).
    pub has_long_mode: bool,
    /// CPUID.8000_0008h:EAX[7:0] — physical address bits.
    pub phys_addr_bits: u32,
}

pub fn cpu_info() -> CpuInfo {
    let v = cpuid(0);
    let mut vendor = [0u8; 12];
    for (chunk, reg) in vendor.chunks_mut(4).zip([v.ebx, v.edx, v.ecx]) {
        chunk.copy_from_slice(&reg.to_le_bytes());
    }

    let f = cpuid(1);
    let stepping = f.eax & 0xF;
    let base_model = (f.eax >> 4) & 0xF;
    let base_family = (f.eax >> 8) & 0xF;
    let ext_model = (f.eax >> 16) & 0xF;
    let ext_family = (f.eax >> 20) & 0xFF;
    // SDM: display family = base + ext (when base == 0xF); display model =
    // (ext << 4) + base (when base family >= 6).
    let family = if base_family == 0xF {
        base_family + ext_family
    } else {
        base_family
    };
    let model = if base_family >= 6 {
        (ext_model << 4) + base_model
    } else {
        base_model
    };
    let has_sse2 = f.edx & (1 << 26) != 0;

    let ext_max = cpuid(0x8000_0000).eax;
    let (has_nx, has_long_mode) = if ext_max >= 0x8000_0001 {
        let e = cpuid(0x8000_0001);
        (e.edx & (1 << 20) != 0, e.edx & (1 << 29) != 0)
    } else {
        (false, false)
    };
    let phys_addr_bits = if ext_max >= 0x8000_0008 {
        cpuid(0x8000_0008).eax & 0xFF
    } else {
        0
    };

    CpuInfo {
        vendor,
        family,
        model,
        stepping,
        has_sse2,
        has_nx,
        has_long_mode,
        phys_addr_bits,
    }
}

impl fmt::Display for CpuInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let vendor = core::str::from_utf8(&self.vendor).unwrap_or("???");
        write!(
            f,
            "vendor={} family={} model={} stepping={} physaddr={}bit sse2={} nx={} longmode={}",
            vendor,
            self.family,
            self.model,
            self.stepping,
            self.phys_addr_bits,
            self.has_sse2,
            self.has_nx,
            self.has_long_mode
        )
    }
}
