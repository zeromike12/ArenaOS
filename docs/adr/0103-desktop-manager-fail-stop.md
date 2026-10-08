# ADR-0103: Desktop manager failure is machine fail-stop

## Status

Accepted for Phase 13.

## Context

Desktop owns AppInstance records, each instance's ProcessGroup and exact child
Process caps, document-capability offers, and the private owner caps for helper
timer Notifications. The kernel currently has no independent userspace service
that can recover that complete state after Desktop exits. Restarting Desktop
alone would leave live applications without their manager and without a
trusted owner for group teardown.

## Options considered

- Restart Desktop and reconstruct live groups from descriptive IDs or PIDs.
  These do not recover exact capability ownership and could adopt unrelated
  processes.
- Let application processes continue after Desktop dies. They may retain
  stale event authority and have no owner able to close their windows or reap
  helper processes.
- Have the trusted boot root monitor Desktop's liveness and stop the machine
  if its manager process exits, matching the existing fail-closed policy for
  boot-root graphics services.

## Decision

The kernel boot root retains Desktop's boot-child identity in its
`GraphicsRuntime` record. If Desktop has no live user threads, the root reaps
its process record and enters the existing machine halt path. It does not
restart Desktop or adopt its application processes. The normal path remains
Desktop-owned: it stops and reaps every process in an AppInstance group before
destroying the group's private notification owner caps.

## Reasoning

The present kernel cannot reconstruct the Desktop's exact ProcessGroup and
window ownership after manager death. A machine halt prevents managerless
applications from running with stale or orphaned authority. This policy is
bounded, deterministic, and does not introduce PID-based recovery.

## Downsides accepted

A Desktop fault stops the whole machine instead of recovering the graphical
session. An independently supervised application manager with kernel-retained
group capabilities could permit a narrower recovery policy later.

## Future implications

Any restart policy must preserve exact Process caps, window ownership,
notification lifetimes, and protected file authority across manager failure.
Reconstructing those relationships from names, paths, app IDs, or PIDs is not
acceptable.
