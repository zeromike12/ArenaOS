# ADR-0053 — Phase 8.4 signed packages and verified staging

Status: **Accepted — exact-source audit and offline/vector/fuzz gates passed; guest implementation subsequently qualified**
(2026-09-30). The 23 exact archive hashes, vendor/file hashes, feature/build/unsafe
review, no_std target compile and independent + coverage-guided host verifier/decoder
proofs are recorded in [the evidence ledger](0053-gate-evidence.md),
[unsafe review](0053-unsafe-review.md) and [fuzz evidence](0053-fuzz-evidence.md).
No material dependency, executed crypto backend, signed-wire or trust-model change was
required. Acceptance authorizes **implementation only**, not a completed/qualified
Phase 8.4 guest checkpoint. Its separate implementation, 47/47 historical suite,
final-artifact-bound 100/100 and extracted boot bundle are recorded in the
evidence ledger; Phase 8.4 is now complete **for verified staging only**.

## Scope and threat model

A file name, AFS1 ownership and a shell assertion are not signature or install authority.
Phase 8.4 defines package identity, a canonical signed format, bounded signer policy,
revocation/version semantics, and a **receiving userspace service** that verifies and
stages bytes. Phase 8.5 owns installation/activation, filesystem-backed execution,
Image-cap acquisition, upgrade transactions and rollback recovery. Phase 8.4 NEVER loads
or registers an on-disk executable, dynamically links code, gives a package an Image cap,
or changes a syscall/IPC primitive. The shell is the pre-existing trusted Power/raw-FS
administrator; an ordinary client holds only the staging endpoint. Neither a caller's
pid/name nor a filename authorizes a stage.

The cryptographic test root below is **public, test-only and deliberately known**; tests
prove parser, signature and receiver enforcement but do not establish production key
secrecy, Secure Boot or distribution integrity. No private production key enters the OS,
repository, guest disk or deployable artifact. An attacker with the *test* seed can sign
test packages; that is explicitly not a production security claim. Malicious userspace
without the admin marker or signing key, malformed inputs and same-disk crashes are
tested. A holder of the trusted shell's raw fsd cap or the root signing key is outside
this 8.4 threat model. AFS1 has ordered writes and atomic-sector commits, **not**
adversarial disk rollback/tamper or arbitrary commit-sector corruption protection; an
offline disk replacement can reinstate old policy/stages. No RTC/TPM/remote monotonic
anchor is invented.

## Cryptography, custody and bounded ADR-0004 exception

Use pure RFC 8032 Ed25519 (not Ed25519ph): `ed25519-dalek` **2.2.0**, candidate
`default-features = false`, guest **verification only** through
`VerifyingKey::verify_strict`, rejecting weak/noncanonical keys and signatures. Hash
payloads/key IDs with SHA-256 (`sha2` candidate **0.10.9**, defaults off). Signing is
offline host tooling; no guest RNG, key generation, private-key parsing, signing, batch,
PEM/PKCS#8, serde, alloc or std feature unless an exact audited dependency proves one
unavoidable. In particular, never enable `legacy_compatibility` or `hazmat`.

**On this ADR's acceptance**, ADR-0004 is narrowly amended for the exact pinned,
source-vendored Phase 8.4 cryptographic closure inside the userspace package verifier:
the Ed25519 verification implementation, SHA-256 directly used for payload/content hashes
and key IDs, and their required target/runtime dependencies. ADR-0004's zero-third-party
rule remains controlling everywhere else. This does not approve any changed version,
feature, backend, target or additional third-party dependency without a new review.

The first guest root is the RFC 8032 test vector 1 **public** 32 bytes
`d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a`; its key ID is SHA-256
of those exact raw bytes,
`21fe31dfa154a261626bf854046fd2271b7bed4b6abe45aa58877ef47f9721b9`. The corresponding
widely published seed is usable only in host fixture generation, never in the image/ESP,
guest disk or archive, and is not a production secret. No root rotation by a package:
replacing/revoking the root requires a separately trusted OS image and a future
production-key ceremony (generation, offline custody, release procedures and compromise
response are NOT 8.4 claims).

