# ADR-0054 — Phase 8.5 installed images, activation and atomic upgrade

Status: **Proposed — C's direction recorded; persistent format and focused ADR-0055 kernel ABI not accepted; no installer or disk-image execution authorized**
Date: 2026-09-30
Milestone context: Phase 8.5, following the qualified, staging-only Phase 8.4 ([ADR-0053](0053-signed-package-trust-and-staging.md)).

## Problem

Phase 8.4 authenticates immutable `APKG`/`APOL` bytes, checks current policy and
stages at most two package generations on AFS1. `ELIGIBLE` means **neither**
installed nor active. No disk bytes become a kernel Image cap. Phase 8.5's
roadmap promises installation/upgrade, an explicit trust chain and power-loss
recovery. We must decide *which exact bytes may execute*, who can turn a
verified file into spawn authority, what an installed/active decision means
across reboot and crashes, and when an old image may safely remain runnable.
A shell message, pathname, staged reply, or old eligibility check cannot be
the authority to register executable bytes.

This is a design **start**, not an acceptance of a new syscall, executable
format, persistent record, key, service or security guarantee. Keep Phase 8.4's
qualified checkpoint bootable while resolving the questions below. Record and
review material ABI/trust/persistence choices **before** dependent code.

## Existing constraints and measured work to do

* The signed v1 package is `manifest[128] || payload[1..4096] || signature[64]`;
  the target is `x86_64-unknown-none`, flags are zero, and the payload is
  **opaque** in 8.4. Neither the accepted signatures nor APOL's eight-slot
  policy format may silently change. **Host measurement now confirms a
  purpose-built useful 648-byte static ELF passes the unmodified production
  `elf::validate`, and its test-root-signed APKG v1 file is 840 bytes**;
  see [payload-fit evidence](0054-elf-v1-evidence.md). Preserve v1 for this
  narrow mechanism proof. It is not proof of guest execution or that a
  general linked application fits. A future larger-app need requires a
  separately reviewed, explicitly versioned signed format.
* The existing `elf::validate/load` accepts bounded static x86-64 `ET_EXEC`,
  1..=8 program headers, `PT_LOAD` in the lower half, executable entry,
  no `PT_INTERP`, and W^X. `spawn::image_bytes` is a **kernel-embedded
  match** through image 26; `CapObj::Image { img_id }` with READ powers
  `SYS_SPAWN`. There is no disk-backed image registration or kernel call to
  fsd. The `MAX_IMAGES = 24` declaration is stale relative to the embedded
  0..26 match; do not treat it as available dynamic capacity or casually
  increase it. `spawn::MAX_INHERIT = 5` and manager `MAX_GRANTS = 5` still
  bound child authority.
* fsd is a userspace server. Its v1 interface has CREATE/OPEN/READ/WRITE/
  CLOSE/LS/UNLINK, **not** rename, overwrite or a multi-file transaction.
  AFS1 provides ordered writes and atomic-sector commits under its documented
  crash model. The 32-object table, finite sectors and exact source/policy
  budgets cannot be waived to make an upgrade demonstration pass.
* ADR-0053's root is a **published test-only public key**; corresponding
  host fixture signing is not production custody. The guest verifier's
  narrow ADR-0004 dependency exception does not authorize moving crypto into
  ring 0, a new crate/backend or signing inside the guest. AFS1 does not
  authenticate a hostile platter, prevent complete-platter rollback, or
  promise atomicity under arbitrary commit-sector corruption.
* An Image cap, once copied, remains a usable reference when another copy is
  destroyed. Similarly, a running process does not disappear because its
  package is revoked. Any revocation or upgrade promise must specify how
  **all** outstanding image authority and live processes are handled; a
  changed file name or deleted issuer-side cap cannot implement it.

## Options reviewed; C selected the narrow direction, not the final mechanism

### Turning signed bytes into executable authority

1. **Kernel reads the AFS1 disk or speaks fsd's IPC from `SYS_SPAWN`.** It
   could revalidate file bytes on every spawn, but makes filesystem and
   package policy part of the kernel/boot path, creates a circular fsd
   dependency, and greatly expands the ring-0 trust surface. Not preferred.
2. **Trusted ring-3 verifier/installer hands an immutable, byte-exact
   snapshot to a narrowly gated image-registration mechanism.** The kernel
   validates its ELF subset and owns/pins the bytes through cap/loader
   lifetime. The trusted verifier receives a provisional transferable
   Image cap; **only after durable selection** does a manager-marker-gated
   reply transfer it to the manager for attenuation to READ|DESTROY. This
   fits cap-by-possession and keeps fsd/policy
   out of the kernel, but requires a **new reviewed kernel primitive/public
   ABI** plus a lifecycle/revocation design. A buffer pointer, filename or
   caller pid is not proof that the bytes were verified. The service that
   holds registration authority becomes part of the execution TCB.
3. **Trusted launcher copies verified bytes straight into a new process**
   via current memory/process caps, avoiding registration. This risks
   duplicating the ELF/W^X loader in userspace or handing it broad memory
   mapping power. It also changes spawn/teardown accounting. Only consider
   if an actual mechanism can preserve the strict validator and attenuated
   child grants without a bigger trust expansion than option 2.

