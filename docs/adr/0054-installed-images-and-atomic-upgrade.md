# ADR-0054 — Phase 8.5 installed images, activation and atomic upgrade

Status: **Proposed — architectural questions open; no installer or disk-image execution authorized**
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
  policy format may silently change. A typical linked ELF may exceed 4096
  bytes: first prove that a *real, runnable, strict-subset ELF* and meaningful
  application fit this exact bound, or propose a separately reviewed versioned
  package format. A synthetic signature over data is not an execution proof.
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

## Options under consideration (none yet selected)

### Turning signed bytes into executable authority

1. **Kernel reads the AFS1 disk or speaks fsd's IPC from `SYS_SPAWN`.** It
   could revalidate file bytes on every spawn, but makes filesystem and
   package policy part of the kernel/boot path, creates a circular fsd
   dependency, and greatly expands the ring-0 trust surface. Not preferred.
2. **Trusted ring-3 verifier/installer hands an immutable, byte-exact
   snapshot to a narrowly gated image-registration mechanism.** The kernel
   validates its ELF subset, owns/pins the bytes for as long as any Image
   reference or child load needs them, and issues a READ Image cap only to
   an authorized manager. This fits cap-by-possession and keeps fsd/policy
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

**Working preference for review, not a decision:** option 2. Specify the
snapshot transport, kernel object lifetime, exact cap rights, limits, who
may register/spawn, and how a revoked image reference becomes unusable
*before* accepting it. Do not label a normal `Untyped`/LENT frame or a
mutable file handle “sealed” without proving immutability and ownership.

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

**Working preference for review:** immutable bounded records and an explicit
activation step. Install/activate should require an additional
receiver-checked admin approval, separate from 8.4's STAGE marker; do not
silently reinterpret the old marker's copies as new authority. Decide if
installer, verifier and manager are one or several services only after an
exact cap inventory and restart/absence proof. No implicit startup of an
installed app merely because AFS1 has a matching filename.

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

## Decisions required before accepting this ADR

1. **Executable payload and versioning:** can a real runnable `ET_EXEC`
   under 4096 bytes satisfy a meaningful 8.5 fixture? If not, what
   versioned signed format increases payload size, and how do v1 readers
   refuse or coexist with it? No use of APKG v1's zero-reserved bytes.
2. **Image authority and kernel ABI:** choose a source-of-bytes,
   registration/spawn interface, ownership/lifetime and cap invalidation
   rule. Specify an exact authority transfer, not pid checks or a
   string-named executable. Any new syscall, Image object kind/registry
   representation or kernel trust boundary needs explicit approval.
3. **Persistent schema and upgrade policy:** choose finite record names,
   exact wire, order/atomicity, commit acknowledgement, loss-of-reply
   handling, recovery/rollback semantics, and whether previously active
   code may run while a new policy is pending. Is activation immediate
   or a separate explicit admin action? State how install and active
   differ from staged and signature-valid.
4. **Revocation/lifecycle:** decide whether already-running code is
   force-stopped or drains; how all copied Image references are
   invalidated (or structurally prevented); what survives manager,
   verifier and fsd death and same-disk reboot. No claim that deletion
   of a single cap revokes its copies.
5. **Keys and scope:** keep the public test root for an 8.5 *mechanism*
   proof, or make a separate production custody/rotation/release-ceremony
   ADR before any production-security claim. Decide explicitly whether
   8.5 includes multiple package IDs, GC, unattended boot activation or
   dynamic linking. None is inferred from the word “installer”.

## Decision (pending)

No Image-registration ABI, signed format revision, installation record,
activation transaction or revocation policy is approved by this draft.
The working preferences above identify a direction to investigate, not an
implementation mandate. Resolve the five questions before accepting a
concrete mechanism; split out separately reviewed ADRs for any independent
kernel primitive, v2 wire format or production-key ceremony as needed.

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
