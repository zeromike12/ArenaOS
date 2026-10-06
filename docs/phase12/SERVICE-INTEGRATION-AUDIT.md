# APB1 service integration audit (Phase 12)

This is a source-derived boundary audit, not an accepted install protocol or a
claim of guest integration. The code and accepted ADRs remain authoritative.

## Current service boundaries

### `packaged`

- `userspace/packaged/src/main.rs` is the APKG v1 receiver, lifecycle journal,
  and Image-registration service. It loads the bounded policy chain by scanning
  the AFS1 namespace, then performs a full receiver scan before readiness and
  rescans on requests.
- It starts with exactly five child grants in slots 0–4: AFS1 endpoint/W,
  package endpoint/R, diagnostic Notification/R, ImageRegistrar/W, and
  lifecycle Notification/R. Startup rejects live caps in slots 5–31.
- The request loop handles every landed cap with `take_diagnostic()` before
  operation dispatch. That helper only accepts the service's expected
  Notification marker, destroys the landed reference, and refuses other kinds.
  Therefore a source-file capability cannot reach a new APB1 handler through
  the current loop.
- The service does not own the AFS2 `Volume`, `InstallWorkspace`, APB1 bundle
  verifier, or installed registry. Its current exact five-grant spawn contract
  must not be widened as an implementation shortcut.

### `servicemgr`

- `userspace/servicemgr/src/package.rs` requests those five grants literally;
  the child spawn still uses the existing five-cap limit.
- The manager already has a trusted package-service client endpoint and
  performs readiness/Process-cap lifecycle checks. It has no source-file cap
  for a user's AFS2 object and no filesd install authority today.
- A post-start explicit cap handoff is mechanically distinct from widening
  `SYS_SPAWN`, but no handoff message, recipient slot, or authority has been
  accepted or implemented.

### `filesd` and AFS2

- `userspace/filesd/src/main.rs` owns the one live `Volume<Blk>` and is the
  natural transaction boundary for the APB1 host install core. It holds a
  read-only AFS1 service endpoint for the one-shot import; AFS2 starts at
  sector 16,384 and is 16,384 4 KiB blocks in the normal guest profile.
- The only installed record is the kernel-minted `/Users/user` root, badge
  record 1/generation 1, with `R_ALL`. Requests are relative to the exact
  record object; names do not grant authority and `..` is not a filesystem
  operation. `/System` has no ordinary caller record. The current boot path
  creates `/System/imported-afs1`, but not yet `/System/Applications` or a
  private APB1 staging root.
- The AFS2 request dispatcher has no APB1 install or registry operation.
  Neither a new generic filesystem right nor a path-based `/System` exception
  is acceptable. Any future install path must use a separate exact receiver
  badge/capability and dispatch only to a fixed protected target.

## Host integration seam added

`arena-platform-core` now offers
`install_with_policy_from_volume_file()`. It takes an internal AFS2 object ID,
checks that the object is a regular file, and performs bounded random-access
reads through the same `Volume` that stages and publishes the install. The
implementation is safe Rust: the reader is called with the single `&mut
Volume` in sequence, so it does not alias the transaction's volume borrow and
does not buffer an archive (APB1 remains capped at 32 MiB and 4 KiB chunks).
It preserves the existing claim/key/policy/digest binding, pre-mutation
re-verification, durable readback, and atomic rename. The existing independent
`BundleSource` API and APKG v1 bytes remain unchanged.

The public host-core function does not turn an object ID into authority. A
guest receiver must first prove possession of a live same-filesd file
capability, verify its badge against the held record, require `R_READ` and a
regular file, then pass that record's exact object ID. It must separately
possess the install-authority capability for the fixed protected roots.

## Unresolved design gate

The persistent APKG v1 policy chain is currently verified by `packaged` from
AFS1, while the AFS2 transaction lives in `filesd`. A source-file cap cannot
cross the current `packaged` diagnostic-marker gate, and `filesd` does not
currently reload the policy chain. The service contract must make the selected
policy receiver explicit, preserve the five bootstrap grants, prevent caller
metadata from becoming authority, fail closed on damaged/missing persistent
policy state, and keep `/System` unavailable through ordinary file grants.

No new operation number, install badge, extra bootstrap capability, AFS2
namespace mutation, or guest-service patch has been accepted in this audit.
The next architecture step must define that receiver-verified handoff and its
RED tests before implementation.

## Resource notes relevant to that decision

- Kernel capability slots are 128 per process under Phase-12 ADR-0086; startup
  identity remains 0–31 under ABI-0083. These are separate domains.
- `filesd` has 256 record slots, a 16-record per-lineage quota, 24 watch slots
  globally and four per lineage. Existing values are not install capacity.
- Kernel endpoint/notification tables are 16/31 with queue depth 16; the M12
  proof covers the 32-caller queue boundary, not new service endpoint demand.
- The 32-session Desktop checkpoint is a single-process/single-window-per-
  session proof. It does not qualify multi-window apps, APB1 installations, or
  the static memory footprint of the proposed filesd installer workspaces.
- The settled Phase-11 baseline receipt remains
  `115127/15/15/1/469/2/23` in its documented order. No new frame, timer,
  endpoint, notification, watch, or cap high-water was measured by this source
  audit.
