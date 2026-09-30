# ADR-0053 — Phase 8.4 signed package trust and staging (proposal)

Status: **Proposed, design direction reviewed** (2026-09-30). The Ed25519/test-root/staging boundaries are approved in principle; dependency audit, exact pinned source hashes and frozen package/policy bytes remain acceptance gates. No 8.4 implementation or completion claim.
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

1. **Algorithm and key custody.** Offline host tooling signs pure Ed25519
   (RFC 8032, not Ed25519ph). The guest holds only a fixed **test-only**
   32-byte root public key and its SHA-256 key ID in a privileged verifier
   image. No private key belongs in any OS image, guest disk or deployable
   checkpoint; a deterministic host-only fixture key must never be reused
   as a production signing root. Guest verification uses one pinned,
   vendored, vetted pure-Rust no_std implementation, tentatively
   `ed25519-dalek` 2.2.0 with `default-features = false`, invoking
   `VerifyingKey::verify_strict` so weak keys and noncanonical signatures
   refuse. This is a **narrow exception to ADR-0004**, not a blanket
   invitation to ship dependencies. The complete transitive *runtime*
   closure, enabled features, licenses, checksums, upstream revisions,
   unsafe/build-script/proc-macro inventory and source vendoring must be
   frozen and reviewed before this ADR is Accepted or code depends on it.
   Do not claim an audit without evidence for the exact version/config.
   Prohibit runtime network access, code generation and guest signing,
   key generation, RNG, PEM/PKCS#8, batch, serde, default or unnecessary
   features. Offline signing may use separate trusted host tooling;
   production release-key custody is a later security decision. Proposed
   non-secret **test** root is RFC 8032 test vector 1 public key
   `d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a`;
   its raw-public-key SHA-256 key ID is
   `21fe31dfa154a261626bf854046fd2271b7bed4b6abe45aa58877ef47f9721b9`.
   The widely published RFC test seed is host-test-only material; these
   known bytes afford *zero* production signing security.
2. **Canonical package v1 (frozen candidate bytes).** A file is exactly
   `manifest[128] || payload[payload_len] || signature[64]`; there is no
   trailing data. Little-endian integers, no native struct layout:

   | Bytes | Field | Canonical rule |
   |---|---|---|
   | 0..4 | magic | ASCII `APKG` |
   | 4..6 | format version, u16 | exactly 1 |
   | 6..8 | manifest length, u16 | exactly 128 |
   | 8..10 | target architecture, u16 | exactly 1 = x86_64-unknown-none |
   | 10..12 | flags, u16 | exactly 0 |
   | 12..44 | package ID, 32 bytes | 1..31 lowercase ASCII `[a-z0-9.-]`, first byte alphanumeric; first NUL and all trailing bytes zero |
   | 44..52 | package version, u64 | >=1 |
   | 52..56 | payload length, u32 | 1..4096, exact file length 192 + payload length |
   | 56..88 | payload SHA-256 | exact hash of payload bytes |
   | 88..120 | signer key ID | SHA-256 of raw 32-byte Ed25519 public key |
   | 120..128 | reserved | all zero |

   Pure Ed25519 signs/verifies **exactly** `b"ArenaOS.pkg.v1\x00" ||
   manifest[0..128] || payload[0..payload_len]`, with no prehash mode.
   The signature is the last 64 bytes and is not itself signed. Check all
   lengths with checked arithmetic, reject noncanonical bytes, bad IDs,
   unknown algorithms/architectures and truncated/extra bytes *before*
   signature verification. Identical package IDs are compared using the
   full 32-byte canonical field, never an AFS1 filename or a truncated
   digest. Exact host/guest vectors and external OpenSSL cross-checks are
   required before implementation acceptance.

   A separate root-signed **policy v1** is exactly 320 bytes: bytes 0..4
   `APOL`, 4..6 u16 version=1, 6..8 u16 header length=256, 8..16 u64
   generation>=1, 16..48 the same canonical namespace ID, 48..80 raw
   subordinate Ed25519 public key, 80..112 its SHA-256 key ID,
   112..120 u64 minimum accepted package version>=1, 120..152 optional
   revoked package digest (all-zero means none), 152 u8 state (1=ALLOW,
   2=REVOKE subordinate), 153..256 all zero; 256..320 is a 64-byte
   **root** Ed25519 signature over `b"ArenaOS.policy.v1\x00" ||
   policy[0..256]`. Exactly one active subordinate key per namespace,
   no arbitrary key lists or unsigned policy fallback. REVOKE invalidates
   every package from that key even if a bearer or old file was copied.
   A root-direct package is permitted for the positive test-root case;
   root rotation/revocation requires a new trusted OS image. An offline
   replacement of the entire policy/disk can defeat local generation
   checks: no anti-rollback claim.
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

