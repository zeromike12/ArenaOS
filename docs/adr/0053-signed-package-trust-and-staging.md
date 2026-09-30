# ADR-0053 — Phase 8.4 signed package trust and staging (proposal)

Status: **Proposed for security/trust-model review** (2026-09-30). No 8.4 implementation or completion claim.
Milestone: 8.4 package format + signed packages; 8.5 installer/updater is separate.

## Problem and boundary

A filename, the creator of an AFS1 file, and possession of an fsd endpoint
are not evidence that executable bytes are authorized. Phase 8.4 requires
package identity, trust roots, update/revocation rules, signature verification,
host tamper and rollback tests, and a guest installation-verification boundary.
The kernel currently spawns only its explicitly embedded images via Image
caps. We must neither pretend an on-disk binary can already be executed nor
smuggle an install primitive into the kernel. AFS1 guarantees ordered writes
and atomic sectors for its own commit model, not anti-rollback or adversarial
commit-sector protection. A running guest has no trusted wall clock, TPM or
remote transparency log. ADR-0004 prohibits third-party crates in OS images.

## Proposed decision — review gates before architecture-dependent code

1. **Algorithm and key custody.** Offline host tooling signs with RSA-3072,
   SHA-256 and strict EMSA-PKCS1-v1_5 encoding (public exponent 65537).
   The guest holds only a fixed root public modulus, exponent and key-id
   fingerprint compiled into a privileged verifier image. Private keys are
   never built into the OS, disk fixture, test archive, or repository.
   Test keys are explicitly distinct from any production root and never
   advertised as a secure production signing authority. The no_std guest
   implementation must compare the *entire* encoded block, enforce a
   canonical 384-byte signature and verify against external OpenSSL vectors
   plus adversarial/cross-language differential tests; no permissive ASN.1
   parsing or variable-size big integers. No third-party crate enters the
   image. There is no claim that an unaudited in-house crypto primitive is
   production-certified; review must approve or replace this choice.
2. **Canonical package v1.** A self-delimiting, bounded container has a
   versioned fixed-size manifest (ASCII package ID <=31 bytes, target
   architecture, u64 version, exact payload length, SHA-256 payload digest,
   signer key ID, reserved bytes required zero), a bounded payload and one
   fixed 384-byte signature. Domain separation covers a canonical byte
   sequence including *all* manifest fields, digest and payload, not a
   user-controlled filename or an unbounded parser. Reject duplicate IDs,
   trailing bytes, noncanonical lengths, unknown versions/algorithms and
   integer overflow before cryptography. Select concrete offsets/maxima and
   publish independent host/guest vectors in this ADR *before* code.
3. **Authority and guest proof.** A bounded receiver-side staging verifier,
   not the shell's opinion or caller identity, checks bytes and trust policy
   before returning an accepted-install *eligibility* decision. It holds an
   explicit AFS1 endpoint and a private manager-issued approval marker;
   callers have only its request endpoint. A forged, missing, wrong-kind or
   insufficient-rights marker must fail at the receiver. Guest fixture
   injects a host-signed package onto an actual AFS1 disk and checks accept;
   altered payload, manifest, signature, root, wrong architecture and
   truncated/oversize packages refuse. The same service must refuse all
   attempts to stage an unverified package. Phase 8.4 does NOT grant an
   on-disk executable an Image cap, dynamically link it, activate it at boot
   or implement the 8.5 installer/update transaction.
4. **Update and revocation semantics.** A root-signed bounded policy record
   may authorize subordinate signing keys for a specific package namespace,
   with monotonically increasing *locally observed* policy generation and
   explicit key/package revocations. An accepted version/digest is a
   separate staging decision from an installed/active version. If a newer
   policy or installed version is visible on the same platter, an older
   package/policy refuses; a signed `REVOKE` denies all matching versions
   and surviving copies. With no new root-signed policy, fail closed for
   unknown keys, not open to arbitrary developer keys. Because AFS1 can be
   rolled back offline, **neither revocation nor version monotonicity is
   anti-rollback** across hostile disk replacement; removing this caveat
   would require a separately approved monotonic anchor/trust model. Root
   compromise or rotation requires a new trusted OS image: packages cannot
   certify their own new root. Host tamper/rollback tests must distinguish
   visible same-disk stale data refusal from excluded full-disk rollback.
5. **Evidence and release discipline.** Prove canonical codec/crypto vectors
   and negative space on the host and in a no_std build; run real guest
   verification, an absent/forged-authority proof, prior AFS1 crash and
   mediated-permission regressions, then all historical suites and a fresh
   image-bound 100/100 before calling any 8.4 checkpoint complete. Each
   commit remains bootable and keeps a qualified deployable image. The
   already published corrected 8.3 artifact remains the bootable build for
   this ADR-only design checkpoint; it is not an 8.4 implementation.

## Alternatives and trade-offs

- **Ed25519 via a vetted external verifier:** attractive smaller signatures
  and mature implementations, but an OS-image crate would require a material
  exception to ADR-0004 and separate vendoring/audit policy. Handwriting
  Ed25519 field/group arithmetic instead is more complex to review than a
  fixed-exponent RSA verifier. Choose only after an explicit change to the
  no-third-party rule.
- **Rely on UEFI Secure Boot, host OpenSSL alone or shell `sha256sum`:** none
  makes the guest's package receiver verify a signed artifact, and none
  provides package-level revocation. Host signing is tooling, not guest
  authority.
- **Verify in the kernel or add a `Package` cap:** broadens the frozen
  syscall/authority ABI for a format whose policy belongs in userspace;
  reject unless a later failure proves a genuine kernel enforcement need.
- **Claim tamper/rollback resistance from AFS1 persistence:** false. Its
  legal crash model and same-platter generation checks do not detect a
  copied old platter. Never turn a host disk-rollback test into a claim of
  full-disk anti-rollback.

## Required review before implementation

This proposal chooses a cryptographic algorithm, root location, delegation
and revocation trust model, and a persistent package/policy format. Those are
material security and persistent-format decisions under the project's
standing review rule. Approval must resolve: (a) whether to authorize a
bounded in-house RSA verifier or explicitly relax ADR-0004 for a vetted
Ed25519 implementation; (b) whether a test-only embedded root suffices for
8.4 guest proof, with production signing deferred to a separately secured
key ceremony; and (c) whether guest *verified staging* without executable
activation correctly separates 8.4 from the 8.5 installer/updater. Do not
implement signature parsing, key grants or persistent records until these
choices are accepted and canonical wire offsets are frozen in an ADR update.
