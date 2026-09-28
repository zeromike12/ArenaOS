#![no_std]
//! Phase 8.0 policy substrate. This is not a running manager: until a
//! bootstrapped userspace service queries its actual caps, validates a
//! manifest, and spawns/reaps a child, these types authorize nothing.

pub mod inventory;
pub mod manifest;
