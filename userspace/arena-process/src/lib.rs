#![no_std]
//! Capability-native process lifecycle primitives shared by the reusable
//! runtime and trusted process managers. PIDs remain descriptive metadata.

pub mod handles;
pub mod process;
