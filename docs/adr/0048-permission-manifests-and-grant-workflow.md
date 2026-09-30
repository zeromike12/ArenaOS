# ADR-0048 — Phase 8.2 permission requests, approval and revocable grants

*Status: Accepted for bounded Phase 8.2 implementation (2026-09-30).
The proposal was published for review at `8c041ae` and `add2d30` before
acceptance. This revision resolves the single-endpoint/spawn-grant
integration conflicts and records the pre-code static/target, authority,
namespace and ordering audits. Acceptance is a design decision, **not**
completion of Phase 8.2 or a claim of working permission grants.*

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
  The manager resolves the request against its actual boot-held inventory
  and attenuation stays enforced at SPAWN; the broker separately checks
  ALLOW and the live bearer at ACQUIRE/READ. No pid/name-based privilege.
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

1. **One endpoint, one server thread.** `SYS_IPC_RECV` blocks on one
   endpoint, `SYS_TRY_WAIT` watches Notifications only, and there is no
   userspace thread-create or endpoint-select call. Two independent
   admin/app endpoints would leave one unserved, so use **one dedicated
   permission-mediator endpoint**. The broker holds READ to serve; the
   manager holds READ|WRITE|COPY to replay the broker's serve cap and
   to grant the app WRITE|COPY when it spawns the app. The shell holds
   WRITE to the *same* endpoint for CLI calls **plus** a separate
   READ|COPY|DESTROY reference to a boot-issued inert approval
   Notification. An admin ALLOW/DENY/REVOKE request is authorized only
   when `permissiond` compares the *transferred* marker's kind, object
   and exact rights with its own READ-only boot reference, then drops
   every IPC-landed cap even on a refusal. The marker never reaches an
   app. A string, opcode, endpoint/WRITE alone or guessed number is
   insufficient to approve or revoke. One synchronous receive loop
   handles both paths; no new IPC primitive, thread or endpoint-select.
   This endpoint is not the raw fsd endpoint.
2. The manager resolves the versioned, bounded built-in **app request**
   against its live `SYS_CAP_DESCRIBE` inventory and installs *only*
   mediator Endpoint/WRITE|COPY at app SPAWN, even if policy is DENY;
   this cap enables ACQUIRE but is not file-read authority. The broker
   separately validates its own live fsd/rng/serve/marker inventory,
   persists the trusted-shell **approval** and enforces issuance.
   A CLI `perm show/request/allow/deny/revoke` displays the request,
   decision and **actually issued** permission separately, including
   OFFLINE/NO_SPACE/IO rather than a claimed success. A new request is
   never implicitly approved on boot or service restart; only an
   earlier *durably approved* decision may be rehydrated. The initial
   approval requires the marker-bearing CLI action. Requesting WRITE
   when the held policy permits only READ is refused, not clamped.
