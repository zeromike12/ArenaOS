# APB1 service integration audit (Phase 12 closeout)

This closeout update supersedes the pre-integration architecture snapshot below
in the git history. It describes the implementation qualified by
`tools/test_phase12_apb1_guest.py`; [ADR-0091](../adr/0091-apb1-filesd-install-handoff.md)
and [FINAL-REPORT.md](FINAL-REPORT.md) are the current protocol and evidence
records.

## Current service boundaries

### `packaged`

- `userspace/packaged/src/main.rs` retains APKG v1 policy and package-record
  behavior. It starts with the same five explicit capabilities and performs
  its existing full readiness scan before accepting the APB1 handoff.
- The APB1 install-only `BadgedEndpoint` is received after readiness into
  reserved slot 5. The receiver checks the endpoint identity, exact kind, and
  exact `WRITE|COPY` rights before retaining it. It does not reinterpret
  APKG v1 policy records as APB1 bundles.
- A live IPC proof refuses a generic LIST request through the install-only
  endpoint. A same-kind endpoint cap with extra `DESTROY` rights is refused by
  the exact shape check and destroyed after the negative control.

### `servicemgr`

- `userspace/servicemgr/src/package.rs` preserves the historical low-32 cap
  count, separately counts slots 32–126, and separately reports the descriptor
  for slot 127. Compile-time assertions tie the table to 128 slots and slot 127
  to the final valid index.
- The manager checks that the packaged receiver is still live, checks that
  slot 127 is occupied, and describes the exact held endpoint before
  forwarding it. The numeric slot is a location, not authority.
- `userspace/servicemgr/src/readiness.rs` removes the APB1 wake bit from the
  strict readiness mask while preserving it as a sideband hint. The targeted
  coalescing unit test proves that `NET|APB1` does not satisfy a `NET|RNG`
  gate, an APB1-only wake does not reach the gate, and a later real RNG signal
  is required. Production subsequently re-describes the live capability.
- Package restart obtains a replacement receiver, completes the same strict
  readiness proof, then re-establishes the handoff. The guest proof verifies
  the restart path.

### `filesd` and AFS2

- `userspace/filesd/src/main.rs` owns the live AFS2 Volume, protected
  `/System/Applications` and private `/.apb1-staging` objects, and the APB1
  install dispatch. An ordinary `/Users/user` File capability is denied that
  private operation.
- The source bundle crosses the boundary as an exact live filesd File
  capability selected through the real Desktop affordance. The receiver
  rechecks the landed capability and badge, requires the source-file rights,
  verifies the signature and file records, stages bounded writes, verifies
  durable readback, and activates by atomic rename.
- The install badge grants only APB1 operations; it is not a normal filesystem
  root. The guest attempts and observes denial of generic LIST through that
  authority. The protected AFS2 result contains the signed record and all
  payload files; staging is empty after activation.
- Existing AFS1 content is preserved by the tested migration/install flow.
  APKG v1 host/guest regressions remain separate and pass.

## Guest evidence

`tools/test_phase12_apb1_guest.py` drives a signed APB1 fixture from the actual
Desktop file icon, validates the installed signed record and payload bytes by
reading the AFS2 image on the host, verifies ordinary Filesd and install-only
scope refusals, exercises wrong-kind and wrong-rights controls, restarts
`packaged`, verifies the exact slot-127 handoff again, checks that
`[32,127)` remains separately accounted, and shuts down cleanly. The run and
EFI hash are recorded in `FINAL-REPORT.md`.

A boot without an available AFS2 install endpoint keeps slot 127 empty. The
M12 scale boot records that empty descriptor before the late APB1 authority is
available, then separately observes the held `kind=12 rights=6` capability.
This preserves the historical low-32 metric while making reserved authority
visible to qualification.

## Resource notes

- Capability spaces contain exactly 128 slots. The manager's legacy count
  continues to inspect only `0..32`; the separate extended count covers
  `32..127`, and slot 127 has its own exact descriptor receipt.
- The kernel IPC endpoint/notification and filesd record/watch limits remain
  independently bounded. The 32-session Desktop capacity proof does not imply
  multi-window-per-application or general service scaling.
- The 32-session resource high-water and exact teardown are reported in the
  final qualification receipt; no limit is inferred from host model bounds.
