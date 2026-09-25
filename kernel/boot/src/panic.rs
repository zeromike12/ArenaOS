//! Kernel panic handler: diagnostics first, then a safe halt.
//!
//! Contract (ADR-0005): any panic emits a machine-checkable
//! `[arena PANIC file:line] message` marker on serial before the machine is
//! halted, so the test harness fails the run even if the panic happened after
//! some tests already passed. Halting uses UEFI `ResetSystem(Shutdown)` —
//! the same safe-halt path as normal completion (ADR-0003). If the firmware
//! pointer was never captured (panic before `uefi::init`), we fall back to
//! CLI+HLT forever: with interrupts disabled, HLT parks the CPU without
//! risking a spurious-wakeup loop.

use core::fmt::Write;
use core::panic::PanicInfo;

use crate::drivers::serial::SerialConsole;

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    let mut console = SerialConsole;
    let location = info.location().map(|l| (l.file(), l.line()));
    match location {
        Some((file, line)) => {
            let _ = write!(
                console,
                "\n[arena PANIC {}:{}] {}\n",
                file,
                line,
                info.message()
            );
        }
        None => {
            let _ = write!(
                console,
                "\n[arena PANIC unknown-location] {}\n",
                info.message()
            );
        }
    }

    // Try a clean firmware shutdown; fall back to park-the-CPU.
    crate::uefi::reset_shutdown();

    // reset_shutdown returns only if firmware hand-off failed (e.g., panic
    // before the system table was captured).
    // SAFETY: CLI keeps interrupts off; HLT with IF=0 parks the CPU
    // permanently. Single-CPU boot context (ADR-0003), nothing else runs.
    unsafe {
        core::arch::asm!("cli");
        loop {
            core::arch::asm!("hlt", options(nostack, nomem, preserves_flags));
        }
    }
}
