# ADR-0100: Stable process exit status through the Process capability

Status: Accepted (2026-10-07)
Date: 2026-10-07
Milestone context: Phase 13 native child lifecycle

## Problem

Process groups can determine whether a child is live and reap it, but
`SYS_PROC_FINISH` does not return the child's final status. The bounded
thread-exit diagnostic ring is not lifecycle authority: traffic can evict an
entry, and a PID-indexed log would not prove possession of the child's
Process capability. Headless applications and future helpers need a stable
way to distinguish normal completion from a fault before reaping the child.

ADR-0049 rejected a Process-exit-status syscall for its bounded readiness
probe because that probe needs proof that a PING reply was valid; a status
code alone cannot prove successful work, while its existing private result
notification already carries that proof. That decision is scoped to
readiness. It does not supply the general lifecycle data needed by native
ProcessGroups.

## Options considered

- Keep only liveness and exit badges. This cannot report why a child ended
  and cannot distinguish successful completion from a fault after the wake.
- Read the diagnostic thread-exit log by PID. Its bounded newest-wins storage
  is evictable, and PID is descriptive rather than authority.
- Add a stable, Process-cap-gated status query. This adds a narrow syscall but
  keeps identity, status lifetime, and reap authority tied to the existing
  Process record.

## Decision

Add syscall 61, `SYS_PROC_STATUS(Process slot, output[2])`. The caller must
hold a live `Process/READ` capability for the target. The output is
`[exited, status]`: `exited = 0` means the process still has live threads;
`exited = 1` carries the full `u64` status of the thread whose exit ended the
process. Status zero is a normal successful exit. A user fault is reported as
`0x100 + vector`.

The status is written to the kernel Process record exactly once when the last
live thread exits, including ordinary thread exit and user-fault teardown. It
remains readable through the exact Process/READ capability until the Process
is reaped. After reap, the capability is stale and the query refuses. The
output pointer must be present and writable in the caller's process.

`arena-process::ChildProcess` exposes this query, and `ProcessGroup` offers a
member-scoped status query and wait helper. Wait treats notifications only as
wake hints and rechecks status through the member's held Process cap after
each wake. Desktop reports the primary status when it exits and continues to
use the exact group for cleanup.

## Reasoning

The Process record already owns the target cap space, address space, and
liveness transition. Keeping final status there avoids a parallel PID index
and makes its lifetime identical to the capability that allows a manager to
reap the child. The result is descriptive lifecycle data only: it grants no
spawn, stop, document, Desktop, or filesystem authority.

Exit status is not a readiness witness. Services that must prove a protocol
result still require a separately authenticated result signal and deadline,
as ADR-0049 specifies.

## Downsides accepted

- This adds one syscall and one optional `u64` field to each bounded Process
  record.
- The final status belongs to whichever thread terminates the process. A
  future multi-thread process policy may need a rule for a primary thread
  exiting while workers remain, but it must preserve Process-cap scoping and
  this final-status lifetime.
- Desktop's serial status line is diagnostic output; applications must query
  through the Process capability rather than parse it.

## Future implications

- Helper supervisors can distinguish normal status from a crash, then choose
  whether to reap, restart, or retire the AppInstance.
- ProcessGroup wait must recheck the exact member status after a wake;
  notification badges and PIDs remain non-authoritative.
- Add tests for concurrent process threads, helper crash, parent death, and
  status stability before reap as helper and user-thread lifecycle mature.

## Evidence

- Kernel target check and arena-process host tests passed. ProcessGroup host
  tests verify that status is scoped to one generation-safe member and keeps
  the complete `u64` value.
- `python3 tools/test_m8_lifecycle.py` passed the live/running state, foreign
  `Process/READ`, wrong-kind, invalid slot, bad output pointer, exact child
  status 42, and stale-cap-after-reap controls on QEMU 10.0.11 / OVMF 2025.02.
- `python3 tools/test_phase13_registry_guest.py` passed signed installed-app
  discovery and launch, two three-window instances, per-window close, a
  timer-driven headless exit with status 42, and identity-resource teardown
  to baseline on QEMU 10.0.11 / OVMF 2025.02.
