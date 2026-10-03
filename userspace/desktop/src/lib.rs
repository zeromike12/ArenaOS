//! Desktop userspace policy. Receiver adapters must verify capabilities.
#![no_std]
pub mod model;

pub mod perf;

pub mod compose;
pub mod shell;

pub mod wire;

pub mod client;

pub mod input_wire;

pub mod fs_backend;
pub mod preferences;
pub mod scope;
pub mod service_wire;

#[path = "../../abi.rs"]
mod abi;

pub mod app_client;
pub mod apps;