**C's selected direction:** option 2, ring-3 verified snapshot plus a
capability-gated kernel copy/production-ELF validation. The substantial
new ABI, fresh dynamic object IDs, stale-cap revocation and fail-closed
lifecycle are specified **for review** in focused [Proposed ADR-0055](0055-capability-gated-dynamic-image-registry.md).
Neither option 1 nor option 3 is authorized. The ABI is not accepted merely
by choosing the direction. Do not call a mutable file or LENT frame “sealed”.

### Durable installed/active state

* **One mutable `current` file or overwritten stage:** small and tempting,
  but AFS1 has no atomic rename/overwrite and a lost WRITE reply can leave
  ambiguous bytes. File presence alone cannot certify installation.
* **Immutable generation records with a separately committed activation
  decision:** promising within the existing AFS1 prefix model; each record
  would need canonical exact bytes, checksums/binding to the *entire signed
  package digest*, namespace and sequence, plus explicit linkage to policy.
  Its schema, bounds, object accounting and authority to write it are
  **not yet decided**. A committed intent is not itself active; activation
  is acknowledged only after durable re-read and successful authorization.
* **Always launch the latest staged file:** no install boundary, no
  explicit activation, and can accidentally activate corrupted or revoked
  history. Reject as a substitute for an install/upgrade transaction.

**C's selected direction:** immutable bounded installed-generation records
bound to the **complete signed APKG file digest**, with separate immutable
activation decisions. Receiver-checked install/activate approval must be
additional to 8.4's STAGE marker; copies of the old marker cannot silently
gain new authority. An installed, active or reverified image is **not**
a running process. No automatic boot launch. One package namespace only;
no GC, general multi-package manager or dynamic linking. The concrete
schema and grant inventory below remain **Proposed**.

## Proposed state machine and fail-closed rules to evaluate

The following are **candidate invariants**, not a frozen on-disk/API spec:

```text
signed bytes verified -> stage-eligible -> installed (durable digest binding)
                 -> activated (explicit durable selection) -> running child
```

* Verify **again at the install receiver**, reading exact signed bytes
  from fsd; stage replies cannot confer install authority. Verify again
  before making an Image cap or after restart/rescan as required by the
  selected snapshot protocol. Kernel ELF validation is necessary but is
  **not** a substitute for a trusted signer/policy decision. No
  TOCTOU: the launched bytes must be the bytes whose full-file digest and
  signed payload were checked, not a later file read by name.
* Every boot/restart scans the complete relevant namespace before reporting
  READY. Ignore an uncommitted transaction only when the documented AFS1
  model proves it absent; reject malformed/partial **visible newest**
  records and unrecognized names without falling back to older ALLOW.
  A previous active version may be used after an interrupted upgrade
  **only** if the chosen record scheme proves it the last committed
  activation, its exact signed package remains present, and it is still
  eligible under the *current* policy. Otherwise remain INELIGIBLE/OFFLINE
  rather than inventing availability. Never describe this as defense
  against hostile rollback of the whole disk.
* Specify separately the behavior of already running children, pending
  calls, and copied Image caps when policy revokes the current signer or
  full package digest. Proposed conservative boundary: installer/manager
  alone holds non-copyable launch authority; manager uses its **held
  Process cap** to stop/reap children on deactivation before reporting
  completion. That alone does not revoke a copied Image cap: either prove
  no copy can escape or add explicit, separately approved invalidation.
  State exactly when a revoked process stops and what it may do meanwhile.
* Every bound is finite and preflighted: signed bytes, installed records,
  active selections, registered/pinned image bytes, live Process/Record/
  cap slots, AFS1 objects/sectors and inherited grants. Pre-CREATE
  NO_SPACE leaves prior committed state exact; after ambiguous CREATE/
  WRITE, DEGRADED means no success until a same-platter rescan/restart.
  Do not add GC or multi-namespace capacity merely by raising a table
  constant; choose and prove safe reclaim separately.

## Candidate bounded AFS1 transaction (wire **not** accepted yet)

Keep the **single** APKG/APOL v1 namespace and the 8.4 `p8-`/`s8-`
predecessor scan. Proposal: immutable `n8-<20-lowercase-hex>-01..02`
installed-generation files and `v8-<same-namespace>-01..04` activation
files, each exactly **512 bytes** with its own checksum. The namespace is
SHA-256 of the full canonical ID field as in 8.4; recheck the **full** ID
inside each record, not just the filename prefix. Zero unassigned bytes.
`FS_OP_LS` scans all names before any decision; unexpected suffix,
collision, gap, malformed or partially visible newest record refuses
READY, not a fallback to an earlier ALLOW. A preflight includes the 32
AFS1 objects/sectors already consumed by 8.1/8.2/8.4; it may return typed
NO_SPACE without changing history. Post-CREATE ambiguity latches DEGRADED.
No renaming, overwriting, GC or enlargement of 8.4's signed wire.