**Dependency acceptance gate:** obtain the exact versioned sources of **every**
target/runtime and host-build dependency, vendor them in the repo, pin a resolved lockfile
and enabled target features, and record per-source SHA-256, upstream revision, license and
relationship to crates.io archive checksums. Inspect all unsafe blocks, build scripts,
proc macros and feature unification, including target-specific backends; forbid network
access and runtime code generation. Demonstrate an offline/reproducible
`x86_64-unknown-none` build using only the vendored sources. Validate RFC 8032 vectors,
independent OpenSSL-compatible signature results and malformed/noncanonical key/signature
cases; fuzz the format before accepting this ADR. **Completed for the pinned serial
x86_64-unknown-none configuration:** source/feature/unsafe review, clean offline
no_std build, independent OpenSSL/RFC vectors and both Python-transition and
LLVM-Rust-region coverage-guided fuzz campaigns. See
[0053-gate-evidence.md](0053-gate-evidence.md). Changing this configuration
reopens the gate; this acceptance does not claim guest end-to-end proof.

## Frozen candidate package v1 bytes

One file is **exactly** `manifest[128] || payload[n] || signature[64]`, `1 <= n <= 4096`,
total length `192+n` (193..4288 bytes). Integers are little-endian; no Rust/native struct
layout, optional fields, trailing bytes or implicit NUL parsing. Parse lengths with
checked arithmetic before reading/allocating or verifying. The canonical package ID field
is 32 bytes: 1..31 bytes lowercase ASCII `[a-z0-9.-]`, first byte `[a-z0-9]`, then one NUL
followed by zeros. It is not a filesystem path and has no identity authority.

| Byte interval | Field | Mandatory validation |
|---|---|---|
| 0..4 | magic | `APKG` |
| 4..6 | u16 format | `1` |
| 6..8 | u16 header bytes | `128` |
| 8..10 | u16 target | `1` = `x86_64-unknown-none` only |
| 10..12 | u16 flags | zero |
| 12..44 | package ID | canonical 32-byte form above |
| 44..52 | u64 package version | >=1 |
| 52..56 | u32 payload length | 1..4096; file size exactly `192+n` |
| 56..88 | SHA-256 payload digest | hash of all `n` exact payload bytes |
| 88..120 | SHA-256 signer key ID | hash of the exact 32-byte raw Ed25519 public key |
| 120..128 | reserved | all zero |
| 128..128+n | payload | opaque bytes; NEVER activated by 8.4 |
| 128+n..192+n | Ed25519 signature | exactly 64 bytes, strict canonical verification |

Ed25519 signs/verifies the exact **raw message** `b"ArenaOS.pkg.v1\x00" (15 bytes) ||
manifest[0..128] || payload[0..n]`. This is pure Ed25519 over that concatenation, **not**
a prehash/signature over a hex digest, and the signature itself is excluded. Verify the
signed payload digest as an independent corruption check and compare key IDs to the chosen
trust root or a root-signed, currently authorized subordinate key. An unknown key ID,
wrong root, wrong arch, different payload/signature/manifest bytes, malformed padding or
extra data is a refusal. Two signed files with identical ID+version but different
full-package SHA-256 are a conflict, not an update.

## Frozen candidate policy v1 bytes and revocation

A policy is **exactly 512 bytes** (`header[448] || root_signature[64]`). All integers LE.
Only the fixed root may sign it. One namespace has at most four immutable policy
generations; a policy authorizes **one** subordinate public key for that exact package ID
at a time. The key does not authorize a different ID even if deliberately transferred.
There is no unsigned fallback or automatic trust-on-first-use.

