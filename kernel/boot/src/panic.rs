//! Kernel panic handler: diagnostics first, then a safe halt.
//!
//! Contract (ADR-0005): any panic emits a machine-checkable
//! `[arena PANIC file:line] message` marker on serial before the machine is
//! halted, so the test harness fails the run even if the panic happened after
//! some tests already passed. Halting delegates to the kernel's terminal
//! path (`arena_kernel::halt`): UEFI `ResetSystem(Shutdown)` — the same
//! safe halt as normal completion (ADR-0003) — with a CLI+HLT park loop as
//! the fallback when the firmware pointer was never deposited.

use core::fmt::Write;
use core::panic::PanicInfo;

use arena_kernel::drivers::serial::SerialConsole;

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

    // Clean firmware shutdown; the kernel halt path falls back to a
    // CLI+HLT park loop if the runtime pointer was never deposited
    // (panic before uefi::init) — either way this never returns.
    arena_kernel::halt::reset_shutdown()
}
