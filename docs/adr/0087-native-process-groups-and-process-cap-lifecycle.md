# ADR-0087: Native child groups retain and finish exact Process capabilities

Status: Accepted
Date: 2026-10-05
Authors: ArenaOS project / Phase 12

## Milestone context

Phase 12.7 and 12.9, implementing the explicit child-authority boundary in
[ADR-0080](0080-phase12-application-platform-and-abi.md). This decision uses
the existing `SYS_SPAWN`, `SYS_PROC_LIVE`, and `SYS_PROC_FINISH` contracts; it
adds no kernel syscall or process/app identity authority.

## Problem

`SYS_SPAWN` returns a positive PID as descriptive metadata and atomically
places a `Process/READ|DESTROY` capability in the caller's first free slot.
A userspace app instance needs to retain each child as a group member, check
whether that exact child is still live, and stop or reap it without turning a
PID, app ID, or process name into authority. Exit notifications are useful
wake hints but may be shared or signalled by other holders; they must not
replace checking the held Process capability.

A group must also refuse its own fixed-capacity overflow before creating a
child. On teardown, failure must not silently discard the still-live Process
capability or the group record needed to retry cleanup.

## Options considered

1. **Keep lifecycle in call sites as raw PIDs and cap-slot integers.**
   Rejected: duplicated code makes it easy to use PID metadata as authority,
   lose a granted Process cap, or consume a wake badge as proof of exit.
2. **Add a kernel ProcessGroup object and implicit parent/child inheritance.**
   Rejected: app-instance grouping is userspace policy; it would add ambient
   authority and duplicate the existing Process-cap lifecycle mechanism.
3. **Use a fixed-capacity userspace group of typed Process-cap wrappers.**
   Chosen: the kernel remains a generic mechanism provider, the existing
   Process cap remains the lifecycle authority, and the group is an opaque
   local handle collection with explicit cleanup.

## Decision

`arena-process::process::ChildProcess::spawn` calls the existing `SYS_SPAWN`
with a bounded explicit `InheritGrant` slice (at most the existing five
`spawn::MAX_INHERIT` entries) and an optional explicit exit-notification slot
and badge. It does not inherit the caller's other capabilities. The ABI mirror
`MAX_SPAWN_INHERIT` records the existing limit; neither the kernel limit nor
any unrelated capacity is raised.

The positive PID payload is retained only for diagnostics and correlation.
Before returning a wrapper, the runtime scans the caller's actual slots and
requires exactly one describable kind-4 Process cap whose descriptive object
field matches that payload and whose rights are exactly READ|DESTROY. If this
kernel-guaranteed cap is absent, duplicated, or malformed, spawn returns an
invariant error rather than treating the PID as a substitute authority.

After construction:

- `ChildProcess::is_live` calls `SYS_PROC_LIVE` with the held Process-cap
  slot; it never passes the PID to the kernel.
- `ChildProcess::finish` calls `SYS_PROC_FINISH` with that slot and an explicit
  `ReapExited` or `StopAndReap` mode. A failed finish leaves the wrapper
  active and retryable.
- An exit-notification badge is only a wake hint. The caller verifies
  liveness through the Process cap before deciding whether to reap or stop.
- `ProcessGroup<T, N>` stores members in the existing generation-checked
  handle table. Group capacity is preflighted before the supplied spawn
  closure runs. A post-spawn record failure returns the still-owned child
  wrapper instead of dropping it.
- `stop_all` stops live children, reaps already-exited children, and removes a
  member only after successful kernel finish. On failure, the refusing member
  and all not-yet-processed members remain addressable for retry/reporting.

The group is single-owner and is not synchronized across user threads. It has
no implicit Drop-time syscalls: app-instance teardown must call explicit group
cleanup. Desktop now uses it for ordinary app children, but installed-app and
helper resolution, startup-record handoff, 32-window scaling, and cleanup on
parent death remain separate gates; this ADR does not claim automatic child
cleanup on parent exit.

## Evidence

- The combined `arena-process` and `arena-runtime` host suites pass 20/20
  (7 + 13). Group tests prove refusal before calling the spawn closure when
  full, generation-stale handles after removal, live-stop versus exited-reap
  selection, and retention of the exact member after a failed finish.
- The independent M12 ring-3 guest spawns the existing proof BootImage with
  zero inherited caps and an explicit exit notification. The child refuses
  its absent startup page and exits; the parent treats the badge only as a
  wake, checks liveness via its Process cap, reaps through the cap, and verifies
  the slot is empty and the group handle stale. `SYS_PROC_LIVE` and
  `SYS_PROC_FINISH` calls made with the numeric PID as though it were a cap slot
  are refused. A second spawn is refused by the full one-member group before
  `SYS_SPAWN` is called.
- The full M12 suite passes 7/7; M1–M7 and M11 regressions pass in the same
  boot, with exact process/cap/frame/resource return.

## Consequences

Child authority remains explicit and native. A numeric PID may label logs or
help identify the cap just returned by `SYS_SPAWN`, but it cannot authorize
liveness, wait, stop, or reap. Generation-checked group handles are local
references only; the held Process capability remains the kernel authority.

The protected Desktop process manager now uses `ProcessGroup` for its real
application children and performs liveness and retirement through group
handles, preserving explicit launch grants and notification-as-wake-hint
semantics. Closeout guest proofs qualify 32 built-in one-window sessions and
the mixed signed-dynamic lifecycle regressions. Installed-app registry launch,
multi-window-per-application behavior, headless helpers, parent-death policy,
streams, and multithreaded synchronization remain open for Phase 13.