**Candidate `AINS` record:** offsets `0..4` magic `AINS`, `4..6` format 1,
`6..8` length 512, `8..16` contiguous generation 1..2,
`16..48` canonical full ID, `48..80` SHA-256 of **entire signed APKG file**,
`80..88` signed package version, `88..120` manifest payload digest,
`120` immutable stage ordinal 1..2, `121` observed policy generation
0..4 (audit fact, **not** trust authorization), `122..128` zero,
`128..160` SHA-256 of preceding complete `AINS` record (all zero at gen1),
`160..480` zero, `480..512` SHA-256 of exactly `0..480`.
An `INSTALL` request requires the **new 8.5-only receiver-verified
manager marker** from Proposed ADR-0055 (never the 8.4 STAGE marker), full
current-policy/package verification from the exact staged file and an
available object slot. An installed package is not yet a validated Image:
the existing **kernel** ELF validator is used at the activation boundary
through ADR-0055, never duplicated in ring 3. The receiver
commits the next immutable record, closes and rereads it along with all
predecessors before acknowledging INSTALLED. No Image cap, activation or
running child results from INSTALL. A record checksum is an AFS1 crash-
prefix/integrity check, **not** a hostile-disk authenticity proof; APKG
signature/current policy are rechecked at every authorization boundary.

**Candidate `AACT` record:** offsets `0..4` magic `AACT`, `4..6` format 1,
`6..8` length 512, `8..16` contiguous decision generation 1..4,
`16..48` full ID, `48..80` SHA-256 of the selected **complete `AINS`
record**, `80..112` full signed APKG digest from that record,
`112..120` package version, `120` action (`1=select`, `2=deactivate`),
`121..128` zero, `128..160` SHA-256 of the preceding complete `AACT`
record (zero at gen1), `160..480` zero, `480..512` SHA-256 of `0..480`.
For action 2 all selected-image fields `48..120` are **zero**. Activation
requires a **fresh call carrying the new 8.5 manager marker** (the same
new object proposed for INSTALL, **never** the STAGE marker), plus
independent SELECT operation validation, a complete installed record,
freshly verified exact APKG bytes, present valid chain and current-policy
eligibility. C approved this **single coarse Phase-8.5 lifecycle-admin marker** for
INSTALL, SELECT/PREPARE, SELECT/COMMIT, DEACTIVATE, privileged LAUNCH and
ABORT in the one-manager scope. It is not install-only delegation: splitting
operations for a future delegate requires separate review and grant accounting. **Before**
creating an `AACT select`, pass a copy of the already-verified payload to
ADR-0055's capability-gated kernel registration: only the kernel's
existing ELF validator can declare it loadable; keep that Image cap
private and provisional, and revoke/drop it if the disk transaction
fails. A new `select` may not lower the version of a previously committed
`select`, even if both files remain on disk; deactivation is not version
rollback. After commit+CLOSE, rescan and exact re-read establish ACTIVE
or DEACTIVATED, then acknowledge; only then may a manager receive launch
authority. The provisional registry object must not leak on verifier death
before the durable commit (ADR-0055 object-lifetime gate). No automatic
boot launch. The subsequent explicit LAUNCH must reverify current policy
and the exact signed file, freshly register a byte-exact Image through
Proposed ADR-0055 (two-slot overlap with the old image, then revoke the
old ID), and spawn only through its held Image cap with explicit bounded
grants. Reboot discards
Image authority and reconstructs the durable selection only after full
rescan; it may register again only on such a fresh verified explicit
LAUNCH. An earlier installed/active stage need not be the **latest staged
candidate**; reverify it against the current policy and monotonically
committed activation version without pretending 8.4's latest-stage QUERY
was an installer authorization.

**Candidate two-phase volatile/durable cutover, contingent on accepting
ADR-0055:** the manager serializes LAUNCH, SELECT and DEACTIVATE for this
namespace and freezes launches before the first SELECT call. It calls the
*existing packaged endpoint* with the distinct 8.5 marker; the verifier
checks the exact staged file, installed record, signed policy, current
eligibility, version and pending decision generation, then registers the
new payload via `SYS_IMAGE_REGISTER=33`. It holds the Image cap only in
its private local slot and replies PREPARED **without transferring a cap**.
A volatile, single-use prepare token binds the full signed APKG digest,
complete `AINS` record digest, next `AACT` generation and current signed
policy digest. At most **one** token/provisional Image exists at a time;
use the checked, never-repeated-within-that-verifier-lifetime encoded
`u64` token defined in the IPC section below (refuse on exhaustion). A byte-identical PREPARE with unchanged policy/file returns the **same** token
and does not register again. A *different* PREPARE while one is pending
returns BUSY until marker-approved ABORT; verifier teardown or *any*
policy/stage mutation revokes and drops the provisional cap and invalidates
the token. A manager stalled while
still alive may temporarily consume one slot; it must explicitly abort
or stay OFFLINE, not invoke unreviewed GC. On manager death with a LIVE
registration, the proposed ADR-0055 fail-stop takes precedence. Never
acknowledge ACTIVE or hand the manager Image authority merely for PREPARED.

