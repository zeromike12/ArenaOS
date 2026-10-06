#![no_std]
//! Small native ArenaOS runtime layer. It deliberately exposes held-cap
//! semantics rather than emulating POSIX descriptors or inherited ambient
//! authority.

pub mod capabilities;
pub use arena_process::{handles, process};
pub mod heap;
pub mod startup;
pub mod tls;
