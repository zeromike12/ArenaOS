//! Independent OpenSSL-signed host fixture comparison against the guest's
//! exact no_std ADR-0053 parser; these bytes are public, not signing seeds.
#[path = "../../../userspace/package.rs"]
mod package;
use package::*;

const ROOT_FILE: &[u8] = include_bytes!("../corpus/package-79a0ebe5653addf98c8060fc.bin");
const SUB_FILE: &[u8] = include_bytes!("../corpus/package-6a1b68578b2f1cdaabc41975.bin");
const POLICY: &[u8] = include_bytes!("../corpus/policy-06487f2f7e48586e876b8552.bin");
// Root-signed with the public RFC test seed, but subordinate has a ZIP-215
// noncanonical non-small-order alias. Signature-only checking would pass.
const ALIAS_POLICY: &[u8] = include_bytes!("../corpus/policy-a8f728dff763708fd84e7eb1.bin");

#[test]
fn rfc_root_and_open_ssl_fixtures() {
    let mut scratch = [0u8; VERIFY_SCRATCH];
    assert_eq!(arena_phase84_crypto_audit::sha256(&ROOT), ROOT_ID);
    let root = parse_package(ROOT_FILE).unwrap();
    assert_eq!(root.version, 7);
    assert_eq!(root.full_digest, arena_phase84_crypto_audit::sha256(ROOT_FILE));
    assert_eq!(root.verify(&ROOT, &mut scratch), Ok(()));
    assert_eq!(root.verify(&[0; 32], &mut scratch), Err(Error::UnknownSigner));
    let sub = parse_package(SUB_FILE).unwrap();
    assert_eq!(sub.version, 8);
    assert_eq!(sub.verify(&ROOT, &mut scratch), Err(Error::UnknownSigner));
    let policy = parse_policy(POLICY).unwrap();
    assert_eq!(policy.generation, 1);
    assert!(policy.allow);
    let mut chain = Chain::new();
    assert_eq!(chain.eligible(&root, &mut scratch), Ok(()));
    assert_eq!(chain.eligible(&sub, &mut scratch), Err(Error::UnknownSigner));
    chain.add(policy).unwrap();
    assert_eq!(chain.eligible(&root, &mut scratch), Ok(()));
    assert_eq!(chain.eligible(&sub, &mut scratch), Ok(()));
    let mut history = History::new();
    assert_eq!(history.stage_candidate(&root, &chain, &mut scratch), Ok(true));
    history.ingest(&root, &chain, &mut scratch).unwrap();
    assert_eq!(history.stage_candidate(&root, &chain, &mut scratch), Ok(false));
    assert_eq!(history.stage_candidate(&sub, &chain, &mut scratch), Ok(true));
    history.ingest(&sub, &chain, &mut scratch).unwrap();
    assert_eq!(history.decision(&sub, &chain, &mut scratch), Ok(()));
    assert_eq!(history.stage_candidate(&sub, &chain, &mut scratch), Ok(false));
    assert_eq!(history.stage_candidate(&root, &chain, &mut scratch), Err(Error::Downgrade));
    let id = root.id;
    assert_eq!(input_name(&id, false).unwrap().len(), 23);
    assert_eq!(numbered_name(b"s8-", &id, 2, 2).unwrap().len(), 26);
    assert_eq!(numbered_name(b"p8-", &id, 5, 4), Err(Error::BadFormat));
}

#[test]
fn signed_and_unsigned_mutations_refuse() {
    let mut scratch = [0u8; VERIFY_SCRATCH];
    let mut file = ROOT_FILE.to_vec();
    file[10] = 1; // flags
    assert!(parse_package(&file).is_err());
    file = ROOT_FILE.to_vec();
    file[44] ^= 1; // signed version, not digest-protected
    assert_eq!(parse_package(&file).unwrap().verify(&ROOT, &mut scratch), Err(Error::BadSignature));
    file = ROOT_FILE.to_vec();
    file[129] ^= 1; // payload corruption
    assert_eq!(parse_package(&file).err(), Some(Error::BadFormat));
    file = ROOT_FILE.to_vec();
    *file.last_mut().unwrap() ^= 1;
    assert_eq!(parse_package(&file).unwrap().verify(&ROOT, &mut scratch), Err(Error::BadSignature));
    assert_eq!(parse_package(&ROOT_FILE[..ROOT_FILE.len()-1]).err(), Some(Error::BadFormat));
    let mut policy = POLICY.to_vec();
    policy[121] = 9;
    assert_eq!(parse_policy(&policy), Err(Error::BadFormat));
    policy = POLICY.to_vec();
    policy[448] ^= 1;
    assert_eq!(parse_policy(&policy), Err(Error::BadSignature));
    policy = POLICY.to_vec();
    policy[48..80].fill(0);
    assert_eq!(parse_policy(&policy), Err(Error::BadFormat));
    policy = POLICY.to_vec();
    let mut alias = [0xff; 32]; alias[0] = 0xf0; alias[31] = 0x7f;
    policy[48..80].copy_from_slice(&alias);
    assert_eq!(parse_policy(&policy), Err(Error::BadFormat));
    assert_eq!(parse_policy(&POLICY[..511]), Err(Error::BadFormat));
    let mut signed_message = b"ArenaOS.policy.v1\0".to_vec();
    signed_message.extend_from_slice(&ALIAS_POLICY[..448]);
    assert!(arena_phase84_crypto_audit::strict_verify(&ROOT, &signed_message,
        ALIAS_POLICY[448..].try_into().unwrap()));
    assert_eq!(parse_policy(ALIAS_POLICY), Err(Error::BadFormat));
}

