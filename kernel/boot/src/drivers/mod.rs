//! In-boot-stage device drivers. M1: the 16550 serial console. M2: the 8254
//! PIT (kernel tick + real-time reference for TSC calibration). Everything
//! beyond panic diagnostics moves to userspace driver servers in Phase 6
//! (ADR-0002).

pub mod pit;
pub mod serial;
