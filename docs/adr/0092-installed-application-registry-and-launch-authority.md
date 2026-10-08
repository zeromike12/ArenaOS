# ADR-0092 — Installed application registry and launch authority

**Status:** Accepted; installed-registry launch guest and Phase-13 T4 passed.
**Date:** 2026-10-07.
**Related decisions:** ADR-0053, ADR-0080, ADR-0081, ADR-0082, ADR-0083,
ADR-0086, ADR-0090, ADR-0091.

## Context

Phase 12 installs immutable APB1 version trees below the protected AFS2
`/System/Applications` root. It also has a bounded `AppRegistry` and an
installed-tree verifier in `arena-platform`, but no boot service invokes
them. `packaged` owns the receiver-verified APKG v1 signer policy and the
kernel ImageRegistrar capability. `filesd` owns the only AFS2 volume and the
private install roots. Desktop currently launches six compiled-in app kinds.

An app ID, package ID, version, installed path, manifest entry path, or
executable file name cannot authorize process creation. A launch decision
must bind the current policy, exact immutable installed tree, exact
executable bytes, and a live kernel Image capability.

## Decision

### Ownership split

- `filesd` remains the only owner of the protected AFS2 volume and root
  object IDs. The ordinary `/Users/user` Files endpoint cannot traverse or
  name `/System/Applications`.
- The existing internal `R_INSTALL` endpoint remains the only route from
  `packaged` to APB1 install state. Extend that endpoint with bounded
  read-only installed-catalog operations. They accept no caller path and do
  not return a general `/System` capability.
- `filesd` enumerates candidate app/version directories and returns only
  unauthenticated claims until `packaged` selects a key from its current
  receiver-verified APKG v1 policy chain.
- `filesd` then re-verifies the exact installed APB1 record, signature,
  payload hashes, and complete installed tree under that key before returning
  descriptive manifest fields and the executable bytes. The protected root
  is immutable to ordinary users; the filesd request loop serializes this
  verification against install/retirement operations.
- `packaged` checks current signer eligibility and constructs/publishes the
  bounded descriptive catalog. It resolves launch requests through the
  current catalog and repeats policy/tree verification before minting a
  transient Image. Desktop receives only that exact Image capability and
  uses the existing Startup ABI v2 launch path.
- Installed registry records and launch replies remain separate. The
  catalog contains descriptive manifest metadata; only a live exact Image
  capability can be supplied to spawn. Replacements invalidate old
  descriptive generations and old Images are revoked before another launch
  can resolve the same application identity.

### Executable contract

- The manifest entry path must resolve to exactly one signed APB1 executable
  file in the installed immutable version tree.
- APKG v1 policy files continue to select signer authority; APB1 signature
  and whole-bundle digest continue to authenticate application contents.
  APKG policy format and APB1 install records remain distinct.
- The launch service hashes and validates the exact entry bytes it passes to
  `SYS_IMAGE_REGISTER`. It does not reopen by a caller-supplied path after
  verification.
- The kernel Image cap is minted only after its ELF validation succeeds.
  The returned cap, not a name, ID, or path, is the Desktop spawn authority.
- Descriptive application-ID collision, guessed ID, retired/stale version,
  malformed APB1 metadata/tree, ineligible signer, and stale current policy
  all refuse before an Image cap is published.

### Failure and bounds

- Catalog rebuild is transactional: a candidate table is fully populated and
  checked for duplicate IDs before it replaces the live table. Invalid
  candidates are reported and skipped; storage/offline failures keep the
  service fail-closed and do not publish a partial table.
- Catalog capacity remains 64 until an end-to-end workload establishes a
  justified bound. The installed root is scanned with a finite object budget.
- Image bytes, Image slots, dynamic child count, executable segment pages,
  memory, and launch concurrency are independent bounded resources. Any
  later increase requires an accounting change and guest proof; a larger
  manifest or package limit alone does not imply a larger executable limit.
- A catalog response is data. It is never treated as a token that can be
  replayed after an install, retirement, or policy update.

## Security properties

- No user app receives the APB1 install-only endpoint or ImageRegistrar.
- No user process reads `/System/Applications` directly.
- No path-only, ID-only, or version-only launch is accepted.
- File associations may select an application ID but cannot mint or transfer
  a document File cap.
- Helpers, windows, and processes remain separately authorized objects.
- APKG v1 behavior and bytes remain unchanged; no POSIX/Linux semantics are
  introduced.

## Implementation evidence required

The guest proof must install and verify a real signed APB1 containing a valid
native ELF, rebuild the catalog from protected AFS2 state, resolve the exact
entry, mint a real Image capability, launch a real child through Startup ABI
v2, observe its Process/window state, and cleanly retire the process and
Image. Negative controls must cover an unregistered guessed ID, replacement
or retirement of a stale version, malformed package/tree, app-ID collision,
and executable-byte mismatch. These checks must exercise services and kernel
state rather than fixed output strings.
