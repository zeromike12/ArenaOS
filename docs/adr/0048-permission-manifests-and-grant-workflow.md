# ADR-0048 — Phase 8.2 permission requests, approval and revocable grants

*Status: PROPOSED FOR REVIEW (2026-09-29), not accepted or implemented.
These choices must be pushed and reviewed before architecture-dependent
implementation. No Phase 8.2 runtime feature or checkpoint is claimed.*

## Why 8.0 and 8.1 are not a permission system

The 8.0 manager's built-in manifest (`userspace/servicemgr/src/manifest.rs`)
requests a subset of **caps it actually holds**. `SYS_SPAWN` checks COPY
and attenuation at the point of inheritance. It is production service
bootstrap, not an application's request or an operator's approval. The
8.1 `cfg8-01`…`cfg8-08` store is a bounded **single key**, with 32-byte
payloads and an opt-in proof updater; arbitrary bytes in its file are not
a grant. It does not authenticate storage against a process with raw FS
write authority. The existing Power-holding shell is itself a trusted
raw-FS writer, not an untrusted app or a login boundary.

A manifest naming an endpoint, a pid, a port or a cap slot is not
possession of authority. Conversely, destroying one kernel cap does not
revoke copies held by another process (`cap.rs`). A permission UI that
merely says “approved” and hands out a direct endpoint cannot honestly
promise live revocation. Authority remains by possession, not by pid,
identity string or name (ADR-0033, ADR-0037, ADR-0047).

**Three deliberately distinct things:** (a) the built-in application's
*request* describes what it would like to do and authorizes nothing;
(b) an explicit trusted-shell *approval* records the bounded policy
choice, but is not an app's operative capability; (c) only a fresh
service-issued *grant* enables an actual read. The enforcing service
checks the grant on every operation. A persisted approval never becomes
a silently inherited fsd endpoint, and an unapproved request never
becomes a grant merely because the app knows a file name.

## First bounded target and rejected shortcuts

Start with **one built-in, statically registered application image** and
one narrowly scoped, service-mediated permission: READ the existing
`arena.txt` AFS1 fixture through the broker. The broker uses its own
held fsd call cap, returns only byte-exact file data and never forwards
the raw fsd endpoint or a WRITE-capable file handle to the app. The
fixed filename selects a resource; it does not authorize the operation.
This is a real file read, not a stubbed permission success. Do not add
package signatures (8.4), filesystem image loading, arbitrary executable
names, login identities or graphics. The built-in manifest names a stable image
key and requests a permitted operation and maximum rights. It has no
kernel object number and is never executable approval. The approver is
the existing explicitly trusted serial shell, *not* an ordinary app:
whoever can type at that shell already has Power and raw FS rights. It
must hold a boot-issued approval marker and transfer a COPY to the
receiving permission service for an explicit `perm` CLI action. A
request string, endpoint/WRITE or guessed marker number alone is denied.
The shell is not a model of authenticated multi-user access.

Rejected:

- Parse a manifest into an unconditional `cap::grant`/kernel mint path.
  The service must resolve against its actual boot-held inventory and
  attenuation stays enforced at SPAWN. No pid/name-based privilege.
- Give an app the production fsd or netstackd endpoint directly and
  describe deletion of the broker's copy as revocation. Existing copies
  would remain operational. Nor is a UDP port a kernel capability.
- Treat `cfg8-*`'s unkeyed FNV checksum as an approval signature or
  consume the existing single key as a policy blob: that would both
  invent an integrity property and break the 8.1 guest-value fixtures.
- Add authority to the shell's transient child slots or displace the
  fixed lifecycle proof and diagnostic slots. The full fixture uses
  slots 0–6, 7 for a child, 8–9 for file windows, 10–14 for lifecycle
  probes and 15 for the stack diagnostic marker. The current 16-slot
  table has **no permanent shell grant capacity**. Phase 8.2 is the
  right place to revisit ADR-0015's early numerical limit, not distort
  the authority model around it.

## Proposed authority flow (subject to the audits)

