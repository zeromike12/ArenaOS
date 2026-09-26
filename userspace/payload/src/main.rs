//! ArenaOS userspace test payload — the first running program (M4.3).
//!
//! A genuine cargo/rust-lld ELF64 artifact (`x86_64-unknown-none`,
//! `payload.ld`) with a fixed, self-describing layout so the kernel's
//! loader and process tests assert exact facts:
//!
//! - `0x200000` — `_start`, the real program (PT_LOAD #1, R+X): it
//!   proves the loaded image RUNS in ring 3 — reads its own META,
//!   verifies the NOLOAD bss reads zero, stamps it, writes a message
//!   through `SYS_DEBUG_WRITE`, and exits through `SYS_THREAD_EXIT`
//!   (ABI v1, ADR-0017).
//! - `0x200800` — `MSG_BUF`, the message pinned by the linker script so
//!   META can describe it as a plain (va, len) fact.
//! - `0x201000` — `META`, the payload manifest: magic, entry, bss span,
//!   message span, success exit code. The kernel tests cross-check these
//!   against the ELF header AND derive their expectations for the
//!   running program from META itself — no literals duplicated across
//!   the privilege boundary beyond the magic and the diagnostic codes.
//! - `0x202000` — 4 KiB of `.bss` (`NOLOAD` in the linker script): it
//!   occupies `p_memsz` but not `p_filesz`, proving the loader's
//!   zero-fill path; the program stamps its first slot from ring 3 and
//!   the kernel reads the stamp back under the process CR3.
//!
//! Exit codes (the diagnostic contract with kernel m4 test
//! `first_process` — mirrored there): 42 success (= META.exit_ok),
//! 43 META magic wrong, 44 bss not zero before the stamp, 45
//! debug_write returned the wrong count, 99 the panic handler ran.
#![no_std]
#![no_main]

use core::panic::PanicInfo;

// ---- fixed layout facts (payload.ld pins these addresses) ----------------

/// Entry VA — the linker script's `. = 0x200000`.
pub const ENTRY_VA: u64 = 0x200000;
/// Message VA — the linker script's pinned `.payload_msg` section.
pub const MSG_VA: u64 = 0x200800;
/// META VA — the linker script's `. = 0x201000`.
pub const META_VA: u64 = 0x201000;
/// BSS VA and length — the NOLOAD `.payload_bss` region.
pub const BSS_VA: u64 = 0x202000;
pub const BSS_LEN: u64 = 0x1000;

// ---- exit-code contract (mirrored in kernel/kernel/src/m4.rs) ------------

pub const EXIT_OK: u64 = 42;
pub const EXIT_BAD_MAGIC: u64 = 43;
pub const EXIT_BSS_NOT_ZERO: u64 = 44;
pub const EXIT_BAD_WRITE: u64 = 45;
pub const EXIT_PANIC: u64 = 99;

/// The META identity marker (duplicated once — the manifest field and
/// the ring-3 read-back check below both spell it, on purpose: the check
/// proves the file-backed data page arrives with its linked content).
pub const MAGIC: &[u8; 16] = b"ARENAOS-PAYLOAD!";

// ---- ABI v1 syscall wrappers (ADR-0017) -----------------------------------

/// Raw two-argument syscall: RAX = call number, args in RDI/RSI; typed
/// i64 status back in RAX. Per the ABI the kernel preserves only
/// RBX/RBP/R12–R15, so every other caller-saved register — including
/// RDI/RSI themselves and the hardware-consumed RCX/R11 — is declared
/// clobbered. The stub runs entirely on the kernel stack; the user
/// stack (and its red zone) is untouched (`nostack`).
unsafe fn syscall2(nr: u64, a0: u64, a1: u64) -> i64 {
    let ret: i64;
    // SAFETY: caller contract — nr/a0/a1 are ABI v1 register arguments;
    // the clobber list below is the documented caller-saved set.
    unsafe {
        core::arch::asm!(
            "syscall",
            inlateout("rax") nr as i64 => ret,
            inlateout("rdi") a0 => _,
            inlateout("rsi") a1 => _,
            lateout("rdx") _,
            lateout("r8") _,
            lateout("r9") _,
            lateout("r10") _,
            lateout("rcx") _,
            lateout("r11") _,
            options(nostack),
        );
    }
    ret
}

/// SYS_DEBUG_WRITE (call 1): validated copy-out to the kernel console.
fn sys_debug_write(buf: *const u8, len: usize) -> i64 {
    // SAFETY: the wrapper's contract; buf/len describe this image's own
    // pinned message (or a string literal) — always inside the thread's
    // registered user regions.
    unsafe { syscall2(1, buf as u64, len as u64) }
}