| Byte interval | Field | Mandatory validation |
|---|---|---|
| 0..4 | magic | `APOL` |
| 4..6 | u16 format | `1` |
| 6..8 | u16 signed header bytes | `448` |
| 8..16 | u64 generation | 1..4, contiguous across visible records |
| 16..48 | namespace ID | same canonical 32-byte package ID |
| 48..80 | subordinate raw Ed25519 public key | strictly parseable, not root key |
| 80..112 | subordinate key ID | SHA-256 of 48..80, byte-exact |
| 112..120 | u64 minimum package version | >=1, never decreases vs predecessor |
| 120 | state u8 | 1=ALLOW this subordinate; 2=REVOKE subordinate signing for this namespace |
| 121 | revocation count u8 | 0..8 |
| 122..128 | reserved | all zero |
| 128..384 | eight SHA-256 full-package-file digest slots | first `count` nonzero, unique, strictly ascending; remaining slots zero |
| 384..448 | reserved | all zero |
| 448..512 | root Ed25519 signature | strict, over exact header and domain |

The root signs `b"ArenaOS.policy.v1\x00" (18 bytes) || policy[0..448]`. Policy signatures
are never signed by the delegated key. A revoked digest is SHA-256 of the **entire**
package file including its 64-byte signature, not the payload digest at manifest 56..88.
Each new policy preserves the previous record's revoked-digest set as a subset and cannot
decrease minimum version; a previously REVOKEd or replaced subordinate key may **never**
become active again in this visible sequence. A newer ALLOW may replace a revoked key with
a *new* key. A REVOKE state denies every subordinate-signed package of that namespace; the
cumulative digest list also denies matching root-direct packages. A root-direct package is
allowed without policy for the positive test-root case, but if any policy is visible for
that namespace, the root-direct package must still satisfy minimum version and cumulative
digest revocation. No certificate chain, time-based expiry or remote policy is claimed.

Visible policy files are named `p8-` + the first **20 lowercase hex digits** of
SHA-256(full canonical package ID field) + `-01`..`-04` (26 bytes, below AFS1's 32-byte
name limit). A package input file is `i8-` + those digits, and a policy intent is `a8-` +
those digits (each 23 bytes). Immutable accepted stage files use `s8-` + those digits +
`-01`..`-02` (26 bytes). Each file's *internal signed ID* is checked in full: a truncated
filename-hash collision is a **typed collision refusal**, not an alias or a second
identity. Namespace prefixes are disjoint from existing `cfg8-*` and `perm8-*` records. At
boot and before each decision, use bounded `FS_OP_LS` to inspect **every** visible `p8-*`
and `s8-*` name, rejecting unrecognized suffixes, multiple namespaces, gaps,
duplicate/misnamed records, truncated-ID hash collisions and malformed file contents. With
no visible accepted files the first marked operation selects the sole namespace;
thereafter a different ID refuses. Enforce the one-namespace budget even if the file was
written by another raw-FS holder. Reverify every policy chain member under the root. Check
each stage predecessor's exact bytes and signature against a historically valid signer
from that chain (or the root), and its strictly increasing version; a prior stage later
revoked is structurally valid but **ineligible**. Only the latest stage under the
**current** policy can yield ELIGIBLE. A malformed, missing or out-of-order visible
predecessor is an error; never fall back to an older ALLOW or stage. Fifth policy / third
staged package returns typed NO_SPACE before CREATE without eviction; clean
pre-CREATE object/sector refusals also return NO_SPACE. The accepted eight-slot signed
wire cannot represent a ninth distinct digest: by explicit user decision on 2026-09-30,
the ninth-digest capacity proof is a typed **issuer-side** `NoSpace` before any mutation
or signature, plus real guest verification of an authentic root-signed eight-digest
policy. This does *not* assert a ninth-digest guest request; extending APOL v1 padding
or the public IPC ABI would require a separate signed-wire decision. Late write/commit space exhaustion
is ambiguous and returns DEGRADED instead. With at most 4 policy, 2 stage and 2 input
objects for the **single test namespace** in 8.4, at most 8 new AFS1 files are
budgeted; prove the object-count preflight alongside existing 8.1/8.2 records, rely on fsd for actual disk-sector refusal
(there is no public free-sector query), and never raise their bounds or overwrite them.
General multi-package storage/GC belongs to 8.5.

## Receiver-side staging and lifecycle authority

