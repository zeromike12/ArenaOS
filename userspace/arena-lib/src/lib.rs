#![no_std]
//! ADR-0052: separately compiled clients. Every authority is an explicit
//! caller-held cap slot (or service-issued bearer), never a process name.
#[path = "../../abi.rs"]
pub mod abi;
pub mod sys;
pub mod ipc;
pub mod fs;
#[path = "../../net.rs"]
pub mod net;