## Dependency reconnaissance (NOT the completed audit)

`ed25519-dalek` 2.2.0's published API documents `no_std` with default
features disabled and `verify_strict` for weak-key rejection; the upstream
monorepo tag `ed25519-2.2.0` resolves to Git commit
`8016d6d9b9cdbaa681f24147e0b9377cc8cef934` at
https://github.com/dalek-cryptography/curve25519-dalek . Its source tree
contains `ed25519-dalek` 2.2.0 (BSD-3-Clause), `curve25519-dalek` 4.2.0
(BSD-3-Clause) and `curve25519-dalek-derive` 0.1.1 (MIT OR Apache-2.0).
The dalek dependency is not a single crate: its runtime dependencies
include the `ed25519`, `sha2`, `subtle`, `digest`, `cfg-if` and target-specific
`cpufeatures` crates; curve25519's x86_64 backend also pulls in a **proc
macro** (`curve25519-dalek-derive`) and its build script uses
`rustc_version`. The proc macro's `syn`, `quote`, `proc-macro2` closure
must be reviewed even though it is built for the host, not loaded at guest
runtime. This list is preliminary, **not** a complete pinned dependency
graph, feature audit or source-vendoring claim. An audit-only bare-metal
Cargo probe using `ed25519-dalek = { version = "=2.2.0",
default-features = false }` and `sha2 = { version = "=0.10.9",
default-features = false }` was blocked by TLS errors to the crates.io
index/static CDN in this sandbox; no binary or signature code was
substituted or shipped. Fetch the *exact* versioned crate sources via a
verifiable channel (or map complete upstream revisions), vendor each
package and its licenses, record source SHA-256s and the resolved lockfile,
then audit the complete enabled closure **before** accepting this ADR or
using a dependency in an OS image. The older, qualified 8.3 EFI and archive
remain unchanged while this design is reviewed.

## Alternatives and trade-offs

- **In-house RSA-3072 or Ed25519 arithmetic:** avoids a dependency but
  creates a large unaudited cryptographic TCB and new parser/malleability
  risks. Rejected in favor of a *narrow, pinned, vendored* Ed25519-verifier
  exception to ADR-0004, subject to the exact dependency gate below.
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

The material direction has been reviewed: allow **only** a pinned/vendored
vetted Ed25519 verifier and its audited runtime dependency closure as a
scoped ADR-0004 exception; use a clearly non-production public root for
guest tests; keep verified staging distinct from 8.5 activation. This is
NOT a claim that the exact dependency has been audited. Before ADR
acceptance, enumerate every runtime crate/feature, pin and vendor exact
sources, record SHA-256/license/upstream revision, inspect unsafe uses and
build scripts/proc macros, and forbid network/runtime code generation.
Cross-check RFC 8032 vectors and independently implemented signatures;
fuzz malformed keys/signatures/packages and run actual QEMU negative-space
proofs. Freeze and independently test the precise canonical package/policy
bytes and signature domain above. Production root creation, custody,
rotation, compromise response and release signing ceremony require a
separate pre-production security decision; a public RFC test-vector seed
used by host fixtures is not a secret or a production key and must NEVER
be put in the guest binary/disk. Until these audits and vectors are done,
this ADR remains Proposed and no architecture-dependent 8.4 code may ship.
