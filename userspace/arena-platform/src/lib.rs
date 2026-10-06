#![no_std]
//! Bounded native application-platform records (Phase 12, ADR-0080).
//!
//! This crate parses descriptive signed bundle metadata, provides a pure
//! registry/association and lifecycle model, an ABI-v2 startup codec, and a
//! bounded host-callable AFS2 install/readback and exact-tree registry-rebuild
//! core. It does not
//! hold capabilities, call filesd, choose protected directory authority,
//! authorize launch, or replace the receiver's persistent signer-policy
//! service. A cryptographically verified
//! bundle is not by itself an installed or active application.

#[cfg(test)]
extern crate std;

pub mod bundle;
pub mod install;
pub mod lifecycle;
pub mod manifest;
#[allow(clippy::new_without_default)]
#[rustfmt::skip]
#[path = "../../package.rs"]
pub mod package_policy;
pub mod policy;
pub mod registry;
pub mod startup;
