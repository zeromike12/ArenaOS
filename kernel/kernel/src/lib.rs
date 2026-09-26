//! ArenaOS kernel proper (M2.7 split, ADR-0011).
//!
//! Everything machine-level lives here: CPU/arch state, interrupt and
//! exception machinery, device drivers owned by the kernel (serial
//! console, PIT), physical frame allocation, the heap glue, logging,
//! timekeeping, and the terminal halt path. The boot stage (a UEFI
//! application in `kernel/boot`) initializes firmware-facing pieces and
//! hands over through the plain-data record in [`handoff`]; this crate
//! never sees a UEFI type or calls a boot service.
//!
//! Subsystem contracts are documented per-module; the boot contract
//! (single CPU, IF=0 outside bounded tested windows) applies until the
//! kernel entry sequence says otherwise.

#![no_std]

pub mod arch;
pub mod cap;
pub mod console;
pub mod drivers;
pub mod elf;
pub mod entry;
pub mod frames;
pub mod halt;
pub mod handoff;
pub mod heap;
pub mod ipc;
pub mod log;
pub mod m3;
pub mod m4;
pub mod m5;
pub mod proc;
pub mod relay;
pub mod sched;
pub mod spawn;
pub mod sync;
pub mod timekeeping;