1. A dedicated, boot-granted `permissiond` holds an image READ|COPY cap
   for the one app, only the backend caps it needs, a private permission
   endpoint/READ, and a READ-only reference to a new otherwise inert
   approval Notification. The shell holds endpoint/WRITE and a separate
   READ|COPY|DESTROY reference to that **same** marker. The server checks
   kind, object and *exact* transferred rights against its own reference
   and drops every IPC-landed ref, including wrong-kind/missing/forged
   cases. The boot root never grants this marker to the app. Shell text
   cannot authorize an update without this receiver-side check.
2. The broker validates a versioned, bounded built-in **request** against
   its live `SYS_CAP_DESCRIBE` inventory. A CLI `perm show/request/allow/
   deny/revoke` displays requests and the **actual** granted rights,
   including OFFLINE/NO_SPACE/IO rather than a claimed success. A new
   request is never implicitly approved on boot or service restart;
   only an earlier *durably approved* decision may be rehydrated. The
   initial approval requires the marker-bearing CLI action. Requesting
   WRITE when the held policy permits only READ is refused, not
   silently clamped.
3. The app receives **only** access to the permission/file mediator
   (Endpoint/WRITE) and, after approval, an unforgeable 64-bit
   rngd-backed **service-issued grant** for `arena.txt` READ. The
   mediator retains the fsd authority; neither the app nor a delegate
   gets the raw fsd endpoint, an fsd file handle or filesystem WRITE.
   The grant is checked at the *enforcing receiver* on **every** read.
   Possession grants access; deliberately passing the grant plus the
   mediator endpoint delegates access without caller identity. An
   endpoint or request string alone, a forged grant or an old grant is
   refused. If entropy is absent, issuance fails closed. No new kernel
   cap kind, kernel grant tracing, pid authorization or UDP-specific
   revocation rule is introduced.
4. Revocation invalidates the active grant in the mediator's state;
   **every copy of those bytes becomes unusable** on the next read,
   including copies held by independent survivors. Deleting the
   originally issued reference, killing the first app or forgetting a
   copied grant is NOT revocation. The mediator serializes a read and
   a revoke: a read completed before the revoke may return data; a
   read received afterward must refuse. The revoke acknowledgement
   follows validated durable deny/revoke (when persistence is enabled)
   AND in-memory invalidation; a queued read cannot slip through as
   approved afterward. Exercise that boundary in a guest test rather
   than infer it from a log line. If the broker also stops its own app,
   that is an additional Process-cap-gated lifecycle action, not the
   mechanism that revokes the service-issued grant.
5. **Policy persistence is not grant persistence.** A separate reserved
   policy record namespace, not `cfg8-*`, will store a versioned allow/
   deny decision under an authorized transactional writer using 8.1's
   ordered-write/atomic-sector model and visible-record validation.
   Failed/ambiguous writes and exhaustion are explicit: an approval or
   revoke is NOT acknowledged until its complete new decision is durable
   and independently rescanned. Until then the operation fails or is
   typed AMBIGUOUS; it cannot promise a persistent revoke. If an
   ambiguous revoke fails closed in RAM and QEMU is killed before the
   new deny record commits, the older approval can still be observed on
   reboot. That is not a successful persistent revocation. On normal
   reboot or a broker restart, rebuild the decision from validated
   visible records, revalidate actual backend caps and issue a **fresh**
   random grant only if allowed. Never restore old grant bytes from
   disk or turn a readable request/decision into a grant.

   The persistence claim is limited to normal reboots and the **AFS1
   ordered-write/atomic-512-byte-sector crash model** of ADR-0046:
   visible malformed records are refused, but arbitrary corruption of
   an AFS1 commit sector can hide a later deny record and select an
   older approval. Disk rollback, malicious writes by a trusted raw-FS
   holder and volatile-cache power loss are likewise outside this
   guarantee. Consequently 8.2 revocation is **not anti-rollback or
   tamper-resistant**; neither the 8.1 checksum nor this approval record
   is a signature. A separate lower-layer integrity/anti-rollback
   design and proof would be required before making such a claim.
   Distinct namespace/writer and SIGKILL tests are required before
   claiming any persistent decision at all.