With the manager no longer blocked in IPC, it uses its **held**
ImageRegistrar/WRITE to call `SYS_IMAGE_REVOKE=34` on the *old* dynamic ID,
invalidating **all** copies; then it uses each **held** non-copyable
Process/DESTROY cap through `SYS_PROC_FINISH` (mode 1 live stop, mode 0
exited reap), including races where the child exits, until all old dynamic
spawn records are retired. Any refused teardown leaves the namespace
OFFLINE, never silently delegates the old cap to another service. The
manager now calls SELECT-COMMIT on the same packaged endpoint with a
**fresh** landed 8.5 marker and the exact prepare token. The verifier
rescans/re-verifies current bytes and policy and refuses if any binding
changed, then writes `AACT`, CLOSEs, rescans and rereads the committed
record. Only after that proof does it reply with its provisional
Image/READ|COPY|DESTROY cap. The manager verifies reply status,
digest/version/ID and `SYS_CAP_DESCRIBE` kind/rights, copies a steady-state
Image/READ|DESTROY cap, and destroys the IPC-landed transitional cap; the
verifier destroys its original. A mismatched or unexpected returned cap
is destroyed even on error (the 8.3 reply-cap rule). The manager unfreezes
new launches **only after** commit and cap/teardown reconciliation.

For DEACTIVATE the manager freezes launches, revokes its old dynamic ID,
and stops/reaps children *before* the marker-approved `AACT` deactivate
write; no replacement cap exists. Before either disk commit, old durable
selection may remain while old volatile code has been deliberately
stopped: remain OFFLINE until a later **explicit** request fully
reverifies and (if appropriate) re-registers the selected signed bytes.
After an ambiguous CREATE/WRITE, stay DEGRADED until same-platter
restart/rescan, never optimistically fall back. A lost reply after a
committed select does **not** undo `AACT`: rescan decides durable state,
then a new explicit LAUNCH rechecks and, if necessary, registers again.
A manager death with a LIVE dynamic registration **or any unretired
dynamic spawn record** invokes ADR-0055's **proposed** kernel fail-stop,
not a fictional Process-cap reacquisition. New policy revocation blocks
launch immediately, invalidates the selected Image ID and requires
stopping/reaping affected children before acknowledging completion; the
persisted old selection is then INELIGIBLE, not a fallback. Neither
userspace sequencing nor an old QUERY status substitutes for the kernel
liveness/last-reference hooks.

### Candidate packaged IPC extension (exact proposed wire; **not** implemented)

Reuse the existing package Endpoint/WRITE and IPC v1.1 `CALL`:
`words=[op,arg,CAP_NONE-or-landed-slot]`, an **exactly 64-byte** in/out
message, and `REPLY=[status,value,CAP_NONE-or-landed-slot]`. `op=0..3`
retain frozen 8.4 PING/QUERY/POLICY/STAGE bytes/statuses. Every new op
`4..9` requires a fresh IPC-landed **Phase-8.5 manager lifecycle marker**
(anchored at packaged, exact Notification object and literal READ anchor /
READ|COPY|DESTROY sender). The receiver destroys **every** landed cap on
all paths, even on malformed, missing, failed or old-v1 requests; only
successful COMMIT/LAUNCH may return an Image cap. New request `msg[0..32]`
is the canonical padded ID; `msg[32..64]` has the operation-specific
32-byte digest below (no ignored padding). An unrelated/unknown op, wrong
marker, wrong digest/arg/size or unexpected reply cap fails closed. Reply
`msg` is **all zero on every refusal**, `value=0`, `reply_cap=CAP_NONE`;
IPC transport errors (`STATUS_SERVICE_GONE`, BUSY) are separate from
application statuses. A caller destroys any unexpected landed reply cap
**before** interpreting a status (ADR-0053/8.3 discipline).

| `words[0]` and name | `words[1]` / `msg[32..64]` request | Successful reply `status`, `value`, `msg[0..32]`, `msg[32..64]`, cap |
|---|---|---|
| **4 INSTALL** | Stage ordinal `1..2` / expected SHA-256 of complete signed staged APKG | `4=INSTALLED`, immutable `AINS` generation `1..2`, APKG full digest, complete `AINS` record SHA-256, **no cap**. |
| **5 SELECT_PREPARE** | Installed generation `1..2` / complete `AINS` record SHA-256 | `5=PREPARED`, nonzero token, APKG full digest, `AINS` record SHA-256, **no cap**; if identical selected newest decision already exists, `6=ACTIVE` with current generation, digest pair, **no cap** (no new decision). |
| **6 SELECT_COMMIT** | Token from PREPARE / same `AINS` record SHA-256 | First commit: `6=ACTIVE`, value `(img_id<<8) | AACT_generation`, full APKG digest, complete newest `AACT` SHA-256, **one** Image/READ|COPY|DESTROY cap. Exact durable replay: same status/digest pair, value is **only** `AACT_generation` (no ID), **no cap**; a new explicit LAUNCH is needed before spawn. |
| **7 DEACTIVATE** | Expected **next** `AACT` generation `1..4` / full previous `AACT` SHA-256 (zero if none, but deactivation of UNSET is refused) | `7=DEACTIVATED`, committed generation, full new `AACT` record SHA-256, 32 zero bytes, **no cap**. Exact durable replay returns same with no new generation. |
| **8 LAUNCH** | Current selected `AACT` generation / complete selected `AACT` SHA-256 | `8=LAUNCH_READY`, value `(img_id<<8) | selected_generation`, full APKG digest, exact selected `AACT` record SHA-256, **one** freshly registered Image/READ|COPY|DESTROY cap. The manager alone may `SYS_SPAWN` after checking all reply fields, the live Image cap, and its one-child limit. |
| **9 ABORT** | Outstanding token / `AINS` record SHA-256 | `0=OK`, value zero, 64 zero bytes, **no cap** after revoking/dropping the matching provisional Image; same-token repeat in this verifier lifetime is a no-op. |

