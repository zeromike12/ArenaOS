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
   for the one app, only the backend caps it needs, a private *admin*
   endpoint/READ and a distinct *app-specific* endpoint/READ. It also
   holds a READ-only reference to a new otherwise inert approval
   Notification. The shell holds admin endpoint/WRITE and a separate
   READ|COPY|DESTROY reference to that **same** marker. The server checks
   kind, object and *exact* transferred rights against its own reference
   and drops every IPC-landed ref, including wrong-kind/missing/forged
   cases. The boot root never grants this marker to the app. Shell text
   cannot authorize an update without this receiver-side check.
   Neither endpoint is a raw fsd endpoint.
2. The broker validates a versioned, bounded built-in **request** against
   its live `SYS_CAP_DESCRIBE` inventory. A CLI `perm show/request/allow/
   deny/revoke` displays requests and the **actual** granted rights,
   including OFFLINE/NO_SPACE/IO rather than a claimed success. A new
   request is never implicitly approved on boot or service restart;
   only an earlier *durably approved* decision may be rehydrated. The
   initial approval requires the marker-bearing CLI action. Requesting
   WRITE when the held policy permits only READ is refused, not
   silently clamped.
3. **Reacquisition authority is possession of the app-specific mediator
   Endpoint/WRITE capability**, derived from the broker's boot-held
   endpoint and installed in the app's spawn grants with COPY for
   deliberate delegation. The app may hold this endpoint even while
   policy is DENY; it is *not* itself file-read authority. On ACQUIRE,
   `permissiond` knows the request arrived on its dedicated app endpoint
   (not the admin or another app's endpoint), checks the durable ALLOW
   decision and live backend, then draws and returns a **new 128-bit
   rngd-backed service-issued grant**. A delegated copy of the endpoint
   delegates *reacquisition* while ALLOW remains active; no pid, image
   name or caller-supplied string is considered proof. A future second
   app needs a distinct acquisition endpoint, not a caller-id field on
   a shared endpoint. The app has no raw fsd endpoint, file handle or
   filesystem WRITE, and neither does its delegate.

   The app presents the grant on each READ; the *enforcing receiver*
   checks it against current state. A different endpoint or request/
   image name cannot ACQUIRE, and endpoint possession without ALLOW or
   READ without a current grant is refused. Choose **128 rather than
   64 bits** because this token is a general security-critical grant
   rather than ADR-0033's bounded UDP port handle. Sixteen random bytes
   fit the 64-byte IPC message with an opcode and offset; perform a
   full-length comparison and never derive tokens from policy or object
   numbers. Under the rngd entropy assumption, a blind guess has at
   most 2^-128 success per independent try (approximately q/2^128 for
   q tries); this is probabilistic unguessability, not a mathematical
   proof about the entropy source. No rngd or failed draw means **no
   issuance**, never a predictable fallback. No new kernel cap kind,
   global cap tracing, pid authorization or UDP-specific rule is added.
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
   disk or turn a readable request/decision into a grant. The *same
   app-specific endpoint object must survive broker restart*: the
   lifecycle owner keeps its serve cap and reattaches the new broker
   instance, as with ADR-0028. If that cannot be guaranteed in the
   bounded supervisor model, broker restart cannot claim grant
   reacquisition: callers get typed SERVICE_GONE until a trusted owner
   explicitly installs a new acquisition endpoint in the app. No
   silent replacement based on pid/image name. On restart all old
   grant bytes are forgotten (not replayed); the app or a deliberate
   endpoint delegate must call ACQUIRE again on the still-held cap.
   Required real-guest proof: **old token → refused; possessed
   app-specific endpoint + persisted ALLOW → freshly drawn token;
   same endpoint + persisted DENY → refused; a request/image name
   without that endpoint → refused.** Test endpoint transfer to a
   distinct process while ALLOW and refusal after DENY. Merely seeing
   the app image spawned or a policy string printed is not evidence.

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
The last is an **early bring-up number** in ADR-0015, not a reason to
contort policy architecture. Compare the smallest immediate increase
with a durable fixed bound. On x86_64, Rust 1.97.0, compiling the actual
`CapObj`, `Cap`, `CapSpace` and `Process` field definitions extracted
from `kernel/kernel/src/{cap,proc}.rs` with the array length varied
(reproduce with `python3 tools/probe_capspace_layout.py`):

| Slots | `CapSpace` | `Process` and `Option<Process>` | 32-entry process table | Delta vs 16 |
| ---: | ---: | ---: | ---: | ---: |
| 16 (today) | 400 B | 448 B | 14,336 B | — |
| 18 (exact current need) | 456 B | 504 B | 16,128 B | +1,792 B |
| 32 (proposed) | 800 B | 848 B | 27,136 B | +12,800 B |

`CapObj` is 16 B and `Cap` is 24 B, aligned to 8 B. This is **static
maximum table size**, not a measured live-frame delta: padding is
included, but allocator/page granularity and kernel-image layout may
change frame counts. A 32-slot space adds 400 B over 16 per process
and 344 B over 18 in both `CapSpace` and `Process`; the 32-entry bound
costs 12.5 KiB more than 16 or 10.75 KiB more than 18. The source-extracted host measurement is a design
estimate; repeat `size_of::<CapSpace>()`, `size_of::<Process>()` and
actual boot frame accounting **on the final target build** before
acceptance of the implemented bound.

**Propose `CAP_SLOTS=32`: fixed per process, no dynamic expansion.**
Eighteen is just sixteen plus this milestone's two shell grants and
would invite another global ABI/table-size revision at the next
mature-userspace client. Thirty-two buys 14 additional slots over 18
for future *held* authority, at a bounded static maximum increase of
10.75 KiB across 32 process entries; it mints **no** new capability
objects or rights and does not raise `MAX_INHERIT` or the manager's
independent `MAX_CAPS=16` inventory. Shell slots 16 and 17 may carry
the broker admin endpoint and approval marker without moving any
historical grant in 0–15. A full 32-slot table still refuses
COPY/MOVE/IPC landing/grant atomically; test the true last slot 31,
out-of-range slot 32, attenuation, table-full rollback and cleanup on
reap. Retain all prior M3 capacity assertions (parameterized by
`CAP_SLOTS`) and add explicit new-bound tests rather than changing a
failure into a SKIP. This revisits only ADR-0015's numeric capacity,
not per-process ownership, rights or destruction semantics.

App image and broker require explicitly mapped image IDs and real
binaries. **Two distinct broker endpoints** (admin and app acquisition/
read) and the approval marker require accounted increases from the
currently full endpoint/notification tables. Audit the endpoint's
lifetime across an actual broker restart, bounded policy namespace,
FS/rng dependency order, broker inventory, IPC-landed cleanup and
lifecycle owner. Compare exact post-EBS-relative free-frame/spawn-
record/process consumption before/after issuance, revoke and restart,
on device-present and absent boots. If that audit cannot be bounded,
revise this proposal before implementation rather than reusing
transient slots or claiming restart semantics a new endpoint cannot
satisfy.

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

This remains **Proposed**, not Accepted: the on-target resource census,
separate policy-namespace crash design/proof and actual in-flight
read-versus-revoke ordering tests have not passed yet. Record those
results before accepting the ADR or shipping any architecture-dependent
8.2 implementation. Updating this proposed ADR after review does not
change the 8.1 guarantee or qualify an 8.2 runtime.
