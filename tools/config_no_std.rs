//! Standalone no_std compile gate for the same module configd will include.
#![no_std]
#[path = "../userspace/config.rs"]
pub mod config;
