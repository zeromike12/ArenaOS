//! Terminal failure path: diagnostics, then a safe machine halt.
//!
//! "Halt safely" means the firmware's `ResetSystem(EfiResetShutdown)`
//! *runtime* service (ADR-0003) — runtime services survive
//! `ExitBootServices` by design (UEFI 2.10 §8.6), so this path works
//! identically in the boot stage and in the kernel proper after the M2.7
//! handoff. The boot stage deposits the entry point here as a raw
//! function pointer (`set_reset_system`); if it was never deposited
//! (failure before the firmware tables were captured) or firmware refuses
//! the call, we fall back to a CLI+HLT park loop: with IF=0 the CPU stays
//! parked forever.

use core::sync::atomic::{AtomicU64, Ordering};

/// EFI_RESET_TYPE `EfiResetShutdown` (UEFI 2.10 §8.6.11).
pub const RESET_SHUTDOWN: u32 = 2;

/// `EFI_SUCCESS` for the ResetStatus argument (UEFI 2.10 §8.6.11).
const EFI_SUCCESS: usize = 0;

/// EFI_RESET_SYSTEM (UEFI 2.10 §8.6.11): (ResetType, ResetStatus,
/// DataSize, ResetData).
type ResetSystemFn = unsafe extern "efiapi" fn(u32, usize, usize, *const u8);

static RESET_FN: AtomicU64 = AtomicU64::new(0);

/// Deposit the firmware's `ResetSystem` entry point (handoff ABI; see
/// [`crate::handoff`]).
///
/// # Safety
/// `ptr` must be the address of the firmware runtime services'
/// ResetSystem function (efiapi ABI), live for the machine's remaining
/// lifetime — the boot stage passes the typed table field directly.
pub unsafe fn set_reset_system(ptr: u64) {
    RESET_FN.store(ptr, Ordering::Relaxed);
}

/// Request a full system shutdown. Returns only if firmware never handed
/// over (or refuses) — and then parks the CPU instead, so from the
/// caller's perspective this never returns.
pub fn reset_shutdown() -> ! {
    let ptr = RESET_FN.load(Ordering::Relaxed);
    if ptr != 0 {
        // SAFETY: `ptr` was deposited by the boot stage from the live
        // runtime-services table (contract on `set_reset_system`);
        // ResetSystem is legal at any time and does not return on success.
        let reset: ResetSystemFn = unsafe { core::mem::transmute::<u64, ResetSystemFn>(ptr) };
        unsafe { reset(RESET_SHUTDOWN, EFI_SUCCESS, 0, core::ptr::null()) };
    }

    // Never deposited, or firmware misbehaved: park the CPU.
    // SAFETY: CLI+HLT with interrupts masked parks the single CPU
    // permanently; nothing else is runnable in the boot/kernel context.
    unsafe {
        crate::arch::x86_64::cli();
        loop {
            core::arch::asm!("hlt", options(nostack, nomem, preserves_flags));
        }
    }
}

/// Log the terminal reason, then halt safely.
pub fn halt_machine(reason: &str) -> ! {
    crate::log::log_error!("halt", "halting machine: {reason}");
    reset_shutdown()
}
