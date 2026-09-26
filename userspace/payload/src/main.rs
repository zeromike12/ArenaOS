//! ArenaOS M4 test payload — the first userspace image (ADR-0016).
//!
//! A genuine cargo/rust-lld ELF64 artifact (`x86_64-unknown-none`,
//! `payload.ld`) with a fixed, self-describing layout so the kernel's
//! loader tests assert exact bytes:
//!
//! - `0x200000` — `_start` entry stub. In M4.1 it is a `jmp self`
//!   placeholder (the loader must place it, not run it — ring-3
//!   execution of a loaded image is M4.3, behind the M4.2 syscall ABI).
//! - `0x201000` — `META`, the payload manifest: magic plus the entry
//!   and BSS facts. The tests cross-check these against what they parse
//!   from the ELF header itself — two independent descriptions of the
//!   same image that must agree.
//! - `0x202000` — 4 KiB of `.bss` (`NOLOAD` in the linker script): it
//!   occupies `p_memsz` but not `p_filesz`, proving the loader's
//!   zero-fill path.
#![no_std]
#![no_main]

use core::panic::PanicInfo;

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    // The payload never runs in M4.1; if a future milestone reaches a
    // panic here, spinning is the honest outcome (no console exists for
    // it yet — M4.2's debug-write syscall will change that).
    loop {}
}

/// Payload manifest at the fixed VA `0x201000` (ADR-0016).
#[repr(C)]
pub struct Meta {
    /// Identity marker the loader tests read back byte-for-byte.
    pub magic: [u8; 16],
    /// Must equal the ELF header's `e_entry` — cross-checked in-guest.
    pub entry: u64,
    /// Start of the NOLOAD `.bss` region.
    pub bss_va: u64,
    /// Length of the `.bss` region (whole pages).
    pub bss_len: u64,
}

#[used]
#[unsafe(no_mangle)]
#[unsafe(link_section = ".payload_meta")]
pub static META: Meta = Meta {
    magic: *b"ARENAOS-PAYLOAD!",
    entry: 0x200000,
    bss_va: 0x202000,
    bss_len: 0x1000,
};

/// The `.bss` canary: 4 KiB of zero-initialized static that the linker
/// script places at `0x202000` as NOLOAD — absent from the file, present
/// in `p_memsz`. Every byte must read back zero under the loaded
/// process's CR3.
#[used]
#[unsafe(no_mangle)]
#[unsafe(link_section = ".payload_bss")]
pub static BSS_CANARY: [u64; 512] = [0; 512];

core::arch::global_asm!(
    ".globl _start",
    "_start:",
    "1: jmp 1b", // M4.1 placeholder: two bytes, EB FE, at 0x200000.
);
