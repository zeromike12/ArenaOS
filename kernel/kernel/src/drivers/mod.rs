//! Kernel-owned device drivers. M1: the 16550 serial console. M2: the 8254
//! PIT (kernel tick + real-time reference for TSC calibration) and, from
//! M2.7 on, the interrupt-controller pair (IOAPIC/LAPIC) whose routing the
//! firmware tears down at ExitBootServices. Everything
//! beyond panic diagnostics moves to userspace driver servers in Phase 6
//! (ADR-0002).

pub mod intc;
pub mod pit;
pub mod serial;
