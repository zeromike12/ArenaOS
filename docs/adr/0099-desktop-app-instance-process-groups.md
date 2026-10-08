# ADR-0099: Desktop AppInstances own separate native ProcessGroups

*Status: Accepted (2026-10-07). Production AppInstance and ProcessGroup
ownership, signed helper cleanup, and process exit status passed the Phase-13
guest and T4 preservation suite.*

## Context

Phase 12 introduced a reusable fixed-capacity `ProcessGroup` that owns exact
manager-held Process capabilities. Phase 13 added installed windowed and
headless application launch, but Desktop still placed every child in one
global `ProcessGroup`. The session row held only the primary process handle.
This made a future helper either share a global owner or require a second
unrelated process table, and app teardown could not define one group boundary.

Desktop also has separate window records: a primary window lives in the
session record while additional windows carry an owning session index. These
records describe routing and presentation; the process cap remains the
authority for liveness and teardown.

## Decision

Each occupied Desktop app-instance/session slot owns one fixed-capacity
`ProcessGroup<ChildProcess, 4>`. The first successful spawn creates the group
and stores it in the same owner slot. Later members will be admitted only into
that instance's group after an independent signed helper-policy check.

The session stores a generation-safe handle to its primary child for
authentication and primary-liveness policy. Group operations resolve the
handle to a `ChildProcess`, whose operations use its exact held Process-cap
slot. Session indexes, application IDs, PIDs, and window IDs never authorize
spawn, liveness, or teardown.

Retiring an app instance calls `ProcessGroup::stop_all` before releasing the
group and session row. This reaps exited members and stops/reaps live members
through each exact Process capability. The instance cannot disappear while a
group member remains owned by the Desktop.

## Bounds

- 32 Desktop app-instance/session slots remain the current UI-side bound.
- Each instance group holds at most four Process capabilities, matching the
  accepted `arena-platform` lifecycle model.
- No process, thread, endpoint, notification, capability-slot, or window
  kernel bound changes in this decision.
- The kernel's 64-process table remains the aggregate process ceiling; the
  per-instance four-member bound is not a promise that all 32 groups can be
  full simultaneously.

## Consequences

- Launch and process teardown ownership now follows the app-instance row.
- A helper crash can later be reaped from its owner's group without confusing
  it with the primary process or another app instance.
- Primary exit can be treated as an instance policy event while remaining
  helpers are still explicitly owned and can be stopped during cleanup.
- This change does not yet implement helper authorization, child exit status,
  background persistence, a separate `AppInstanceTable` in Desktop, or
  manager-death recovery. Those remain Phase-13 proof obligations.

## Evidence

- `python3 tools/test_phase13_registry_guest.py` passed with two signed
  instances, three windows per instance, read-only document handoff, a
  headless installed instance, and exact identity-resource teardown.
- `python3 tools/test_m12_scale.py` passed 32 live sessions, refused the 33rd
  without mutation, closed/reused half the sessions, and returned processes,
  SharedRegions, pages, mappings, and caps to baseline. It measured 77 retained
  page-table frames, matching the existing scale behavior.
- `tools/stability_loop.sh 10` passed 10/10 clean M1-M12 boots with zero
  failures for EFI SHA-256
  `c09a41221750958922861712236db5e459bd07739f170ff857cb86fcaa8ecd7e`.