One manager-owned `packaged` verifier image (proposed image ID **26** after existing
0..25) serves **one** package endpoint in **one** receive loop. No new syscall, IPC
select, thread, kernel Package cap or disk-image loader. It holds the test root's *public
bytes* in its own image and obtains no private key. It reads candidate package bytes from
the `i8-*` input on the actual fsd/AFS1 device; input may be host-seeded for the 8.4 guest
fixture or written by the already trusted shell. It never trusts the input filename as
identity. The ordinary caller owns only Endpoint/WRITE; it cannot read fsd or stage by
knowing a name. Only the trusted Power/raw-FS shell holds the separate stage-approval
marker. The existing 8.2 permission marker is **not** reused: doing so would silently
broaden authority held by its independent copies.

The 64-byte IPC request has `words[0]` opcode (`0=PING`, `1=QUERY`, `2=POLICY`,
`3=STAGE`), `words[1]=0`, and `msg[0..32]` the canonical ID (all zeros only for PING),
`msg[32..64]=0`. Refuse unknown opcodes, nonzero padding and unexpected sent caps even for
read-only requests. Replies NEVER transfer a cap: `words[0]` is `0=OK` (PING/POLICY),
`1=ELIGIBLE`, `2=UNSET`, `3=INELIGIBLE`, or two's-complement `-1=BAD_FORMAT`, `-2=DENY`,
`-3=NO_SPACE`, `-4=DEGRADED`, `-5=CORRUPT`, `-6=COLLISION`, `-7=OFFLINE`; syscall
transport error is separate. Reply `words[1]` is zero except PING magic `0x504b4731`,
POLICY committed generation or ELIGIBLE version. All reply bytes are zero except ELIGIBLE
`msg[0..32]` = full-package SHA-256. Probe validation requires all PING words, zero reply
bytes and **no** returned cap.

A **new** root-issued Notification object X is the disjoint marker. The receiver's
anchor is exactly Notification / X / READ; the trusted admin's source cap is exactly
Notification / X / READ|COPY|DESTROY. IPC transfer preserves source rights: a **valid
landed marker** must describe as exactly Notification / the *same X* /
READ|COPY|DESTROY, following ADR-0047's `take_diagnostic` model. Compare the two object
IDs for equality but **do not compare their rights masks for equality**; compare each
rights mask with its own literal expected mask. Independently destroy **every** landed
reference on all request paths regardless of validity, including wrong-kind/wrong-rights
and calls not requiring a marker; preserve the no-cap landing baseline. The receiver
checks the signature/policy on each accepted staging operation, not only on boot. `QUERY`
requires Endpoint/WRITE and verifies the latest visible staged bytes/policy afresh but
changes nothing; it cannot approve staging. `STAGE` and `POLICY` require a transferred
marker as well as Endpoint/WRITE. `POLICY` reads the root-signed `a8-*` intent for the
requested ID, checks the full signed chain including exact next generation/cumulative
revocation, writes the next immutable policy file, closes, rescans all visible
predecessors and acknowledges only the byte-exact newly validated generation. `STAGE`
first validates the requested `i8-*` input under the latest policy and current version
bound, creates the next immutable `s8-*` file, writes **all** exact bytes in bounded fsd
chunks, closes, rereads and revalidates it and policy, then returns `ELIGIBLE` with
package version + full SHA-256 as a **userspace decision**, not Image or execution
authority. The stage scan must reject incomplete/malformed newest files, not silently
return a former stage. Repeating the identical verified package is an idempotent no-op;
equal version with different digest or lower version refuses. Later policy REVOKE makes
any previous staged copy ineligible on the next verification. Use <=3584-byte fsd
READ/WRITE chunks (`FS_XFER_MAX`), checked offsets, and one handle at a time (AFS1 has
only eight open handles). `POLICY`/`STAGE` IO ambiguity **after CREATE or WRITE begins**,
including a late NO_SPACE, latches DEGRADED until a same-disk restart/rescan, never
acknowledges uncertain bytes; a proven pre-CREATE capacity refusal is typed NO_SPACE
instead. A newly visible partial highest generation is CORRUPT after reboot, never an old
success. A crash mid-write can leave incomplete visible bytes: fail closed after reboot.
8.4 does **not** promise old stage availability during interrupted replacement; 8.5 owns
activation/upgrade atomicity. Staged files and reply words are never independent
authentication proof for a future installer, which must verify again at its receiver.

