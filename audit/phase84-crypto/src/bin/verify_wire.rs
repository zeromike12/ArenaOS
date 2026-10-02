//! Host-only fuzz oracle: framed requests to the exact vendored no_std verifier.
//! Not a guest service. No signing or private test material here.
use std::io::{self, Read, Write};
use arena_phase84_crypto_audit::{canonical_public_key, sha256, strict_verify};

fn main() -> io::Result<()> {
    let mut input = io::stdin().lock();
    let mut output = io::stdout().lock();
    loop {
        let mut mode = [0u8; 1];
        if input.read(&mut mode)? == 0 {
            return Ok(());
        }
        if mode[0] > 1 {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "invalid oracle mode"));
        }
        let mut key = [0u8; 32];
        input.read_exact(&mut key)?;
        let mut len = [0u8; 4];
        input.read_exact(&mut len)?;
        let len = u32::from_le_bytes(len) as usize;
        if len > 5000 {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "oversize fuzz message"));
        }
        let mut msg = vec![0; len];
        input.read_exact(&mut msg)?;
        let mut sig = [0u8; 64];
        input.read_exact(&mut sig)?;
        let signer_canonical = if mode[0] == 1 {
            let mut subordinate = [0u8; 32];
            input.read_exact(&mut subordinate)?;
            // The caller cannot substitute a *different* valid key for the
            // subordinate signed in the root-authenticated policy header.
            // This host oracle enforces the same byte binding required of the
            // eventual guest decoder; signature-valid is not policy-valid.
            const DOMAIN: &[u8] = b"ArenaOS.policy.v1\0";
            let header = msg.strip_prefix(DOMAIN);
            header.is_some_and(|h| h.len() == 448 && &h[..4] == b"APOL" &&
                h[48..80] == subordinate && h[80..112] == sha256(&subordinate)) &&
                canonical_public_key(&subordinate) && subordinate != key
        } else {
            true
        };
        output.write_all(&[u8::from(signer_canonical && strict_verify(&key, &msg, &sig))])?;
        output.flush()?;
    }
}