## Resource and integration gates BEFORE architecture-dependent code

The current tree has `MAX_IMAGES=24` with IDs 0–23 used,
`MAX_ENDPOINTS=8` with the production full fixture at its bound,
`MAX_NOTIFS=14`, `MAX_INHERIT=5`, `MAX_PROCESSES=32` and `CAP_SLOTS=16`.
The latter was an **early bring-up number** in ADR-0015, not a reason
for a fragile policy architecture. **Propose `CAP_SLOTS=18`, globally
fixed, never dynamically expanded**. Reserve shell slots 16 and 17
for the broker endpoint and boot-issued approval marker; do not touch
existing slots 0–15. This modifies only ADR-0015's numeric capacity,
not its per-process ownership, rights, attenuation or destroy semantics.
The initial static bound of extra references is two `Cap` entries and
two IPC-landing flags per `Process`, at most 32 process entries, plus
alignment: verify `size_of::<CapSpace>()` and `size_of::<Process>()` at
build/test time before claiming a byte count. Any resulting heap/frame
increase must be measured and bounded, not described as a leak or silently
ignored. Do not expand the separate spawn inheritance bound. All slots remain
bounds-checked and a full 18-slot space still gives a typed refusal;
check COPY, MOVE, IPC landing, table-full rollback and reaping at the
new last slot and the out-of-range slot 18. Existing manager-local
`MAX_CAPS=16` describes its own boot inventory, *not* the global kernel
cap-space bound and must not be casually widened.

App image and broker require explicitly mapped image IDs and real
binaries. One broker endpoint and approval marker require accounted
increases from the currently full endpoint/notification tables, as
well as a bounded policy namespace, FS/rng dependency order, broker
inventory, IPC-landed cleanup and manager ownership. Compare exact
post-EBS-relative free-frame/spawn-record/process use before/after
issuance, revoke and restart, on both device-present and absent boots.
Run old M3 capacity/full-table proofs and the historical suite with
the new fixed bound; a regression is a bug, not a reason to skip a
fixture. If the resource audit cannot be made bounded, revise the
proposal before implementing it rather than reusing transient slots.

## Ordered proof gates; 8.2 remains incomplete until all close

1. Host-test a no_std manifest/request/decision codec: malformed or
   unknown versions, duplicate/unknown operations, overbroad rights,
   unavailable caps, wrong kind, stale decision, and exact encode/decode.
   Verify the model contains **no capability objects** in serialized
   requests. This is design substrate, not an authority proof.
2. Guest-test the real broker with separate shell, ordinary app and
   adversary: no approval, forged marker, wrong kind/object/rights and
   an ordinary endpoint must fail at the receiving service; a genuine
   shell-issued marker makes exactly the requested attenuated operation
   work. Audit each actual installed child cap against the decision.
3. Demonstrate possession delegation and *real* revocation, including a
   previously delegated copy and a call arriving during revoke. A
   revoked bearer must not work even if its holder survives; closing a
   source cap by itself does not count. Exercise optional rng/net/fs
   absence and typed failure.
4. Persist both allow and deny/revoke across same-disk reboots and broker
   restart, with SIGKILL at each policy-record write boundary. A typed
   `NO_SPACE`/`DEGRADED` preserves the old approved state or fails closed
   according to the explicit transaction boundary; no guessed outcome.
   Audit disk bytes as well as the guest-visible effective permission.
5. Preserve all 31 historical suites. Every completed 8.2 checkpoint
   gets its own deployable image, fresh full suite, final-EFI-bound
   100/100 QEMU boots, matching receipt, and extracted-archive boot.

This is intentionally **not yet an accepted implementation plan** for
persistent approvals: the resource census and multi-key transaction
boundary have to be verified, and `arena.txt`'s in-flight read-versus-
revoke ordering specified, before code relies on them.