The four states are deliberately distinct: **signature-valid** means bytes verify under a
known key; **stage-eligible** additionally satisfies current namespace policy, minimum
version, revocation and monotonic stage checks; **installed** and **active** are both
unreachable in 8.4. ELIGIBLE is not an installed/active status and confers no authority to
execute.

**Exact proposed grants and fixed bounds:** `packaged` inherits slot 0 fsd Endpoint/WRITE,
slot 1 package Endpoint/READ, slot 2 marker Notification/READ — **three** grants, no
Power/Process/Image, no broker/app authority. The manager already holds fsd
Endpoint/WRITE|COPY at source slot 14; root additionally grants image26/READ at manager
slot 17, package Endpoint/READ|WRITE|COPY at 18, marker Notification/READ|COPY at 19. The
shell receives package Endpoint/WRITE|COPY at slot 20 and marker
Notification/READ|COPY|DESTROY at 21; app/permission broker receive neither. Slots 0..19
of the existing shell and 0..16 of the manager retain their meanings. The manager's
existing private result/exit/deadline notification receives disjoint package-readiness
OK/EXIT/DEADLINE bits (proposed bits 15, 16, 17, distinct from 8.2 bits 12..14); a bounded
probe worker receives only package Endpoint/WRITE and attenuated WRITE to that existing
notification (two grants), sends PING, validates the exact reply, and exits. Manager
requires success **plus actual exit before deadline**, failure/deadline wins; it reaps
through its held Process cap and has bounded restart/backoff and OFFLINE on absent fsd or
malformed visible policy. `spawn::MAX_INHERIT=5`, manager `MAX_GRANTS=5`, 32 cap slots and
manager `MAX_CAPS=32` stay fixed. The package service is a third managed service within
`MAX_SERVICES=4`; no fabricated readiness or raw FS cap in an ordinary client.

The full 8.3 boot has exactly **9/9 endpoints and 16/16 Notifications**. This new endpoint
and distinct marker therefore require a **bounded internal** increase to
`MAX_ENDPOINTS=10` and `MAX_NOTIFS=17`, never a hidden reuse of permission authority or an
unlimited allocation. `Endpoint` is 976 B, `Notif` 24 B on x86_64 by the current layout
(measure again in the implementation): +1000 B static IPC tables plus 4 more wake-list
thread IDs (+32 B stack), before alignment/frame effects. The image registry needs one
additional literal mapping for ID26 (its historical `MAX_IMAGES=24` comment is already
stale relative to current 0..25 mappings; correct the bound to 27 if enforced). Re-prove
exact 10/10, 17/17 full-table refusal, missing-device SKIP, exact free
frames/records/processes and historical failure semantics; do not merely rewrite an old
assertion. If measured limits differ, revisit this ADR before granting authority. These
are capacity changes, **not** new kernel primitives or rights.

## Test matrix, qualification and acceptance gates

* **Pure bytes/crypto:** independent Python/host Rust reference for package/policy
  offsets, little-endian, signed domains and key IDs; RFC 8032 vectors 1+, OpenSSL/dalek
  cross-verification, key substitution, weak/noncanonical signatures, overflow, unknown
  target, flags, reserved bytes, wrong lengths/trailing data, ID collisions and fuzz
  corpus. Confirm no guest signing feature or network/build-time code generation in the
  vendored closure; record exact versions/hashes/licenses/revisions and audit
  unsafe/build/proc macros. Bare-metal no_std compile and no default features.
* **Structural authority:** in a real QEMU guest, no marker, wrong object/kind/rights,
  guessed slot, endpoint-only and copied app endpoint cannot stage or change policy; valid
  root-signed input with genuine transferred marker is accepted **only** at the receiving
  service. Ensure every landed marker is disposed and 32-slot occupancy stays flat,
  including rejection and restart. Verified output is a digest/version decision, not an
  Image cap. Deliberately break signature or receiver marker check once: focused test MUST
  go red before trusting its green verdict.
