//! Independent public RFC 8032 vectors and OpenSSL-signed ADR-0053 bytes.
//! Host-only audit proof, NEVER compiled into the guest image. No private key.
use arena_phase84_crypto_audit::{canonical_public_key, sha256, strict_verify};
use ed25519_dalek::VerifyingKey;

fn hex<const N: usize>(s: &str) -> [u8; N] {
    assert_eq!(s.len(), N * 2);
    let mut result = [0u8; N];
    for (i, v) in result.iter_mut().enumerate() {
        *v = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).unwrap();
    }
    result
}
const PKG: &[u8] = b"ArenaOS.pkg.v1\0";
const POL: &[u8] = b"ArenaOS.policy.v1\0";
fn root() -> [u8; 32] {
    hex("d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a")
}
fn subordinate() -> [u8; 32] {
    hex("3d4017c3e843895a92b70aa74d1b7ebc9c982ccf2ec4968cc0cd55f12af4660c")
}
#[test]
fn public_rfc_vectors_and_refusal() {
    let v1 = hex("e5564300c360ac729086e2cc806e828a84877f1eb8e5d974d873e06522490155\
                  5fb8821590a33bacc61e39701cf9b46bd25bf5f0595bbe24655141438e7a100b");
    assert!(strict_verify(&root(), b"", &v1));
    let v2 = hex("92a009a9f0d4cab8720e820b5f642540a2b27b5416503f8fb3762223ebdb69da\
                  085ac1e43e15996e458f3613d0f11d8c387b2eaeb4302aeeb00d291612bb0c00");
    assert!(strict_verify(&subordinate(), b"\x72", &v2));
    assert!(!strict_verify(&root(), b"\x72", &v2));
    assert!(!strict_verify(&[0; 32], b"\x72", &v2));
    assert!(!strict_verify(&subordinate(), b"\x73", &v2));
}
#[test]
fn malformed_ed25519_encodings_and_hash() {
    assert_eq!(sha256(b"abc"), hex("ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"));
    let key = root();
    let valid = hex("e5564300c360ac729086e2cc806e828a84877f1eb8e5d974d873e06522490155\
                     5fb8821590a33bacc61e39701cf9b46bd25bf5f0595bbe24655141438e7a100b");
    let mut bad = valid;
    // Scalar S equal to the Ed25519 subgroup order is noncanonical.
    bad[32..].copy_from_slice(&hex::<32>("edd3f55c1a631258d69cf7a2def9de1400000000000000000000000000000010"));
    assert!(!strict_verify(&key, b"", &bad));
    bad = valid;
    bad[..32].fill(0); // Small-order R must not be accepted.
    assert!(!strict_verify(&key, b"", &bad));
    assert!(!strict_verify(&[0xff; 32], b"", &valid)); // Invalid/noncanonical public key.
    assert!(!strict_verify(&[0; 32], b"", &valid)); // Small-order key.
    assert!(canonical_public_key(&key));
    assert!(!canonical_public_key(&[0; 32]));
    // Some noncanonical compressed points DO decompress to non-small-order
    // points in dalek's ZIP-215-compatible parser. Test the key guard itself,
    // not just a forged signature that would fail either way.
    let mut aliases = 0;
    for y in 1..=18u8 {
        for sign in [0u8, 0x80] {
            let mut alias = [0xff; 32];
            alias[0] = 0xed + y; // p + y, where p = 2^255 - 19.
            alias[31] = 0x7f | sign;
            if let Ok(parsed) = VerifyingKey::from_bytes(&alias) {
                if !parsed.is_weak() && parsed.to_edwards().compress().to_bytes() != alias {
                    assert!(!canonical_public_key(&alias));
                    aliases += 1;
                }
            }
        }
    }
    assert!(aliases > 0, "expected a decompressed noncanonical non-small-order alias");
    assert!(<&[u8; 64]>::try_from(&valid[..63]).is_err()); // Wire-length boundary.
    assert!(<&[u8; 32]>::try_from(&key[..31]).is_err());
    // Deterministic negative bit mutations of the independent RFC vector.
    // Not a replacement for a coverage-guided artifact fuzz campaign.
    for i in 0..512 {
        bad = valid;
        bad[(i / 8) % 64] ^= 1 << (i % 8);
        assert!(!strict_verify(&key, b"", &bad), "accepted signature mutation {i}");
    }
}
#[test]
fn independent_package_and_policy_domains() {
    let payload = b"opaque-test-payload\x00\xff";
    let mut manifest = [0u8; 128];
    manifest[..4].copy_from_slice(b"APKG");
    manifest[4..6].copy_from_slice(&1u16.to_le_bytes());
    manifest[6..8].copy_from_slice(&128u16.to_le_bytes());
    manifest[8..10].copy_from_slice(&1u16.to_le_bytes());
    manifest[12..20].copy_from_slice(b"app.test");
    manifest[44..52].copy_from_slice(&7u64.to_le_bytes());
    manifest[52..56].copy_from_slice(&(payload.len() as u32).to_le_bytes());
    manifest[56..88].copy_from_slice(&sha256(payload));
    manifest[88..120].copy_from_slice(&sha256(&root()));
    let mut message = PKG.to_vec();
    message.extend_from_slice(&manifest);
    message.extend_from_slice(payload);
    let signature = hex("8a4c35fb9f7bffbcb68bebadefe8d6e48c666e3a5bf21c7fefe9c2ba9eca6ea28\
                         c39d4122f99249c902f09413edcebd23b7a4f1598e2df18e2044ee2107e0003");
    assert!(strict_verify(&root(), &message, &signature));
    assert!(!strict_verify(&subordinate(), &message, &signature));
    message[PKG.len() + 9] ^= 1;
    assert!(!strict_verify(&root(), &message, &signature));
    message[PKG.len() + 9] ^= 1;
    message[3] ^= 1;
    assert!(!strict_verify(&root(), &message, &signature));
    let mut policy = [0u8; 448];
    policy[..4].copy_from_slice(b"APOL");
    policy[4..6].copy_from_slice(&1u16.to_le_bytes());
    policy[6..8].copy_from_slice(&448u16.to_le_bytes());
    policy[8..16].copy_from_slice(&1u64.to_le_bytes());
    policy[16..24].copy_from_slice(b"app.test");
    policy[48..80].copy_from_slice(&subordinate());
    policy[80..112].copy_from_slice(&sha256(&subordinate()));
    policy[112..120].copy_from_slice(&7u64.to_le_bytes());
    policy[120] = 1;
    let sig = hex("1ad43e6ef3e9245907f589800c036dec163e4b35ee21562ff28cd0b558fc699c\
                   adefc53e41f3610f95ad0ab0bb727e87b4255190c49cfc1367e4fae24d4a5200");
    let mut signed = POL.to_vec();
    signed.extend_from_slice(&policy);
    assert!(strict_verify(&root(), &signed, &sig));
    assert!(!strict_verify(&subordinate(), &signed, &sig));
    signed[POL.len()+120] = 2;
    assert!(!strict_verify(&root(), &signed, &sig));
}
