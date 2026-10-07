#![no_std]
//! Dependency-light native startup wire ABI. This codec is intentionally
//! separate from package verification and registry policy so the entry
//! runtime does not link signing/crypto machinery.

/// Descriptive identity/flag constants shared with the APB1 manifest model.
pub mod manifest {
    pub const ID_BYTES: usize = 32;
    pub const FLAG_MULTI_INSTANCE: u32 = 1 << 0;
    pub const FLAG_BACKGROUND: u32 = 1 << 1;
    pub const FLAG_HEADLESS: u32 = 1 << 2;
    pub const FLAG_STANDARD_STREAMS: u32 = 1 << 3;
    pub const FLAG_NATIVE_SYNC: u32 = 1 << 4;
}

// Keep one implementation of the ARST v2 codec and its 32-slot bound.
// `arena-platform-core` re-exports this module for host policy/tests; the
// runtime and native apps depend directly on this small crate.
#[path = "../../arena-platform/src/startup.rs"]
pub mod startup;
