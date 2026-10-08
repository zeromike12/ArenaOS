# ADR-0101: Signed helper allowlists and AppInstance-owned child launch

Status: Accepted
Date: 2026-10-07
Milestone context: Phase 13 native application lifecycle

## Problem

An installed native application needs a bounded way to request helper
processes. Package paths and helper names are descriptive data, and the
existing APB1 manifest describes only the primary executable. A helper must
resolve through the same current receiver policy and complete signed bundle
verification as that executable. It must not inherit the caller's authority
or expose process control outside its owning AppInstance.

## Options considered

- Add helper fields to AMF1 or change APB1 v1's signed metadata layout. This
  would change an already qualified package contract and complicate old
  installers.
- Treat a path passed by an application as an executable selector. This would
  let a caller choose arbitrary installed payloads and makes paths look like
  authority.
- Keep the APB1 v1 container unchanged and place a bounded helper descriptor
  in a fixed signed resource. Resolve every descriptor against filesd's
  current receiver-verified payload table, then return only an exact Image
  capability to Desktop.

## Decision

The package may contain one `META-INF/arena.helpers` resource in the AHL1
format. AHL1 contains at most four canonical helper IDs, relative executable
paths of at most 47 bytes, an explicit private-timer grant bit, and an
optional AppInstance-signal bit. The signal bit requires the timer bit.
Unknown flags, duplicate IDs or paths, malformed paths, and missing/non-
executable signed payloads are rejected. The resource and payload bytes remain
covered by the APB1 v1 signature and full installed-tree verification.

Packaged resolves `(application ID, helper ID)` only after rebuilding the
verified registry and rechecking the current receiver policy, exact version,
signer, bundle digest, and payload catalog. Filesd serves a payload only when
the exact path is present in the file catalog made by that verification
token. It copies bytes to a lent writable SharedRegion; it never returns a
path-derived filesystem capability.

Desktop authenticates the request through the application's per-instance
badged service endpoint. It owns the helper's exact Process capability in
that AppInstance's ProcessGroup. For a signed timer grant, Desktop uses a
non-copyable boot-issued NotificationFactory at slot 44 to create one fresh
bounded owner Notification. The helper receives only READ|WRITE on that
private object at startup slot 1; Desktop retains the owner cap and retires
the object after helper reap. The optional signal grant gives the helper
WRITE-only access to its AppInstance notification at slot 2, so it can signal
its owner but cannot wait on or consume that shared event stream. These are
separate objects. A helper gets no factory, Desktop endpoint, document
capability, filesystem lineage, network capability, or copy of the parent's
capability table. An opaque generation-checked ProcessGroup handle supports
owner-scoped wait/reap and termination; it is not a PID or transferable
authority. When the primary process or AppInstance ends, Desktop stops and
reaps all remaining members, then retires each helper timer object.

## Reasoning

The signature authenticates helper declarations and executable bytes, while
receiver policy decides whether the signer is still eligible. File tokens,
AppInstance endpoint badges, Image capabilities, and exact Process
capabilities each remain in their existing authority roles. The fixed
descriptor is bounded and independent of the primary AMF1 manifest, so APB1
v1 and APKG v1 remain unchanged.

## Downsides accepted

The manager currently supports a small fixed number of helpers per
AppInstance, and helper readiness is application-defined. Desktop polls a
helper's exact Process status during a wait request rather than blocking its
global event loop. The existing WRITE right permits both `SYS_NOTIFY` and
`SYS_TIMER_ARM`; an AppInstance signal grant therefore permits an owner wake
and a timer aimed at that owner's notification, but never permits consuming
owner events. AHL1 is an application package convention that needs a
versioned successor if richer argument or capability grants become necessary.

## Future implications

Any new helper grant must be an explicit AHL1 policy bit and an exact
capability offered by the manager. Ambient inheritance, arbitrary path
resolution, and direct application SYS_SPAWN remain out of scope. If helpers
need documents or IPC channels, a new signed descriptor contract and
receiver-side authority review are required.
