# ADR-0090 — Badge-authenticated Desktop application sessions

**Status:** Accepted and guest-qualified for built-in ABI-v2 sessions through
32-session capacity and affected M10/M11 regressions; multi-window-per-app and
installed-app launch remain deferred.
**Date:** 2026-10-06.
**Related decisions:** ADR-0074, ADR-0083, ADR-0086, ADR-0087, ADR-0088,
ADR-0089.

## Problem

The legacy built-in client sends a SharedRegion reference with every request;
Desktop uses that backing's descriptive object ID to find a session. That
remains necessary for the unchanged legacy/dynamic path, but the new native
startup path must not treat caller words, application IDs, PIDs, or display
metadata as a session selector. ABI-v2 also needs a separate service-call
authority from the writable surface used for authenticated publication.

## Decision

### Explicit child capability profile

For trusted built-in kinds (`kind < 6`), Desktop prepares the bounded
ABI-v2 startup record and inherits only the following explicit child slots:

| Child slot | Object | Exact rights | Purpose |
|---:|---|---|---|
| 0 | One-page startup SharedRegion | READ\|DESTROY | Descriptive startup transport; consumed before application entry |
| 1 | Per-session BadgedEndpoint | WRITE\|COPY\|DESTROY | Bearer authority to call this Desktop session |
| 2 | Session SharedRegion | READ\|WRITE\|COPY | Client surface and private I/O backing |
| 3 | Private Notification | Per-kind attenuated rights | Client pacing and queued-event wake |
| 4 | Optional explicit tail grant | READ-only MemoryPool for Monitor or attenuated filesd object | Existing Diagnostics/file use cases only |

The grant count is four without a tail and five with one, matching the
existing kernel spawn bound. There is no function SharedRegion capability in
this profile. The manager retains only its intended broker-side resources and
the exact held Process wrapper; the child receives no ambient capabilities.
The v2 child runtime consumes slot 0 and verifies the complete remaining
capability table before invoking application code.

### Per-session badge creation and receive

- Desktop mints one BadgedEndpoint from its trusted server-side endpoint
  authority for each built-in launch. A monotonically increasing nonzero
  badge is never reused during the endpoint lifetime; exhaustion refuses
  rather than wrapping. The client receives only the minted, attenuated
  endpoint cap. It cannot mint a sibling badge from that client-side cap.
- Desktop stores the badge and function-right policy in the broker-owned
  `Session`, alongside the held ProcessGroup handle. The badge number and
  `instance_generation` in the startup page are metadata; possession of the
  actual BadgedEndpoint is the authority.
- The broker receives with `SYS_IPC_RECV_BADGED`. The kernel supplies the
  invoked endpoint badge as receive metadata in word 3; request words 0 and 1
  remain caller-controlled and never select a session.
- A nonzero badge is accepted only when it matches exactly one live broker
  session whose held Process capability still reports live. Bootstrap data and
  trusted scope/function rights are derived from that matched record. If a
  surface cap is attached, its kind, object identity and exact expected rights
  must match that session. An Offer is accepted only on the Files session and
  only for the broker's existing `/Users/user` filesd object with the required
  attenuated rights.
- Unknown, stale, duplicate, or dead-owner nonzero badges fail closed. They
  are never retried through legacy object-ID dispatch. A wrong kind, rights
  mask, surface identity, or Offer object is rejected without changing session
  or window state; the landed request cap is dropped on reply.
- Plain endpoint calls (`badge == 0`) retain the existing legacy dispatcher,
  including ABI-v1 dynamic launches. This does not change APKG v1, Phase-11
  syscall behavior, or any unrelated kernel limit. Caller-supplied app IDs,
  paths, PIDs, windows, generations, and IPC words remain non-authoritative.

### Instance-slot interpretation

ADR-0083's descriptive startup-slot range is 0..31, matching the 32-entry
userspace lifecycle model and Desktop's 32 live session rows. The v2 launcher
writes the exact selected row, never `row % 16`; its nonzero generation changes
when a row is reused. The startup slot is not a window handle or kernel process
slot, and neither startup field participates in IPC authentication. The window
table remains separately bounded. Production currently binds one built-in
process/session to one ordinary window; multi-window-per-application behavior
is deferred to Phase 13.

## Security and compatibility properties

- A caller can send arbitrary values in words 0 and 1 without changing which
  session receives its request. The app-side audit sends a previous session's
  generation value in those words and requires Bootstrap to return the
  current endpoint's own kind.
- An application attempts `SYS_ENDPOINT_MINT` using its client-side service
  cap and requires the kernel to refuse. It also sends a wrong attached object
  with its valid endpoint badge and requires a refusal. These are runtime RED
  checks, not a claim that a caller-supplied badge can be forged in kernel
  receive metadata.
- Endpoint capabilities are transferable only through existing explicit
  capability-copy/inheritance rules. If a child is explicitly given or copies
  the BadgedEndpoint, authority follows that bearer capability; numeric badge
  metadata alone cannot recreate it.
- The dynamic/v1 compatibility path is intentionally distinct and remains
  selected only by the existing trusted launch branch. A nonzero badge cannot
  fall back to it.

## Evidence and remaining work

The dependency-light startup codec, `arena-runtime`, Desktop, and built-in
application are type-checked and linked for `x86_64-unknown-none`; startup and
runtime host suites pass. The M12 guest checks 32 live sessions, exact startup
slot/badge audits, 33rd-launch refusal, slot reuse, and teardown. M10/M11
guest regressions cover dynamic launch, client/service death, Files authority,
and window behavior. Remaining work is multi-window-per-application and
installed-app launcher integration.

## Consequences

Built-in service-call authority is now carried by a kernel-authenticated
BadgedEndpoint, while surface publication remains a separate explicitly
inherited SharedRegion. The protocol does not grant file authority by name,
change the legacy dispatcher, add Linux/POSIX behavior to the kernel, or
replace the future installed-app registry and lifecycle manager. The current
Desktop still maps one built-in process/session to one ordinary window. True
multiple-windows-per-process, headless helper lifecycle, and signed installed-
app launch remain open for Phase 13.