#[test]
fn exhaustive_single_bit_and_length_refusal_of_frozen_artifacts() {
    let mut scratch = [0u8; VERIFY_SCRATCH];
    for (base, policy) in [(ROOT_FILE, false), (POLICY, true)] {
        for index in 0..base.len() {
            for bit in 0..8 {
                let mut changed = base.to_vec();
                changed[index] ^= 1 << bit;
                if policy {
                    assert!(parse_policy(&changed).is_err(), "policy bit {index}:{bit}");
                } else if let Ok(pkg) = parse_package(&changed) {
                    assert!(pkg.verify(&ROOT, &mut scratch).is_err(), "package bit {index}:{bit}");
                }
            }
        }
        for length in 0..base.len() {
            if policy { assert!(parse_policy(&base[..length]).is_err()); }
            else { assert!(parse_package(&base[..length]).is_err()); }
        }
        let mut appended = base.to_vec(); appended.push(0);
        if policy { assert!(parse_policy(&appended).is_err()); }
        else { assert!(parse_package(&appended).is_err()); }
    }
}

#[test]
fn chain_refusals_and_typed_capacity() {
    let mut scratch = [0u8; VERIFY_SCRATCH];
    let policy = parse_policy(POLICY).unwrap();
    let mut chain = Chain::new();
    assert_eq!(chain.add(Policy { generation: 2, ..policy }), Err(Error::Corrupt));
    chain.add(policy).unwrap();
    assert_eq!(chain.add(policy), Err(Error::Corrupt));
    let mut next = Policy { generation: 2, allow: false, ..policy };
    // Tests below only exercise the pure sequence model; each real policy
    // *must* pass parse_policy's root signature check first.
    next.revoked_count = 8;
    for i in 0..8 { next.revoked[i] = [i as u8 + 1; 32]; }
    chain.add(next).unwrap();
    let prior = next;
    assert_eq!(next.add_revocation([9; 32]), Err(Error::NoSpace));
    assert_eq!(next, prior); // typed pre-mutation exhaustion
    assert_eq!(next.add_revocation([1; 32]), Ok(false)); // duplicate idempotent
    assert_eq!(next, prior);
    let root = parse_package(ROOT_FILE).unwrap();
    assert_eq!(chain.eligible(&root, &mut scratch), Ok(()));
    let sub = parse_package(SUB_FILE).unwrap();
    assert_eq!(chain.eligible(&sub, &mut scratch), Err(Error::Revoked));
    let third = Policy { generation: 3, allow: true, ..next };
    assert_eq!(chain.add(third), Err(Error::Corrupt)); // retired signer
    // The parser, rather than Chain::add, refuses a root-as-subordinate
    // policy before any sequence mutation.
    let mut bad_root = POLICY.to_vec();
    bad_root[48..80].copy_from_slice(&ROOT);
    assert_eq!(parse_policy(&bad_root), Err(Error::BadFormat));
    let mut conflict = History::new();
    conflict.ingest(&root, &chain, &mut scratch).unwrap();
    let mut modified = ROOT_FILE.to_vec();
    modified[ROOT_FILE.len()-1] ^= 1;
    let changed = parse_package(&modified).unwrap();
    assert_eq!(conflict.stage_candidate(&changed, &chain, &mut scratch), Err(Error::BadSignature));
    // Same version with a different valid signature would produce Conflict;
    // without the private test seed the host test never forges that signature.
    assert_eq!(chain.add(Policy { generation: 4, ..next }), Err(Error::Corrupt));
}
