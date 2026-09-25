//! Terminal failure path: diagnostics, then a safe machine halt.
//!
//! "Halt safely" means UEFI `ResetSystem(EfiResetShutdown)` (ADR-0003) —
//! the same mechanism normal completion uses, so tests can distinguish
//! "kernel decided to stop" from "kernel stopped existing". If the firmware
//! hand-off is unavailable (panic before `uefi::init`), fall back to a
//! CLI+HLT park loop: with IF=0 the CPU stays parked forever.

use crate::uefi;

pub fn halt_machine(reason: &str) -> ! {
    crate::log::log_error!("halt", "halting machine: {reason}");
    uefi::reset_shutdown();

    // SAFETY: CLI+HLT with interrupts masked parks the single boot CPU
    // permanently; nothing else is runnable in the M1 boot context.
    unsafe {
        crate::arch::x86_64::cli();
        loop {
            core::arch::asm!("hlt", options(nostack, nomem, preserves_flags));
        }
    }
}