/// SYS_THREAD_EXIT (call 2): terminate this thread with a status code.
fn sys_thread_exit(code: u64) -> ! {
    // SAFETY: as above.
    unsafe { syscall2(2, code, 0) };
    // The kernel never returns from thread_exit; if it somehow does,
    // spinning is the honest outcome.
    loop {
        core::hint::spin_loop();
    }
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    // The program below has no panicking path (no arithmetic traps, no
    // indexing, no allocation). If a future payload reaches this, say so
    // through the ABI itself — the kernel side sees the message on the
    // console AND the diagnostic exit code.
    let msg = b"ARENAOS-PAYLOAD: PANIC\n";
    sys_debug_write(msg.as_ptr(), msg.len());
    sys_thread_exit(EXIT_PANIC);
}

// ---- the program ----------------------------------------------------------

/// The message, pinned at `MSG_VA` by the linker script. A const block
/// copies the literal so the array length is derived, never hand-counted.
#[used]
#[unsafe(no_mangle)]
#[unsafe(link_section = ".payload_msg")]
pub static MSG_BUF: [u8; MSG.len()] = {
    let mut out = [0u8; MSG.len()];
    let mut i = 0;
    while i < MSG.len() {
        out[i] = MSG[i];
        i += 1;
    }
    out
};

/// The message text; `MSG_BUF` is its pinned, addressable copy.
pub const MSG: &[u8] =
    b"ARENAOS-M43-FIRST-USER-PROCESS: ELF-loaded image running in ring 3, wrote via syscall, exiting now\n";

/// Payload manifest at the fixed VA `META_VA` (ADR-0016): every fact the
/// kernel tests need about this image, described by the image itself.
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
    /// VA of the pinned message (inside the file-backed text span).
    pub msg_va: u64,
    /// Message length in bytes (within the debug_write cap).
    pub msg_len: u64,
    /// Exit code the kernel expects when every ring-3 check passed.
    pub exit_ok: u64,
}

#[used]
#[unsafe(no_mangle)]
#[unsafe(link_section = ".payload_meta")]
pub static META: Meta = Meta {
    magic: *MAGIC,
    entry: ENTRY_VA,
    bss_va: BSS_VA,
    bss_len: BSS_LEN,
    msg_va: MSG_VA,
    msg_len: MSG_BUF.len() as u64,
    exit_ok: EXIT_OK,
};

/// The `.bss` canary: 4 KiB of zero-initialized static that the linker
/// script places at `BSS_VA` as NOLOAD — absent from the file, present
/// in `p_memsz`. Slot 0 receives the ring-3 stamp; every other byte must
/// read back zero under the loaded process's CR3, before and after.
#[used]
#[unsafe(no_mangle)]
#[unsafe(link_section = ".payload_bss")]
pub static mut BSS_CANARY: [u64; 512] = [0; 512];

/// The first ArenaOS program (M4.3). Entered by `iretq` at ring 3 with
/// RSP = the process stack top; nothing here may return — the only exits
/// are `SYS_THREAD_EXIT` (success or diagnostic code).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _start() -> ! {
    // SAFETY: this is the whole program — bare volatile accesses to its
    // own linked statics, inside the address space the kernel loaded,
    // verified, and handed to ring 3. Single-threaded, no aliases.
    unsafe {
        // 1. The file-backed META must read back with its linked content
        //    — proves the data page arrived byte-exact (ring-3 read).
        let magic = core::ptr::read_volatile(core::ptr::addr_of!(META.magic));
        if magic != *MAGIC {
            sys_thread_exit(EXIT_BAD_MAGIC);
        }

        // 2. The NOLOAD bss must read as zero (ring-3 view of the
        //    loader's zero-fill), then take the stamp (ring-3 write —
        //    the kernel reads this slot back under the process CR3).
        //    The stamp is the FIRST 8 magic bytes as a LE u64 — the
        //    kernel derives the identical expectation from META_MAGIC.
        let slot = core::ptr::addr_of_mut!(BSS_CANARY[0]);
        if core::ptr::read_volatile(slot) != 0 {
            sys_thread_exit(EXIT_BSS_NOT_ZERO);
        }
        let mut stamp_bytes = [0u8; 8];
        let mut i = 0;
        while i < 8 {
            stamp_bytes[i] = magic[i];
            i += 1;
        }
        core::ptr::write_volatile(slot, u64::from_le_bytes(stamp_bytes));

        // 3. The real work: write the pinned message through the syscall
        //    boundary. The typed status must be exactly the byte count.
        let n = sys_debug_write(core::ptr::addr_of!(MSG_BUF).cast::<u8>(), MSG_BUF.len());
        if n != MSG_BUF.len() as i64 {
            sys_thread_exit(EXIT_BAD_WRITE);
        }

        // 4. Success — exit with META's own code: the kernel reads the
        //    same META out of the image file and expects that value.
        sys_thread_exit(core::ptr::read_volatile(core::ptr::addr_of!(META.exit_ok)));
    }
}