New statuses are positive `4..8` as named in the table; the existing
8.4 `0..3` and `-1..-7` are unchanged. For the two replies **with** an
Image cap, `value=(u64(img_id)<<8)|u64(AACT_generation)`, with all other
bits zero, ID in `27..=u32::MAX` and generation `1..4`; compare that ID
with `SYS_CAP_DESCRIBE` of the landed LIVE kind-1 Image and require exact
READ|COPY|DESTROY before attenuating. For every no-cap reply, `value`
is exactly the unshifted generation/token stated in its table row;
never infer an Image ID from a digest or a pid. Additional negative application
statuses (encoded as two's-complement `u64`): `-8=CONFLICT` (equal version,
different complete signed digest); `-9=DOWNGRADE` (lower installed/selected
version); `-10=STALE` (token/generation/hash mismatch, restart-lost token
without exact durable replay); `-11=BUSY` (another PREPARE pending or
serialized lifecycle operation). Existing `-2=DENY` includes absent/bad
manager marker or signature/current-policy failure; `-3=NO_SPACE` is
**pre-CREATE** capacity refusal; `-4=DEGRADED` means possible mutation,
and `-5=CORRUPT`/`-7=OFFLINE` refuse malformed visible history/service
unavailability. On valid `QUERY`, old replies remain unchanged and never
return a cap. A success with `CAP_NONE` when the table promised a cap is
**not** launch authority, including if IPC silently dropped a reply cap
into a full 32-slot caller; the manager stays OFFLINE and reconciles.

A token is `u64 = (counter << 8) | expected_next_AACT_generation`, where
`counter` starts at 1 and increases only for a new PREPARE in that verifier
lifetime (`<= 2^56-1`, otherwise `NO_SPACE`); low byte is 1..4 and the
upper 56 bits must be nonzero. Token reuse after a verifier restart is
**not** trusted as authority: only a byte-matching committed `AACT` at
the token-encoded generation can satisfy the replay rule. At most
one outstanding token/image exists. A byte-identical PREPARE with unchanged
signed file, current policy, installed record and target generation
returns the **same** token and Image ID, with no new kernel mint. A
different PREPARE returns BUSY until ABORT. COMMIT requires the exact
in-memory token and the same reverified digests/policy; it consumes the
token after a fully reread durable decision, **not** on a mere WRITE
reply. If the verifier restarts, no in-memory token survives: a COMMIT
replay may return ACTIVE/no-cap **only if** the newest complete `AACT`
record has exactly the token's encoded next generation, selects exactly
the requested full `AINS` hash/ID, links the correct predecessor and
matches freshly verified APKG/current policy. Otherwise STALE; it never
writes again on an unknown token. After success, an identical COMMIT
within the process lifetime follows the same no-cap replay rule. ABORT
invalidates the token; after verifier restart its old token is STALE.
Policy/STAGE mutation invalidates a pending token and revokes its Image.
After a committed COMMIT an ABORT of that token is STALE; only an ABORTed
*most recently* ABORTed token repeated within the same verifier lifetime
is idempotent OK (keep one `last_aborted_token`, which cannot authorize
another operation; earlier aborted tokens are STALE). A
PREPARE reply of ACTIVE/no-cap means the same decision was already
selected: the manager does **not** revoke or stop its current child for
that no-op; it may request a separate explicit LAUNCH if it needs a cap.
No timeout invents success or consumes another generation.

INSTALL is **idempotent only for the newest installed generation**: with
an exact request (same stage ordinal, complete signed-file digest,
canonical ID, payload digest and version) still eligible under *current*
policy, reread and return that `AINS` without creating a file. A lower
version than newest installed returns DOWNGRADE; an equal version with
a **different complete signed-file digest** returns CONFLICT, before
CREATE; a higher version may consume the second slot after re-verification.
An older exact install is not a rollback loophole after a newer install.
A lost INSTALL reply is retried with the identical request **only after**
required DEGRADED recovery/restart and full same-platter rescan; compare
the durable record, not a previous in-memory answer. SELECT enforces
similar no-lower-version and equal-version/different-digest refusals
against the latest committed select; deactivation does not erase the
version floor. DEACTIVATE repeats only if the *same next generation* and
previous-record hash match its newest exact deactivation; a later decision
makes the old request STALE. DEACTIVATE validates the durable record chain
and held lifecycle marker but may **disable** a package whose signer or
payload is now revoked; it must never require current-policy eligibility
to stop code safely. SELECT and LAUNCH, by contrast, require eligibility. LAUNCH is a fresh authority check/registration,
**not** a disk decision: refuse while a PREPARE is pending, require no
other live dynamic child, reverify selected bytes/current policy and
create a new Image ID through ADR-0055. The manager revokes/destroys its
old Image cap after accepting the fresh one; no cap/ID is inferred from
an earlier ACTIVE reply. Lost LAUNCH reply cannot authorize spawn and
must be reconciled/retired before retry.

