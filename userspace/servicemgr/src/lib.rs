#![no_std]
//! Phase 8.0 policy/runtime substrate: a ring-3 manager now boots,
//! queries actual caps and validates device readiness + a bounded
//! manifest. It now spawns the first production stack and has a
//! Process-cap reap path, but production restart is NOT proven.
//! These types never mint authority; only the kernel grants caps.

pub mod inventory;
pub mod manifest;
pub mod readiness;
