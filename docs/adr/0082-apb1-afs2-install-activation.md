# ADR-0082 — APB1 AFS2 staged install and immutable activation

**Status: Accepted for Phase 12 (service integration and guest qualification remain open).**

**Date:** 2026-10-06. **Milestone:** Phase 12, APB1 installation.

**Related decisions:** ADR-0080, ADR-0081, ADR-0076, ADR-0077.

## Problem

ADR-0081 freezes a signed, bounded multi-file bundle but intentionally does not
specify persistence. AFS2 commits each operation transactionally, while an app
installation consists of a record plus multiple files and directories. The
system must never expose an incomplete version as installed, duplicate all
payload bytes unnecessarily, overwrite an immutable version, or turn a name
or a signature-valid untrusted key into execution authority.

The accepted file-table grammar also needs to exclude a structurally valid
file path that is an ancestor of another file path (for example, both `bin`
and `bin/editor`); such a set cannot be represented as a regular-file tree.

## Options considered

1. Write each file directly below the visible application root. Rejected:
   crashes expose incomplete versions to discovery.
2. Store the entire original APB1 archive beside extracted files. Rejected:
   it duplicates up to 32 MiB and needlessly doubles payload storage.
3. Populate a private staging directory, validate the durable readback, then
   atomically move the complete version directory into the protected install
   namespace. Persist only signed metadata/signature plus extracted files.
   Chosen: one AFS2 rename is the visibility boundary and the source can be
   reconstructed without retaining a second copy of the payload.

## Decision

### Canonical tree representability

The strictly bytewise-sorted APB1 file table must be prefix-free at component
boundaries: no listed file path may equal a proper directory prefix of another
listed path. Thus `bin` and `bin/editor` is refused as `PathConflict`, while
`bin` and `binary/editor` is not a prefix collision. The Rust receiver and
independent Python oracle enforce the rule; a signed hostile fixture exercises
it. This is a structural restriction on APB1 v1, not a new wire field.

### Staging, durable record, and activation

- The trusted installer receives exact authority to a protected AFS2 staging
  directory and protected application-install root under `/System`. Names,
  IDs, versions, paths and object numbers are data; they do not select or
  confer these capabilities. `/System` remains inaccessible without an
  explicit grant.
- One immutable version is stored as
  `/System/Applications/<application-id>/<version>/`. A private staging
  directory is created under `/System/.apb1-staging/` using the complete
  bundle digest as a collision-resistant storage label. Existing stage-label
  or final-version collisions refuse; a final version is never overwritten.
- The staged tree contains each regular file at its signed relative path and
  an `APB1.record` containing exactly `header || manifest || file-table ||
  signature`. It does not contain a duplicate packed payload. Reopening the
  record and tree reconstructs the original APB1 byte stream for the existing
  bounded verifier, which rechecks the signature, policy-selected key, exact
  file set and every per-file digest.
- The installer first verifies the incoming source, writes the record and
  files in bounded chunks, then verifies the durable staged readback. A
  concurrently changed or short source cannot pass this destination
  verification. Only after readback succeeds does one AFS2 `rename` move the
  version directory from the private staging parent into the application
  directory. The registry scans only the installed root and independently
  verifies every candidate record/tree before presenting an app definition.
  The host `rebuild_installed_registry` path additionally binds exact
  `<app-id>/<canonical-version>` namespace names and rejects unsigned extra
  files/directories; it swaps a candidate catalog only after a complete scan.
  Boot-service authority, persistence, and launcher integration remain open.
- AFS2's copy-on-write commit record is the activation linearization point.
  Before it, the version is absent from discovery; after it, the complete
  directory is visible. An interrupted write may leave an incomplete private
  staging tree, but never an installed partial version. A boot-time janitor
  must handle abandoned staging entries without treating them as installed;
  janitor service integration is a remaining gate.
- The host core's policy-aware entry point reuses the accepted APKG v1
  `Chain`/`Policy` semantics without changing APKG bytes: package ID selects a
  policy namespace, signer ID selects an allowed key, and the authenticated
  version/full bundle digest are checked against current minimum and revocation
  state. The lower-level explicit-key entry point is only a primitive; a real
  privileged service must load and validate the chain from receiver-verified
  persistent authority and call the policy-aware path. APB1 does not reuse the
  APKG v1 marker for this purpose. The existing Phase-8 public root is a test
  key, not a production trust root. Signature-valid metadata and an install
  receipt are not an executable cap; launch still requires a held verified
  Image capability and explicit spawn grants.

### Failure and bounds

Each AFS2 file/directory operation may commit separately while staging. A
space/I/O refusal can therefore leave private staging bytes; that is acceptable
only because the registry cannot discover the staging root and recovery never
launches from it. The installer must report the typed failure and the janitor
must eventually reclaim abandoned staging. Duplicate installed versions refuse
before creating a new stage. There is no active-version overwrite or implicit
rollback policy in this ADR. Existing APKG v1 files and transaction rules are
unchanged. APB1's ADR-0081 bounds remain 64 files, 16 MiB per file, 32 MiB
total and 4 KiB streaming chunks; no unrelated kernel/filesystem limit is
raised.

## Reasoning and downsides

AFS2 `rename` is already a single copy-on-write metadata transaction and
refuses an existing target, making it a suitable whole-tree publication
boundary. Reconstructing the bundle from signed metadata plus installed files
allows the registry to verify the exact same byte contract without archive
payload duplication. Names remain descriptive and every execution grant stays
with the trusted app manager.

The accepted cost is that failure can consume private staging space until a
janitor runs, and boot-time registry construction must re-hash installed files
within bounded I/O. These costs are visible and fail closed. If the AFS2 rename
or namespace permissions cannot preserve the visibility invariant in the real
filesd guest path, installation remains unavailable; do not fall back to
in-place visible writes.

## Qualification required

Host RED/GREEN evidence must cover canonical path conflicts, exact metadata
record reconstruction, multi-file path creation, source mutation during copy,
wrong signer, readback corruption, duplicate immutable version, and every AFS2
write/crash prefix across activation. After remount, a package is either absent
from the installed namespace or complete and independently re-verifiable; no
partial version may be launchable. A real guest must exercise the same flow on
the exact AFS2/filesd artifacts, retain an installed-source/hash receipt, and
prove `/System` denial without the install grant before this ADR's integration
gate closes.