If an interrupted upgrade has **no visible new activation record**, the
last committed activation remains selected only while its exact signed
bytes still exist and satisfy current policy. A complete committed newer
record is authoritative even if its reply was lost (repeat idempotently
without consuming another generation). A **visible partial/corrupt newest
record** blocks readiness rather than rolling back silently. A newly
revoked selected package is INELIGIBLE; earlier `AACT`/`AINS` generations
are not fallback authority. Already running children and stale Image
caps are separate volatile lifecycle concerns addressed by ADR-0055.
The record wire and limits above are **proposal for review, not an
accepted persistent ABI**; test the candidate AFS1 byte/crash/capacity
model before freezing it.

### Proposed wire, capacity and crash decision table (not frozen)

All record offsets above are **half-open byte offsets**; multi-byte
integers are unsigned **little-endian**, not host-layout structs. Record
hashes are raw SHA-256 of the specified preceding bytes; chain links hash
all 512 bytes of the exact previous record. The full canonical 32-byte ID
in a record must match the package and the namespace-derived name; the
`AINS` stage ordinal must resolve to the exact immutable `s8-` file whose
**complete signed-file SHA-256** equals bytes `48..80`. `AACT select`
must resolve its `AINS` record by full-record hash, ID, full-file digest
and version; a `deactivate` has zero selected fields. The first link of
each chain is all zeros. Recheck every byte/zero/reserved field and
sequence before consuming any record. A matching digest is a binding
within the documented non-hostile AFS1 crash model, **not** an adversarial
integrity or rollback proof.

The package namespace's theoretical maximum is **four `p8-` policies,
two `s8-` staged files, two `n8-` installs, four `v8-` decisions and up to
two live source/intent files = 14 AFS1 objects**, *plus* all other system
objects. The already documented maximum 8.1/8.2 fixture consumes **19**
(`arena.txt`, eight cfg, two intents, eight permission), so the combined
formal maxima total **19+8 (8.4)+2+4 = 33, one beyond AFS1's 32 slots**.
The `AACT` v1 schema accepts decision numbers 1..4, but **four decisions
cannot be promised on that full platter**: 2 installs + only 3 decisions
fit at 32/32. The fourth must refuse typed pre-CREATE NO_SPACE with the
prior exact state intact. This is a real availability tradeoff under the
no-GC/32-slot scope, not a reason to raise fsd's table bound or pretend
all limits can be simultaneously realized. C must explicitly confirm
this conditional fourth-decision limit at final freeze; a guarantee of
all four on the full historical fixture would be a material capacity
redesign. `n8`/`v8` are separately capped 2/4; preflight the entire real
32-entry fsd object table, all reserved names and disk sectors (including
AFS1 metadata CoW and a maximum 4288-byte APKG), kernel's two Image slots,
manager/verifier cap slots, one provisional token and child Process/Record
slots **before** CREATE. Count the measured real boot/platter fixture,
not an empty test disk; a lower safe quota is preferable to claiming an
unproved 14-object allowance. With no GC or overwrite, a full generation
returns typed NO_SPACE before mutation; after a possibly durable CREATE,
WRITE or CLOSE failure return DEGRADED/unknown until same-platter rescan.
C approved the fixed Notification increase **17 -> 18** for the 8.5
lifecycle-admin marker; an actual full-fixture 18/18 and mutation-free
nineteenth refusal are still mandatory guest proofs. No new endpoint is in
the candidate inventory.

**Reproducible host-side full-platter measurement:** see the
[design-only review ledger](0054-final-freeze-review-evidence.md).
`python3 tools/test_phase85_design_capacity.py` extracts the **qualified
Phase 8.4 bundle's actual 8 MiB AFS1 scratch template** (16,384 sectors,
initial 11 used), populates a *synthetic* 19+8+2+3 = **32/32** object-table
fixture with maximal candidate file sizes and one extent per file, and
runs `afs1.audit` on the resulting 8 MiB image. Observed **99 sectors
allocated, 16,285 free**; a fourth decision's name does not exist and
its object preflight refuses **without changing a byte** despite ample
free sectors. This is an offline AFS1 *geometry/count* measurement, not
guest fsd CREATE/WRITE execution: the synthetic config/policy/AINS/AACT
contents are not signed or semantically acceptable to packaged. The
later full guest fixture must confirm real commit/CoW sector usage and
typed refusal; a 32/32 object cap, not free sectors, is the measured
constraint.

