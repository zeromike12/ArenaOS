//! ADR-0053 pinned no_std, verification-only primitives, now also linked by
//! the Phase 8.4 guest staging receiver. No signing, RNG or private keys.
#![no_std]
use ed25519_dalek::{Signature, VerifyingKey};
use sha2::{Digest, Sha256};

fn canonical_key(key: &[u8; 32]) -> Option<VerifyingKey> {
    let decoded = VerifyingKey::from_bytes(key).ok()?;
    // `from_bytes` permits some ZIP-215 aliases: decompression alone is NOT
    // proof that the public-key encoding is canonical. Never admit aliases
    // into an on-disk signer identity, even though verify_strict checks torsion.
    if decoded.is_weak() || decoded.to_edwards().compress().to_bytes() != *key {
        return None;
    }
    Some(decoded)
}

pub fn canonical_public_key(key: &[u8; 32]) -> bool {
    canonical_key(key).is_some()
}

pub fn strict_verify(key: &[u8; 32], message: &[u8], sig: &[u8; 64]) -> bool {
    canonical_key(key)
        .is_some_and(|decoded| decoded.verify_strict(message, &Signature::from_bytes(sig)).is_ok())
}

pub fn sha256(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

/// Incremental SHA-256 state for bounded stream verification.
///
/// This exposes no new primitive: it is a thin wrapper over the same pinned
/// `sha2` 0.10.9 implementation used by [`sha256`]. Callers should keep the
/// state private to one digest operation and finalize it exactly once.
pub struct Sha256State(Sha256);

impl Sha256State {
    pub fn new() -> Self {
        Self(Sha256::new())
    }

    pub fn update(&mut self, bytes: &[u8]) {
        self.0.update(bytes);
    }

    pub fn finalize(self) -> [u8; 32] {
        self.0.finalize().into()
    }
}

impl Default for Sha256State {
    fn default() -> Self {
        Self::new()
    }
}