* **Persistence/negative space:** actual AFS1 host-seeded package and policy, durable stage and same-platter reboot/rescan;
  subordinate ALLOW, test root-direct acceptance, unknown signer, wrong embedded root,
  altered/truncated manifest/payload/signature, wrong namespace/key, revoked subordinate,
  revoked package digest, version downgrade, identical idempotent retry, equal-version
  conflicting digest, policy-chain gap/corruption, fourth policy/second stage successes
  followed by fifth/third guest typed NO_SPACE; root-signed eight-digest policy accepted
  in guest plus issuer-side ninth-distinct-digest typed NO_SPACE before mutation/signing
  (not a guest-ninth proof, per explicit 2026-09-30 user decision); disk allocation exhaustion, service
  crash/restart and SIGKILL across CREATE/WRITE/CLOSE/reply boundaries. Every ambiguous
  prefix is audited against the documented AFS1 crash model and fails closed; never claim
  hostile full-disk anti-rollback. Old 8.1 configuration and 8.2 permission disk
  namespaces and exact guest counts must stay green.
* **Closure:** preserve all historical suites, run the full suite on the final source, rebuild the final EFI,
  perform **fresh artifact-bound 100/100 QEMU boots**, verify receipt against EFI inside
  ESP, package exact firmware/AFS1 disk/QEMU instructions, checksum and boot the extracted
  archive. A review-only ADR commit reuses the published corrected Phase 8.3 image without
  claiming an 8.4 checkpoint; do not ship a partial implementation as a completed phase.

## Dependency reconnaissance — historical pre-acquisition snapshot

**Update 2026-09-30:** the 23 exact archives were subsequently supplied by the user and independently verified, extracted and committed under `vendor/phase84`; the earlier CDN/cache-blocker statements in this section describe the situation *before* that acquisition. Current accepted audit evidence is tracked in [0053-gate-evidence.md](0053-gate-evidence.md) and [0053-source-audit.md](0053-source-audit.md). Guest implementation and checkpoint qualification were completed separately after ADR acceptance; see the evidence ledger.

Candidate: `ed25519-dalek = { version = "=2.2.0", default-features = false }` and `sha2 =
{ version = "=0.10.9", default-features = false }` targeting `x86_64-unknown-none`. The
upstream dalek monorepo tag `ed25519-2.2.0` at
https://github.com/dalek-cryptography/curve25519-dalek resolves to Git commit
`8016d6d9b9cdbaa681f24147e0b9377cc8cef934` and contains `ed25519-dalek` 2.2.0
(BSD-3-Clause) and `curve25519-dalek-derive` 0.1.1 (MIT OR Apache-2.0), but its colocated
`curve25519-dalek` is **4.2.0 while the registry lock resolved 4.1.3**. Never treat that
tag as proof of 4.1.3's published source. The resolved candidate lock contains 23 registry
packages (some target/backend conditional); derive is a host proc macro depending on
`syn`, `quote`, `proc-macro2`, and curve's build script depends on `rustc_version`. This
is **not** a complete feature/unsafe/license/vendor audit. Its source archive registry
SHA-256 values are listed below for provenance only; these are not hashes of downloaded
and reviewed vendored source:

| Registry package | Pinned candidate | Registry archive SHA-256 (not vendored source hash) |
|---|---:|---|
| `block-buffer` | `0.10.4` | `3078c7629b62d3f0439517fa394996acacc5cbc91c5a20d8c658e77abd503a71` |
| `cfg-if` | `1.0.5` | `4e7648175b45a9a48536d676f68d918270699102aa8dab5496df06904c914600` |
| `cpufeatures` | `0.2.17` | `59ed5838eebb26a2bb2e58f6d5b5316989ae9d08bab10e0e6d103e656d1b0280` |
| `crypto-common` | `0.1.7` | `78c8292055d1c1df0cce5d180393dc8cce0abec0a7102adb6c7b1eef6016d60a` |
| `curve25519-dalek` | `4.1.3` | `97fb8b7c4503de7d6ae7b42ab72a5a59857b4c937ec27a3d4539dba95b5ab2be` |
| `curve25519-dalek-derive` | `0.1.1` | `f46882e17999c6cc590af592290432be3bce0428cb0d5f8b6715e4dc7b383eb3` |
| `digest` | `0.10.7` | `9ed9a281f7bc9b7576e61468ba615a66a5c8cfdff42420a70aa82701a3b1e292` |
| `ed25519` | `2.2.3` | `115531babc129696a58c64a4fef0a8bf9e9698629fb97e9e40767d235cfbcd53` |
| `ed25519-dalek` | `2.2.0` | `70e796c081cee67dc755e1a36a0a172b897fab85fc3f6bc48307991f64e4eca9` |
| `fiat-crypto` | `0.2.9` | `28dea519a9695b9977216879a3ebfddf92f1c08c05d984f8996aecd6ecdc811d` |
| `generic-array` | `0.14.7` | `85649ca51fd72272d7821adaf274ad91c288277713d9c18820d8499a7ff69e9a` |
| `libc` | `0.2.189` | `3eaf3ede3fee6db1a4c2ee091bf8a8b4dccdc6d17f656fb07896ee72867612f2` |
| `proc-macro2` | `1.0.107` | `985e7ec9bb745e6ce6535b544d84d6cd6f7ad8bd711c398938ae983b91a766d9` |
| `quote` | `1.0.47` | `1fbf4db142a473a8d80c26bbf18454ed458bf8d26c8219c331daecfdbd079001` |
| `rustc_version` | `0.4.1` | `cfcb3a22ef46e85b45de6ee7e79d063319ebb6594faafcf1c225ea92ab6e9b92` |
| `semver` | `1.0.28` | `8a7852d02fc848982e0c167ef163aaff9cd91dc640ba85e263cb1ce46fae51cd` |
| `sha2` | `0.10.9` | `a7507d819769d01a365ab707794a4084392c824f54a7a6a7862f8c3d0892b283` |
| `signature` | `2.2.0` | `77549399552de45a898a580c1b41d445bf730df867cc44e6c0233bbc4b8329de` |
| `subtle` | `2.6.1` | `13c2bddecc57b384dee18652358fb23172facb8a2c51ccc10d74c157bdea3292` |
| `syn` | `2.0.119` | `872831b642d1a07999a962a351ed35b955ea2cfc8f3862091e2a240a84f17297` |
| `typenum` | `1.20.1` | `b6f5e870be6c3b371b77fe0ee0bafb859fa4964b4404c27de1d380043c4dda20` |
| `unicode-ident` | `1.0.26` | `d245f478577f809a851594d02313b640fb437e0bb33866753cff937863096954` |
| `version_check` | `0.9.5` | `0b928f33d975fc6ad9f86c8f283853ad26bdd5b10b7f1542aa2fa15e2289105a` |


**Historical pre-acquisition note:** crates.io/mirror TLS failed in this sandbox, so the GitHub-hosted index initially supplied only registry archive checksums. The subsequently user-supplied exact archives authenticated against that preexisting index, were vendored with independent file hashes, and were built offline as recorded in the acceptance ledger. Do not replace them with a GitHub HEAD/tag. Acceptance of the source/host gates does not claim a qualified guest image.

## Rejected alternatives and future rules

* Handwritten RSA/Ed25519 arithmetic creates an unaudited cryptographic TCB; a narrowly
  pinned verifier closure is preferred **subject to the explicit audit gate**. A blanket
  third-party OS dependency exception is not accepted.
* Host-only signature check, unsigned SHA-256, caller identity, AFS1 filename or a
  signed success message from an untrusted client cannot replace receiver verification and a held admin marker.
* Kernel Package caps, new syscall, on-disk ELF activation, dynamic linking and
  production root/secure boot ceremony are not justified by 8.4; 8.5 or a separate approved decision
  must take responsibility if actually required.
* Complete-platter rollback, arbitrary commit-sector corruption, hostile raw-FS
  writer, root-key compromise and signed package distribution security are **not** implied by 8.4's test-root and AFS1 crash-model
  proofs.