**Source-anchored cap high-water *projection*, not guest measurement:**
the manager currently has 20 literal boot caps; proposed registrar and
lifecycle marker make **22**. Reserving **four** other resident Process
handles (stack, broker, app if live, packaged) gives 26; at most one
dynamic child and its Image cap give **28/32** during PREPARE; after old
child/cap teardown, COMMIT's landed Image + attenuated copy give
**28/32**, LAUNCH with an old Image plus two transitional new references
peaks at **29/32**. A separately scheduled readiness worker yields
**29/32**, never concurrently with cutover; packaged's five inherited
caps + LENT buffer + landed marker + provisional Image give **8/32**.
The 8.4 guest evidence counted 12/12 baseline resident processes/records;
the **one** additional dynamic child needs separately proven process/record
headroom. The source-checked host schedule in the capacity script proves
only arithmetic under the stated **one-live-dynamic-child**, no-overlap
assumptions. C must confirm this narrow child-concurrency limit at the
final freeze; more simultaneous children require a new occupancy bound
and Process-cap teardown plan, not an extrapolation. Actual 8.5 guest
cap/Process/record/frame peaks, including
restart and a full caller capspace, must be instrumented and tested;
exceeding the model is a refusal/diagnostic, not permission to bump 32.

| Cut point / observable committed AFS1 prefix | Candidate decision after complete same-platter scan (policy eligibility to SELECT/LAUNCH, not to disable) |
|---|---|
| Preflight before INSTALL/PREPARE/COMMIT/DEACTIVATE; 32 objects or exhausted generation | Typed NO_SPACE before CREATE, old exact bytes unchanged. PREPARE's kernel registration may instead return BUSY before any disk mutation. No assumption of a public free-sector API. |
| INSTALL before CREATE / CREATE not committed | No new `AINS`; old installed/active state only if every exact signed file/current policy still verifies. Retry exact INSTALL safely. |
| INSTALL CREATE visible, before or during its 512-byte WRITE or before metadata commit | Newest empty/short/malformed `n8` makes namespace DEGRADED/OFFLINE; no older installed/active fallback until explicitly repaired under separately accepted policy. |
| INSTALL WRITE committed, CLOSE pending/fails, before rescan | If full exact `AINS` visible, scan validates chain and digest; retry exact INSTALL returns same generation without new file. If corrupt/partial, refuse. CLOSE return is not proof either way. |
| INSTALL reread done, acknowledgement or caller/server reply lost | Durable `AINS` stays installed, **not** active; exact eligible retry returns same record; lower/equal-different request refuses. No Image inferred. |
| SELECT before PREPARE or bad signed/current-policy/AINS check | No new registry object or decision; old selection only if still current-policy-eligible. |
| PREPARE copied Image minted but response lost; verifier killed/ABORT/policy change | No new `AACT`; verifier's last Image reference retires or explicit REVOKE stales all. Same-manager identical PREPARE while service lives returns same token; restart loses token, retry must freshly verify/register. |
| PREPARE answered; before/after old Image REVOKE, before/during old Process stop/reap | Old durable `AACT` may remain while volatile old code is stopped; no COMMIT/ACTIVE acknowledgement; manager stays OFFLINE until teardown proven and explicit reverify. A failed stop is not an acknowledged deactivate. |
| After old teardown, before `AACT` CREATE / CREATE absent | Old durable selection is still the only recorded decision, but old Image cap is stale and child gone. Explicit future LAUNCH must freshly verify and register; never auto-resume old code. |
| `AACT` CREATE committed but before/during WRITE/commit | Newest empty/short/malformed `v8` means DEGRADED/OFFLINE; do not silently run old or new. |
| `AACT` full WRITE committed; CLOSE or reread pending/fails | Rescan decides: valid newest select/deactivate becomes authoritative; partial newest refuses. Reply/close status alone cannot roll back a committed record. |
| COMMIT reread succeeds but reply not staged, server dies, caller dies, cap dropped into full caller, or reply lost | Durable newest `AACT` wins after scan; **no cap is inferred**. Exact next-generation replay returns ACTIVE/no-cap, then explicit LAUNCH re-verifies/registers. Provisional Image auto-retires on last reference (ADR-0055 oracle gate). |
| DEACTIVATE before/after old Image REVOKE and child teardown; CREATE absent | Old durable decision remains but old volatile authority/child may be gone; OFFLINE until explicit recheck. |
| DEACTIVATE CREATE/WRITE/CLOSE/rescan/reply cuts | Same empty/corrupt-newest refusal vs complete-authoritative decision as SELECT; exact-generation/hash retry does not consume another slot. No new cap exists. |
| LAUNCH after fresh verification/registration but before reply, in staged REPLIED queue, dropped on landing, or verifier death | No disk decision changes. Caller may launch **only** with a validated landed LIVE Image and exact success reply; otherwise drop/revoke provisional refs, rescan and explicitly retry. No Image created merely by ACTIVE. |
| Policy/STAGE mutation or package revocation at any boundary | Invalidate PREPARE token, revoke provisional/selected Image, stop/reap child before acknowledged lifecycle completion. Previously committed selection may remain but is INELIGIBLE, not fallback authority. |
| Manager last-thread exit/fault/kernel destroy while LIVE registration or unretired dynamic child exists | Proposed ADR-0055 fail-stop **before** cap/IPC teardown; not a recoverable userspace restart or a QEMU PASS. Without live dynamic state its registrar is dead, and no implicit boot launch occurs. |
| Whole-platter revert, malicious write, or torn commit sector | **Outside** documented AFS1 crash model; no anti-rollback or arbitrary-commit-corruption claim. |

