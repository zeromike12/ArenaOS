//! Structured kernel logging (CONVENTIONS: everything goes through here,
//! never raw prints).
//!
//! Line format — greppable, ASCII, CRLF-terminated for serial:
//!
//! ```text
//! [arena INFO boot] message with key=value fields
//! [arena WARN  serial] ...
//! [arena ERROR vm] ...
//! [arena PANIC kernel/boot/src/main.rs:42] the panic message
//! ```
//!
//! Test markers (`m1:test:*`, `m1: RESULT ...`) are emitted by the test
//! runner in `m1.rs` with the exact grammar from docs/TESTING.md; they are
//! deliberately NOT routed through level tags so the harness can grep them
//! without ambiguity.

use core::fmt::Write;

use crate::drivers::serial::SerialConsole;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Info,
    Warn,
    Error,
}

impl Level {
    const fn label(self) -> &'static str {
        match self {
            Level::Info => "INFO",
            Level::Warn => "WARN",
            Level::Error => "ERROR",
        }
    }
}

/// Write one log line. Serial is polled and interrupts are off during boot,
/// so this is inherently serialized; the M2 kernel will gate this on the
/// boot console lock once concurrency exists.
pub fn write(level: Level, module: &str, args: core::fmt::Arguments<'_>) {
    let mut console = SerialConsole;
    let _ = writeln!(console, "[arena {:5} {}] {}", level.label(), module, args);
}

/// Marker line for the test harness — exact grammar, no decoration.
pub fn write_marker(args: core::fmt::Arguments<'_>) {
    let mut console = SerialConsole;
    let _ = writeln!(console, "{}", args);
}

#[macro_export]
macro_rules! log_info {
    ($module:expr, $($arg:tt)*) => {
        $crate::log::write($crate::log::Level::Info, $module, format_args!($($arg)*))
    };
}

#[macro_export]
macro_rules! log_warn {
    ($module:expr, $($arg:tt)*) => {
        $crate::log::write($crate::log::Level::Warn, $module, format_args!($($arg)*))
    };
}

#[macro_export]
macro_rules! log_error {
    ($module:expr, $($arg:tt)*) => {
        $crate::log::write($crate::log::Level::Error, $module, format_args!($($arg)*))
    };
}

pub use log_error;
pub use log_info;
pub use log_warn;
