//! APB1 signer-policy binding (ADR-0053 policy semantics, ADR-0080/0082).
//!
//! The APB1 signer ID is only an index into a receiver-owned policy chain.
//! This module reuses the existing APKG v1 `Chain`/`Policy` byte-policy model
//! without changing APKG v1 parsing or staging. The current key is selected
//! from the signed package-ID namespace, then the verified APB1 version and
//! complete-bundle digest are checked against minimum-version and revocation
//! policy. The Phase-8 root in `package_policy` is explicitly a public test
//! root; production root provisioning is not claimed by this module.

use crate::{
    bundle::{BundleClaim, VerifiedBundle},
    package_policy::{self as apkg, Chain},
};
/// Resolve a candidate key for a bounded, unauthenticated claim. The caller
/// must still run `Workspace::verify` and then `check_eligible` on the
/// resulting bundle. This operation only selects key material; it does not
/// grant filesystem, launch, process, or document authority.
pub fn select_key(chain: &Chain, claim: &BundleClaim) -> Result<[u8; 32], apkg::Error> {
    apkg::apb1_select_key(
        chain,
        &claim.package_id,
        &claim.signer_id,
        claim.version,
    )
}

/// Recheck policy after cryptographic verification. APB1's complete immutable
/// digest is the revocation key; file names and package names never substitute
/// for it. `Workspace::verify` must have used the key returned by `select_key`.
pub fn check_eligible(chain: &Chain, bundle: &VerifiedBundle<'_>) -> Result<(), apkg::Error> {
    let claim = BundleClaim {
        application_id: *bundle.manifest().application_id(),
        package_id: *bundle.manifest().package_id(),
        version: bundle.manifest().version(),
        signer_id: *bundle.signer_id(),
    };
    check_claim_digest(chain, &claim, bundle.bundle_digest())
}

/// Apply the same current signer/minimum/revocation rules to a verified
/// durable-install receipt after `verify_installed` has rehashed its files.
pub fn check_claim_digest(
    chain: &Chain,
    claim: &BundleClaim,
    bundle_digest: &[u8; 32],
) -> Result<(), apkg::Error> {
    apkg::apb1_check_eligible(
        chain,
        &claim.package_id,
        &claim.signer_id,
        claim.version,
        bundle_digest,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use arena_phase84_crypto_audit::sha256;

    const ROOT_ID_BYTES: [u8; 32] = apkg::ROOT_ID;

    fn id(value: &[u8]) -> [u8; 32] {
        let mut out = [0; 32];
        out[..value.len()].copy_from_slice(value);
        out
    }

    fn claim(package: [u8; 32], signer_id: [u8; 32], version: u64) -> BundleClaim {
        BundleClaim {
            application_id: id(b"com.example.app"),
            package_id: package,
            version,
            signer_id,
        }
    }

    #[test]
    fn root_key_is_test_policy_only_and_namespace_is_checked() {
        let chain = Chain::new();
        let root_claim = claim(id(b"org.example.package"), ROOT_ID_BYTES, 1);
        assert_eq!(select_key(&chain, &root_claim), Ok(apkg::ROOT));

        let mut other = Chain::new();
        other
            .add(apkg::Policy {
                id: id(b"org.other.package"),
                generation: 1,
                subordinate: [7; 32],
                minimum: 1,
                allow: true,
                revoked_count: 0,
                revoked: [[0; 32]; 8],
            })
            .unwrap();
        assert_eq!(select_key(&other, &root_claim), Err(apkg::Error::Collision));
    }

    #[test]
    fn subordinate_authority_minimum_and_revocation_are_exact() {
        let package_id = id(b"org.example.package");
        let subordinate = [0x42; 32];
        let subordinate_id = sha256(&subordinate);
        let digest = [0x55; 32];
        let mut chain = Chain::new();
        chain
            .add(apkg::Policy {
                id: package_id,
                generation: 1,
                subordinate,
                minimum: 7,
                allow: true,
                revoked_count: 0,
                revoked: [[0; 32]; 8],
            })
            .unwrap();
        assert_eq!(
            select_key(&chain, &claim(package_id, subordinate_id, 7)),
            Ok(subordinate)
        );
        assert_eq!(
            select_key(&chain, &claim(package_id, subordinate_id, 6)),
            Err(apkg::Error::Downgrade)
        );
        assert_eq!(
            select_key(&chain, &claim(package_id, sha256(&[0x43; 32]), 7)),
            Err(apkg::Error::UnknownSigner)
        );
        assert_eq!(
            select_key(&chain, &claim(id(b"org.wrong.package"), subordinate_id, 7)),
            Err(apkg::Error::Collision)
        );

        let mut revoked = chain.current.unwrap();
        revoked.revoked_count = 1;
        revoked.revoked[0] = digest;
        let mut chain = Chain::new();
        chain.add(revoked).unwrap();
        assert!(chain.current.unwrap().is_revoked(&digest));
        assert_eq!(
            check_claim_digest(
                &chain,
                &claim(package_id, subordinate_id, 7),
                &digest,
            ),
            Err(apkg::Error::Revoked)
        );
    }
}
