# ADR-0081 — APB1 canonical signed application bundle format

Status: accepted and qualified for Phase 12. Format codec/host evidence is in
`userspace/arena-platform/` and `tools/test_apb1_format.py`; protected AFS2
install and receiver-policy handoff have guest evidence in
`docs/phase12/FINAL-REPORT.md`. Installed-app boot registry/launch remains
open for Phase 13.

Supersedes no historical format. APKG v1 remains byte-for-byte unchanged
(ADR-0053).

## Problem

Phase 12 needs a bounded, signed, multi-file native application artifact.
APKG v1 is an intentionally small single-payload staging record; widening,
reinterpreting, or silently changing it would break existing trust and test
contracts. The receiver must verify file paths, metadata, the complete file
set, and per-file contents without allocating a whole-application buffer.

The format must not turn a package ID, filename, manifest request, registry
entry, or signature-valid-but-untrusted key into launch or file authority.
The signing key is selected by an external receiver policy; this ADR specifies
the wire format and cryptographic binding, not the current signer allowlist,
installation transaction, activation state, or launch grant.

## Options considered

1. Extend APKG v1 with file tables or larger payload lengths. Rejected: changes
the meaning and accepted bounds of an established byte format.
2. Store a manifest and archive in a single signed opaque payload. Rejected:
   an opaque archive would require another canonical parser and makes exact
   per-file integrity/path review less direct.
3. Define APB1 with a fixed canonical manifest/table, signed expected file
   hashes, and a packed streaming payload. Chosen: bounded metadata, direct
   file accounting, and no whole-application scratch allocation.

## Decision

APB1 is a distinct format. All integers are unsigned little-endian. There is
no compression, encryption, link, symlink, sparse-file, implicit directory,
external-length, padding, or trailing-data interpretation in version 1.
Directories are implied by file paths during installation; each table row
names one regular file. Files are packed in table order with no gaps.

### Header and envelope

The exact byte layout is:

| Offset | Size | Field |
|---:|---:|---|
| 0 | 4 | ASCII magic `APB1` |
| 4 | 2 | format version, exactly 1 |
| 6 | 2 | header size, exactly 64 |
| 8 | 4 | manifest size, exactly 512 |
| 12 | 4 | file count, 1..=64 |
| 16 | 8 | table size, exactly `file_count * 144` |
| 24 | 8 | packed payload size, 1..=32 MiB |
| 32 | 32 | signer ID, `SHA-256(canonical_ed25519_public_key)` |

The 512-byte AMF1 manifest follows the header, then the file table, then one
64-byte Ed25519 signature, then the packed payload. The complete source length
must equal `64 + 512 + table_size + 64 + payload_size`; truncated and trailing
bytes both refuse. The signed metadata is the exact header, manifest, and file
table. Ed25519 verifies:

```text
"ArenaOS.application-bundle.v1\0" || header || manifest || file_table
```

The receiver first bounds and structurally validates metadata, checks that the
header signer ID is the SHA-256 fingerprint of the trusted key selected by
its external policy, and applies the existing audited canonical-key,
non-weak-key, strict Ed25519 verifier. A fingerprint is not a trust decision.

### AMF1 manifest (512 bytes)

| Offset | Size | Field / canonical rule |
|---:|---:|---|
| 0 | 4 | `AMF1` |
| 4 | 2 | manifest version, exactly 1 |
| 6 | 2 | manifest size, exactly 512 |
| 8 | 32 | application ID: 1..31 lower-case ASCII letters/digits/dot/hyphen, NUL then zero padding; first byte alphanumeric |
| 40 | 32 | package ID, same rule |
| 72 | 32 | display label: 1..31 printable ASCII bytes, NUL then zero padding |
| 104 | 8 | application version, nonzero |
| 112 | 4 | flags; only bit 0 multi-instance, bit 1 background, bit 2 headless |
| 116 | 4 | requested-capability hints; only bits 0 document-read, 1 document-write, 2 network-client, 3 persistent-background |
| 120 | 64 | entry path, 1..63 bytes then NUL/zero padding |
| 184 | 64 | optional icon path, either all zero or 1..63 bytes then NUL/zero padding |
| 248 | 2 | preferred width |
| 250 | 2 | preferred height |
| 252 | 4 | association count, 0..=8 |
| 256 | 256 | eight 32-byte content-type fields; active values are sorted, unique, lower-case ASCII MIME-like tokens of 1..31 bytes; inactive fields are all zero |

A non-headless window is 80..=1024 by 60..=768. A headless manifest may use
exactly 0×0 and must have the headless flag. Request bits and app flags are
metadata only: they never create capabilities, authorize launch, confer a
filesystem lineage, or trigger background execution. Unknown bits refuse.

### File table (144 bytes per row)

| Row offset | Size | Field / canonical rule |
|---:|---:|---|
| 0 | 1 | kind: 1 executable, 2 resource |
| 1 | 2 | path byte length, 1..=95 |
| 3 | 5 | reserved, all zero |
| 8 | 8 | exact file length, 1..=16 MiB |
| 16 | 32 | SHA-256 of exactly that file's payload bytes |
| 48 | 96 | path bytes, then NUL and zero padding |

Paths use printable ASCII relative components, at most 95 bytes, with `/` as
the sole separator. Absolute paths, empty/dot/dot-dot components, trailing
separator, control/space bytes, backslash, colon, and non-ASCII bytes refuse.
Rows are strictly bytewise sorted by unpadded path; duplicates and unsorted
rows refuse. The exact root-relative path `APB1.record` is reserved for the
signed-metadata/signature record in ADR-0082 and is refused as a payload path
before install mutation; `subdirectory/APB1.record` is an ordinary distinct
path. At least one executable must exist, and the manifest entry path must
name an executable row. A nonempty icon path must name a resource row.
The sum of row lengths must exactly equal payload size and may not exceed 32
MiB. The receiver checks every per-file SHA-256 while streaming payload data
through a 4,096-byte chunk. It also computes the digest of the exact complete
bundle bytes for immutable version identity.

### Receiver memory and trust boundaries

The current Rust implementation has a reusable fixed workspace: at most
9,822 bytes for domain plus signed metadata and 4,096 bytes for streaming I/O,
13,918 bytes total. It is caller-owned and must reside in static/BSS or
runtime-managed storage; it must not be allocated as a local on ArenaOS's
current one-page initial stack. File payloads are never copied into this
workspace as a whole.

`VerifiedBundle` proves only that a supplied key authenticated canonical
metadata and all listed file digests matched the source during verification.
It is not an installed record, a signer-policy decision, a durable readback,
an activation receipt, a held Image capability, or permission to launch. A
trusted receiver must keep the exact file source stable across verification
and installation (or verify readback from the transaction's immutable staged
objects) to prevent a source-swap/time-of-check gap.

## Consequences

- APKG v1 bytes, limits, public-key policy, and tests are untouched.
- APB1 decoding can be bounded and no-alloc; the 64-entry table determines a
  fixed metadata cap, and each payload read is at most 4 KiB.
- Host independent vectors include a valid three-file bundle, a 32 KiB
  streaming fixture, and correctly signed traversal/duplicate/prefix-conflict/
  reserved-record negatives.
- AFS2 transactional path creation, per-file readback, immutable activation,
  uninstall cleanup, accepted signer/policy selection, and a guest-side
  receiver proof are still required before APB1 can be used to install or
  launch an application.
- The format is native ArenaOS package metadata, not a Linux or POSIX ABI.
