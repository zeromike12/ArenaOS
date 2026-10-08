# ADR-0091 — APB1 install authority across packaged and filesd

**Status:** Accepted and guest-qualified for the bounded Phase-12 APB1 install
handoff. Installed-app boot registry and launch remain deferred.
**Date:** 2026-10-06.
**Related decisions:** ADR-0053, ADR-0074, ADR-0076, ADR-0077, ADR-0080,
ADR-0081, ADR-0082, ADR-0086, ADR-0088.

## Problem

The Phase-12 APB1/AFS2 host core is not a guest service. `packaged` is the
receiver for the persistent APKG v1 signer-policy chain, but its child spawn
has exactly five bootstrap grants and currently consumes every landed cap as
a Notification diagnostic marker. `filesd` owns the only live AFS2 `Volume`,
but exposes neither a protected install operation nor an APKG policy
receiver. Passing paths, package IDs, or a caller-selected public key cannot
bridge these services safely.

## Decision

### Authority and ownership

- `packaged` remains the sole runtime receiver for APKG v1 policy updates and
  APB1 signer selection. It continues to scan and verify the same persistent
  AFS1 policy namespace using the unchanged APKG v1 byte contract. Policy
  mutation and APB1 install requests are serialized by the package service's
  single request loop; a bundle install must not run on a detached policy
  worker.
- `filesd` remains the sole AFS2 transaction owner. It owns `/System/Applications`
  and a private `/System/.apb1-staging` root internally. Neither directory is
  represented by an ordinary caller record or exposed by `/Users/user` path
  traversal.
- The signed source is an exact filesd File capability, not a path or object
  number supplied as authority. Before using the host core, filesd obtains the
  source badge from the landed cap on its own serving endpoint, resolves the
  live generation-exact record, requires `R_READ`, and checks that it names a
  regular file. The host-core object ID is used only after those checks.
- The AFS2 host transaction re-verifies the APB1 envelope with the policy
  service's selected key, binds the receiver-verified package/signer/version
  claim and whole-bundle digest, stages bounded 4 KiB chunks, re-verifies the
  durable tree, and publishes it by the single AFS2 directory rename. A typed
  authorization record is only data; filesd must also authenticate the exact
  install-authority endpoint badge.

### Narrow install capability and handoff

- filesd reserves one internal record, badge index 2/generation 1, with only
  the private `R_INSTALL` dispatch right. That record names the fixed
  Applications root; the staging root is a separate filesd-owned object.
  `R_INSTALL` is not added to `R_ALL`, accepted OPEN rights, or an ordinary
  filesystem record. The badge can invoke only the new bounded APB1 verify /
  install operations; every existing generic filesystem operation is denied
  through it.
- The kernel issues one `BadgedEndpoint` for that exact filesd badge to the
  trusted service manager after filesd's endpoint exists. The manager retains
  it in dedicated cap slot 127, outside its fixed low-32 named inventory and
  Process-handle scan, and sends it to a ready `packaged` process over the
  existing manager-to-package endpoint as a one-time capability handoff.
  The notification is a wake hint only: if it coalesces with a strict
  startup-readiness badge, the manager preserves it as a sideband and replays
  it after package initialization; it never satisfies a readiness gate. The
  manager explicitly describes and reports this reserved slot; no manifest,
  child-handle, process, or cap-table bound is widened. `packaged` accepts
  only the expected cap kind/rights, stores it in the first reserved
  post-start slot, and re-establishes the handoff after each supervised
  restart.
- The package child still starts with exactly five inherited capabilities;
  `MAX_INHERIT` and the ABI-0083 startup identity range are unchanged. The
  install capability arrives only after the five-cap bootstrap inventory and
  package-service readiness scan have passed. Package's other callers do not
  receive this filesd capability.
- The trusted Desktop broker receives only a dedicated write-side package
  service endpoint for the APB1 install request. A submitted source File cap
  is the input object, never approval. An explicit Desktop open action on an
  `.apb1` file selects the installer affordance; Desktop resolves that selected
  item to a read-only filesd capability and sends the cap, not the name. The
  suffix is presentation only and cannot select a key, authorize installation,
  or imply launch. Existing APKG mutations retain their separate
  receiver-verified marker requirements; the APB1 request is accepted only on
  the privileged broker endpoint and its forwarded source capability is
  rechecked by filesd.

### Policy binding and failure behavior

1. The package receiver inspects the source through filesd to learn an
   unauthenticated bounded APB1 claim. The claim is used only to select the
   candidate namespace in the current APKG v1 chain.
2. It selects the key from that receiver-verified chain, asks filesd to perform
   a full signature/file-hash verification, then applies the existing minimum,
   allow/revoke, and full-digest checks. It binds the verified source claim and
   digest to the following install call.
3. filesd independently re-verifies the same source and authorization before
   any AFS2 mutation and again through durable staged readback. A changed,
   replaced, unreadable, malformed, oversized, wrongly signed, or non-file
   source refuses closed. An incomplete stage remains private and is removed
   by the bounded boot janitor; it never becomes an installed registry entry.
4. The `packaged` event loop does not process a policy update while this
   request is in progress. Only the policy receiver owns a write capability to
   the policy namespace; the caller cannot alter the policy snapshot or inject
   a key into the chain.
5. Numeric IDs, package/app names, versions, paths, UI selection, process IDs,
   and receipts are descriptive only. No App/Image capability is implied by
   a successful filesystem install.

### Compatibility and limits

- APKG v1 bytes, operations, and tests are preserved. APB1 v1 bytes are not
  changed. No kernel syscall, cap-table limit, child grant limit, startup
  identity range, AFS2 block bound, or general `/System` grant is added.
- The special install endpoint is not a normal read/write root. The only
  ordinary filesystem root remains `/Users/user`; attempts to name `/System`,
  climb with `..`, or use the install badge for generic OPEN/WRITE remain
  denied.
- The current root key is still the explicitly public Phase-8 test root. This
  ADR does not claim production key provisioning.

## Qualification evidence

`tools/test_phase12_apb1_guest.py` exercises the real guest path: a signed
multi-file APB1 selected through the Desktop icon is installed under protected
AFS2, read back and verified on the host, while source AFS1 bytes remain intact.
The run also covers generic LIST denial through install-only authority,
ordinary Filesd denial, wrong-kind/wrong-rights refusals, slot-127 accounting,
strict readiness, packaged restart/recovery, and clean shutdown. The exact
artifact/test receipt is in `docs/phase12/FINAL-REPORT.md`.

The production coalesced-wake path is paired with the `servicemgr` readiness
unit proof: a simultaneous APB1+readiness wake preserves the APB1 sideband but
cannot complete the independent readiness gate; the manager later re-describes
the held capability before forwarding it. The guest proves the subsequent
capability handoff and its scope.

Remaining evidence is production boot integration of the installed-app
registry/launcher and broader installed-app lifecycle policy; those are
Phase-13 work. The bounded protected install decision itself is guest-qualified.

Host codec/policy tests separately cover canonical-format and refusal cases;
the guest proves the real protected service boundary and selected APB1 install.
The historical APKG v1 package-record suite remains separate and unchanged.

The listed tests qualify the install handoff and format path. They do not claim
the installed app can be launched from a production registry or launcher.
