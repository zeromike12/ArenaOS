//! In-boot-stage device drivers. M1: the 16550 serial console. Everything
//! beyond panic diagnostics moves to userspace driver servers in Phase 6
//! (ADR-0002).

pub mod serial;