The per-operation IPC wire, volatile prepare token, idempotence rules and
fsd acknowledgement/crash responses above are now **specified for final
review**, not implemented. Actual guest sector-cut replay, measured cap/
Process/record high-water and the full-fixture fourth-decision capacity
choice remain **acceptance/qualification gates**. Do not infer atomic
multi-file commit
from AFS1; the immutable per-record commit prefix and explicit rescan are
the only proposed linearization points. This candidate ordering binds
the **Proposed** ADR-0055 primitive; if C revises that ABI or its marker
inventory, re-review this transaction before acceptance.

## Resolved direction and acceptance questions

1. **Payload/keys/scope settled for a mechanism proof:** the host-built
   meaningful strict-subset ELF is 648 bytes and the independent root-
   signed v1 file is 840 bytes. Keep **APKG/APOL v1** and the 4096-byte
   payload limit; do not add a signed-wire version without new evidence.
   Keep the public test root, one namespace, no implicit boot launch, GC,
   general multi-package management, production ceremony, Secure Boot or
   dynamic linking. Guest execution of that exact ELF is **still unproven**.
2. **Image authority direction settled; exact ABI still Proposed:** the
   ring-3 verifier supplies exact checked bytes, the kernel copies them to
   immutable storage and uses its existing ELF validator. Only possession
   of a registrar cap permits minting; dynamic IDs are disjoint from 0..26,
   never reused, and liveness is rechecked on *every* spawn. Approve the
   exact calls, cap rights, copy/transfer lifecycle, failure accounting and
   supervisor-death fail-stop in focused [ADR-0055](0055-capability-gated-dynamic-image-registry.md)
   **before** writing dependent installer code.
3. **Persistent transaction still Proposed:** review/freeze the exact
   `AINS`/`AACT` 512-byte schema, two installs plus up to four decisions
   **only when AFS1 has space** (the measured full fixture fits three),
   one approved coarse 8.5 lifecycle-admin marker distinct from STAGE,
   exact IPC/reply/token behavior and crash-cut matrix above. Explicitly
   resolve the 33-object simultaneous-maxima conflict with C, without
   silent GC/table expansion. Prove real guest/platter AFS1 prefixes and distinguish
   last *committed and still-policy-eligible* active version from a
   visible malformed newest record. No silent fallback, no overwrite.
4. **Running-child lifetime still Proposed:** active != running; manager
   retains the non-copyable Process/DESTROY handle for explicit stop/reap.
   Review ADR-0055's narrow fail-stop machine halt if the manager dies
   while a dynamically launched child survives. No other service can
   reacquire the current Process cap. If a reviewer rejects the fail-stop
   hook, return for a separately justified ownership decision; do not
   claim managed execution on orphaned authority.

## Decision (pending)

C approved APKG v1 fit, the **17→18** Notification capacity direction,
**one** coarse 8.5-only manager marker, and ADR-0055's two-slot/fresh-ID/
revoke/page-budget/fail-stop directions. These design approvals are **not**
acceptance of either new public ABI or persistent record wire. ADR-0054 owns the install/activation transaction; ADR-0055
owns kernel Image objects and the manager-death lifecycle. Both remain
Proposed. No new syscall, install/activation record, launch command or
Image authority may be implemented until their exact mechanisms are
accepted. A future production-key ceremony or versioned package format
requires its own explicit decision.

## Proof obligations for a future accepted design

Before implementing, capture an option decision, threat model, state/byte
schema and cap inventory in this or a superseding ADR. Then test a
*real signed, runnable guest executable* through actual fsd bytes and the
production kernel ELF loader/spawn path; independently compare its bytes,
digest, resulting Process cap, strict child grants and observed behavior.
Wrong signer/root, altered payload after verification, wrong marker/cap,
noncanonical/oversized ELF, W+X, wrong arch, downgraded or revoked
installed image, copied authority, missing fsd/manager and torn newest
records must fail closed. Kill QEMU at each real write/commit/ack and
activation/restart boundary; audit the same platter **before** reboot
and distinguish old committed eligible code from partial newest history.
Prove typed table/disk exhaustion, process teardown and exact resource
baselines across repeated upgrades; add deliberate red controls for the
signature/marker/byte-binding or loader boundary. Preserve historical
suites, qualify a final-artifact-bound **100/100** QEMU run, and ship a
checksummed deployable image extracted and booted from its own bundle.
This proposed document is **not** an 8.5 checkpoint; the last qualified
checkpoint is Phase 8.4.

## Downsides and future implications

A new image-registration primitive or signed v2 format is material public
ABI/persistent-format work, not a bounded hardening tweak. The test root
and non-adversarial AFS1 crash model bound any accepted security claim;
production keys, Secure Boot, hostile rollback resistance, dynamic linking
or running arbitrary third-party software need separate decisions. Until
this ADR's open decisions are resolved and accepted, continue to use the
qualified staging-only Phase 8.4 image; no on-disk Image cap is permitted.