3. **For the one-app v1 scope, reacquisition authority is possession
   of Endpoint/WRITE on the mediator** (the app's attenuated client cap,
   derived from the manager's held READ|WRITE|COPY reference at spawn).
   The shell also holds WRITE to call the same endpoint but is already
   in the trusted approver domain; the v1 contract does not identify
   callers among holders of WRITE. The app can hold its endpoint while
   policy is DENY; this is *not* file-read authority. An ACQUIRE call
   arrives on the single mediator endpoint, checks the durable ALLOW
   decision and live backend, then draws and returns a **new 128-bit
   rngd-backed service-issued grant**. Deliberately copying endpoint/
   WRITE|COPY delegates reacquisition while ALLOW remains active. No
   pid, image name or caller-provided string is proof. A later multi-app
   design would need its own possession-based acquisition distinction
   without stranding a second receive endpoint; it is **not** obtained
   by trusting an app-id word on this shared endpoint. The app and its
   delegate have no raw fsd endpoint, file handle or FS WRITE.

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
4. Revocation invalidates the active grant at the enforcing mediator;
   **every copy of those bytes becomes unusable**, including copies
   held by independent survivors. Deleting the originally issued
   reference or killing the first app is NOT revocation. The single
   broker thread dispatches exactly one request at a time on the sole
   endpoint; **its `SYS_IPC_RECV` dispatch order is the serialization
   order**, not whichever sender started its syscall first. A READ
   dispatched before REVOKE may return its already authorized bytes;
   a READ dispatched after an acknowledged REVOKE must refuse. While
   writing/rescanning a deny, the broker cannot dispatch another READ.
   It retires the in-memory grant after durable validated deny and
   **before** replying success to REVOKE; queued READs then fail.
   An ambiguous disk result gets no successful revoke acknowledgement:
   it fails closed in RAM, but an older ALLOW may survive a crash under
   the stated AFS1 model. The **manager** holds the app's actual Process
   cap and exit badge; it handles app death/reap separately from broker
   restart. Stopping the app is not the mechanism that invalidates a
   copied grant held by another endpoint holder. Guest tests must
   establish this ordering, not infer it from a log line.
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
   single mediator endpoint object must survive broker restart*: the
   lifecycle owner holds the actual endpoint and replays READ|WRITE|COPY
   to the new broker, as with ADR-0028. If this is not guaranteed,
   callers get typed SERVICE_GONE until a trusted owner deliberately
   reissues an acquisition cap; no silent pid/name replacement. Old
   grant bytes are forgotten (not replayed); a holder of WRITE must
   call ACQUIRE again. Required real-guest proof: **old token refused;
   held mediator endpoint + persisted ALLOW → fresh token; held
   endpoint + persisted DENY → refused; request/image name without
   endpoint → refused.** Test deliberate endpoint delegation and
   refusal after DENY, including a copied old token. The shell's
   separately held approval marker is required only for admin changes,
   not for ACQUIRE; its endpoint access alone is not admin authority.

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

## Proposed policy namespace and failure boundary (design gate)

Do **not** reinterpret `configd`'s `cfg8-*` single key as a permission.
Reserve only `perm8-01` through `perm8-08` in the same trusted AFS1
filesystem. Each is a fixed 512-byte immutable record with a distinct
8-byte magic (`ARPRM8V1`), format version 1, sequence matching its
filename, 32-byte maximum payload and independent checksum; all
unallocated/reserved bytes must be zero. V1 accepts **only** a fixed
four-byte payload: `(version=1, scope=arena.txt READ, decision=DENY|ALLOW,
reserved=0)`. No app name, pid, endpoint id, raw bearer or fsd handle is
serialized. UNSET means DENY. Unknown version/scope/decision, duplicate,
gap, nonzero unused bytes and visible malformed newest record fail closed,
never fall back to older ALLOW. An empty next-generation file is pending,
not a decision. Use the 8.1 ordered CREATE/WRITE/CLOSE/rescan and exact
value comparison; first successful durable record seq1, eighth seq8,
ninth returns NO_SPACE with old bytes intact. Disk-full or ambiguous FS
errors return a typed failure, latch writes off until recovery and
never acknowledge a revoke from unverified state. The broker is the
**only service-side policy writer** and checks the transferred admin
marker on every write; the old trusted Power/raw-FS shell remains in
the storage TCB (not a cryptographic anti-rollback boundary).

`arena.txt` + eight `cfg8-*` + at most two staged test-intent files +
eight `perm8-*` use at most **19 of AFS1's 32 object slots** in the
known bounded fixture; metadata/data sectors, reserved names and
other trusted filesystem files still need disk-full/object-full guest
proof. An extra policy namespace is not an extra IPC endpoint, app
image grant or configd update marker. Normal reboot and the ADR-0046
crash model apply; arbitrary commit-sector corruption may hide a later
DENY and expose an older ALLOW, so no tamper-resistance claim.

## Resource and integration gates BEFORE architecture-dependent code

The current tree has `MAX_IMAGES=24` with IDs 0–23 used,
`MAX_ENDPOINTS=8` with the production full fixture at its bound,
`MAX_NOTIFS=14`, `MAX_INHERIT=5`, `MAX_PROCESSES=32` and `CAP_SLOTS=16`.
The last is an **early bring-up number** in ADR-0015, not a reason to
contort policy architecture. Compare the smallest immediate increase
with a durable fixed bound. Rust 1.97.0 compiled the actual `CapObj`,
`Cap`, `CapSpace` and `Process` field declarations extracted from
`kernel/kernel/src/{cap,proc}.rs` on both host x86_64 and the bare-metal
`x86_64-unknown-none` target, varying only the array length (reproduce
with `python3 tools/probe_capspace_layout.py`):

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
costs 12.5 KiB more than 16 or 10.75 KiB more than 18. Host and no_std
bare-metal target layouts agree on all 14 measured fields
(`python3 tools/probe_capspace_layout.py`, PASS). These are static
sizes, not a claim that a new boot image already has 32 slots; once
the code changes, measure actual boot frames and the live full-table
refusals before qualifying that implementation.

**Propose `CAP_SLOTS=32`: fixed per process, no dynamic expansion.**
Eighteen is just sixteen plus this milestone's two shell grants and
would invite another global ABI/table-size revision at the next
mature-userspace client. Thirty-two buys 14 additional slots over 18
for future *held* authority, at a bounded static maximum increase of
10.75 KiB across 32 process entries; it mints **no** new capability
objects or rights and does not raise `MAX_INHERIT`/`MAX_GRANTS`.
Manager-local `MAX_CAPS` is a separate *source-slot/handle scan* limit
and must reach 32 for the inventory below. Shell slots 16 and 17 may
hold the sole mediator endpoint and approval marker without moving any
historical grant in 0–15. A full 32-slot table still refuses
COPY/MOVE/IPC landing/grant atomically; test the true last slot 31,
out-of-range slot 32, attenuation, table-full rollback and cleanup on
reap. Retain all prior M3 capacity assertions (parameterized by
`CAP_SLOTS`) and add explicit new-bound tests rather than changing a
failure into a SKIP. This revisits only ADR-0015's numeric capacity,
not per-process ownership, rights or destruction semantics.

### Exact spawn inventory: four broker grants and one app grant

`permissiond` is a **manager-owned SYS_SPAWN child**, not an extra kernel
root exempt from `MAX_INHERIT`. The manager also spawns/owns the app:
this is not an extra `Image` grant to the broker, and avoids an otherwise
missing app-exit Notification/lifecycle path. The manager already has
boot grants 0–11 in the full fixture, including rngd Endpoint/WRITE|COPY
in slot 5, probe-worker Image20/READ in slot 9 and its own event/restart
Notifications in slots 1/7. Only when dependencies actually exist, the
root would add exactly these **five** manager-held sources:

| Manager source slot | Actual object and source rights | Purpose |
| ---: | --- | --- |
| 12 | broker Image/READ | spawn `permissiond` |
| 13 | app Image/READ | spawn the one app; never passed to broker/app |
| 14 | fsd Endpoint/WRITE|COPY | only broker gets attenuated WRITE for file/policy IO |
| 15 | mediator Endpoint/READ|WRITE|COPY | child serve side, app acquisition side, manager probe side |
| 16 | approval Notification/READ|COPY | child READ-only reference; shell separately gets the exact sendable marker |

With existing manager slot 5 for rngd, the **entire broker child
inheritance list** (no hidden optional grants) is:

| Broker child slot | Inherited cap, exact rights | Purpose |
| ---: | --- | --- |
| 0 | fsd Endpoint/WRITE | `arena.txt` and distinct transaction namespace |
| 1 | rngd Endpoint/WRITE | draw a 128-bit bearer, fail closed if absent |
| 2 | mediator Endpoint/READ | sole blocking receive loop; no call/mint side |
| 3 | approval Notification/READ | receiver-side marker reference; cannot notify |

The **app child inherits exactly one** cap at slot 0: mediator
Endpoint/WRITE|COPY (the sole acquisition/delegation authority); no
fsd/rngd/marker/image grant. Both children deliver their exit badges
to the manager's already-held Notification via `SYS_SPAWN` arguments;
the manager retains and reaps each actual Process/DESTROY handle.
`spawn::MAX_INHERIT=5` and `manifest::MAX_GRANTS=5` therefore **remain
unchanged**. Neither child needs an extra readiness/lifecycle grant.
The manager must own two independent lifecycle records and backoff
budgets and replay the same mediator object to every new broker.

For broker startup readiness, the manager must not itself block in an
unbounded IPC_CALL. It spawns a **bounded probe worker** from its
existing image20 with *only* mediator Endpoint/WRITE; the worker calls
an unmarked read-only PING after broker initialization, validates its
typed reply and exits. The worker's kernel exit badge goes to the
manager's existing notification; the manager arms its own timer and,
on timeout, stops the held worker and broker via their Process caps and
marks the broker OFFLINE. PING never approves, ACQUIREs or returns file
data. This is a new audited worker mode using the existing watchdog
pattern, **not** a new broker grant, cancellable IPC fiction, second
receive endpoint, thread or new primitive. The manager reports the app
only as SPAWNED (not READY from its pid); its functional proof is an
actual successful broker call, and its last-thread exit badge triggers
Process-cap reap. If app readiness later requires a new grant, revisit
the exact count before adding one.

With slots 0–16 occupied in the manager's full fixture, its broker
Process handle first lands at 17, an app/worker handle at 18/19. Raise
and audit **manager-local** `manifest::MAX_CAPS` and the caller-cap-only
`inventory::child_handle` scan to 32 alongside kernel `CAP_SLOTS`;
this does **not** raise `MAX_GRANTS`/`MAX_INHERIT`. Check temporary
handle overlap and `MAX_SPAWN_RECS=20` at the worst-case concurrent
peak; do not assume the current single-stack manager loop already
supervises two children.

### Static/on-target capacity envelope (design audit)

The original fully equipped 8.1 boot showed 10 occupied process slots
and 10 live spawn records at the Power snapshot (`ADR-0046`), against
32 processes and 20 spawn records. Adding the broker, one app and at
most one live readiness worker implies an upper-bound **13/32 processes,
13/20 spawn records** relative to that observed steady state; the
manager must reap the worker before reporting broker READY, so normal
steady state is +2. A broker/app crash must release its record before
respawn; if an older ephemeral fixture or extra worker overlaps,
measure the *actual* guest high-water instead of extrapolating 13.
The manager's existing 12 permanent caps plus five sources = 17, plus
stack/broker/app/worker Process handles <=21/32 in the intended
schedule. These are bounded plans, **not** guest-observed deltas after
implementation.

Source-extracted Rust 1.97 `x86_64-unknown-none` layout yields
`CallSlot=240 B`, `Endpoint=976 B`, `Notif=24 B`
(`python3 tools/probe_capspace_layout.py`, PASS). One mediator endpoint
moves the global table **8→9** (+976 B), one approval marker moves
Notifications **14→15** (+24 B); the fixed 32-entry process table adds
12,800 B over the current 16-slot table. Combined static table growth
is **13,800 B**, not a live-frame number. Two built-in image mappings
need `MAX_IMAGES=26` (IDs 24 and 25), but that registry is a bounded
`match`, not 26 per-process Image caps. The boot-root/manager grants
above are issued only when fsd and rngd dependencies are genuinely
present; absent rngd never produces a bearer or partial authority.
Manager readiness must actively probe the broker before spawning the
app; missing fsd/rngd makes that policy path OFFLINE, not guessed READY.

This completes the **pre-code static and bare-metal target layout
census**, exact cap inventory and namespace/order design. After any
implementation changes, still measure actual post-EBS-relative
free-frame/record/process high-water on present/absent fixtures,
endpoint/notification full-table refusals and reaping before claiming
any completed 8.2 checkpoint. A different live result is a mechanism
to investigate, not a tolerance or a reason to invent a sixth grant.

## ADR-acceptance gates (before architecture-dependent implementation)

1. Reproduce the **static/on-target bounds**: x86_64 Rust `CapSpace`/
   `Process` layout for 16 and proposed 32, the exact source/child
   tables above, manager max slots/handles, a bounded live-process/
   spawn-record envelope and present/absent device dependencies.
   Static estimates are not live leak evidence; implementation must
   still measure actual frame/record/process deltas.
2. Review the **specific `perm8-*` namespace**, versioned record and
   crash/NO_SPACE behavior above against AFS1's 32-object/sector bounds.
   Confirm it does not consume `cfg8-*` or pretend FNV prevents rollback.
3. Review the **one-endpoint serialization contract**: dispatch at the
   sole IPC_RECV is the linearization order; an acknowledged revoke
   requires durable validated DENY and token retirement before reply;
   no positive acknowledgement on ambiguous disk state. The manager's
   bounded PING worker, app exit badges and Process handles must not
   add an unlisted broker grant. No implementation until these audits
   pass. Acceptance means a defensible design, not a tested service.

## 8.2 implementation/completion gates (AFTER ADR acceptance)

1. Host-test a no_std manifest/request/decision codec: malformed or
   unknown versions, duplicate/unknown operations, overbroad rights,
   unavailable caps, wrong kind, stale decision and exact encode/decode.
   No serialized request contains a capability or can mint one.
2. Guest-test the *real* single-endpoint broker with separate shell,
   app and adversary: no approval, absent/wrong/forged marker and a
   plain endpoint fail at the receiving service; a genuine shell
   marker persists ALLOW and issues only the requested attenuated read.
   Audit actual child caps, manager lifecycle and the worker's deadline.
3. Prove a copied endpoint delegates ACQUIRE under ALLOW, and old/
   forged/independently copied 128-bit tokens are refused after revoke
   or broker restart. Exercise read-dispatched-before vs
   read-dispatched-after REVOKE and confirm the documented order with
   byte-exact real `arena.txt` data. Test present/absent rng/fs cases.
4. Prove allow, deny and revoke across same-disk reboot and actual
   broker restart; SIGKILL QEMU at each policy-record write boundary.
   Typed NO_SPACE/DEGRADED, previous exact bytes and visible corruption
   refusal must agree with disk audit; do not claim arbitrary
   commit-sector rollback protection.
5. Preserve all 31 historical suites. Each **completed** 8.2 checkpoint
   gets its own deployable image, fresh full suite, final-EFI-bound
   100/100 QEMU boots, matching receipt and extracted-archive boot.

The three **design** audits above are recorded: target-layout probe
PASS, exact four/one spawn grant inventory with a bounded worker, and
separate `perm8-*` namespace plus single-receiver serialization. This
ADR is therefore **Accepted** for implementation. Guest race/order,
restart, delegation and crash tests are **8.2 completion evidence**, not
a pre-implementation condition for accepting an ADR. Neither status
implies the other.

## First implemented slice: fixed cap-space foundation (8.2 incomplete)

The initial architecture-dependent code raises only the kernel's
`CAP_SLOTS` 16→32; it does not create `permissiond`, a client app,
policy records, UI approval or a bearer. The existing M3 capacity
fixture still fills *every* slot and refuses the next grant. It now
explicitly exercises last-slot COPY/MOVE attenuation, occupied/full
refusal without source mutation, slot 32 out-of-range refusal, destroy
and exact frame teardown. A new guest test observes `32 slots/space`
and Power-gated production restart counters `(254,10,10)`; historical
8.1 real updates and eight-generation accounting remain exact at
`(254,10,10)` on this image. Host and no_std target probes verify
`CapSpace=800 B`, `Process=848 B`, one Endpoint=976 B and one
Notification=24 B. The latter two are *design projections*, not newly
allocated objects in this slice.

This **partial checkpoint** passed all **33/33 suites**, a fresh
**100/100** ordinary-boot loop bound to final EFI SHA-256
`06597e5ba07b62968c22acd1c06b946771628501a6c18a904c564a543bb5b176`,
and checksum/extracted-archive QEMU boot verification. Deployable image:
`releases/checkpoints/phase82-capspace-foundation/`. There is still no
8.2 grant/revocation or persistence proof; later code requires a new
image, full suite, fresh receipt and the remaining completion gates.
