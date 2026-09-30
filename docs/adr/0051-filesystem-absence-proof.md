# ADR-0051 — Bounded production-fsd absence proof for Phase 8.2

*Status: Accepted (2026-09-30). Internal test-authority correction under
ADR-0047/0048, not a new permission-grant or kernel primitive. No release,
checkpoint or phase-completion claim.*

## Problem and rejected substitutes

ADR-0048 requires the permission path to fail closed when its real FS
backend is absent. A malformed `arena.txt` or a fake FS error tests a
missing *file*, not a missing fsd process. Unplugging the only boot disk
before the historical M5 suite stops the entire kernel before the
manager or permission receiver can run. The production fsd already has
an `FS_OP_SHUTDOWN` poison operation whose **receiving-service** gate
requires a service-issued Notification marker, but it currently has
no production marker cap; its FS client cap held by the Power shell
cannot authorize shutdown. The broker's inherited FS/W cap has no
DESTROY or COPY: widening it for a diagnostic would violate its
four-grant attenuation. Forging a reply or adding a privileged
`PROC_FINISH` cap to kill a kernel-owned process is not a test.

## Decision

Issue **one additional distinct, inert Notification** for production
fsd. Give fsd only Notification/READ as its third boot grant and the
existing trusted Power/raw-FS shell only a transferable
READ|COPY|DESTROY marker in a formerly empty slot 19. The shell uses
its pre-existing FS Endpoint/WRITE to send the existing
`FS_OP_SHUTDOWN` with this exact reference. `fsd`'s existing
`take_diagnostic` receiver check verifies object AND rights and
consumes the IPC-landed reference; no diagnostic primitive is added.
The fsd replies first and exits by its own hand. On the same physical
platter, the broker's next ACQUIRE actively revalidates fsd's real LS
namespace; it must return typed IO and retire cached bearers, never
issue from a cached ALLOW. A forced broker restart on the manager-held
mediator endpoint must fail readiness while fsd remains absent; no
false policy recovery or new bearer. An independent system reboot with
the same platter recreates fsd and recovers the old durable approval.

The global fixed notification bound rises **15→16** (+24 B of static
kernel table); the full-equipped boot must prove 16/16 and typed
seventeenth refusal. This is a *separate FS diagnostic marker*, not
the config update marker, permission approval marker, or manager
private result channel. Broker and app keep **exactly four/one** spawn
grants, and no other caller is given the new marker. The trusted shell
already possesses Power and raw-FS WRITE; no ordinary app, worker,
manager or broker receives new destructive authority. Verify the old
historical diagnostic/refusal suites and final numeric resources, then
qualify the combined final 8.2 image only once. Do not infer arbitrary
no-disk boot support or arbitrary AFS1 commit-sector integrity from
this controlled *real service-absence* fixture.

## Observed last-thread-exit gap and bounded root reap

The first genuine `fs-stop` run replied and exited fsd, but the very
next broker FS_CALL blocked forever: fsd is a kernel-boot-root child,
not a manager-owned Process-cap child, and `SYS_THREAD_EXIT` makes its
thread a zombie without invoking `proc::destroy`. Its endpoint was
neither orphaned nor its blocked clients failed. A fake negative reply
would hide the defect. The boot/idle root which created fsd therefore
retains its actual fsd pid and, once `proc_live_threads==0`, runs the
same `proc::destroy` + `spawn::forget` lifecycle that tears down other
root children. This releases in-flight callers and marks the genuine
FS endpoint orphaned. No userspace Process-cap authority or restart is
granted for fsd; the disk remains intact. The live manager's broker
receives `STATUS_SERVICE_GONE` from the kernel and must fail closed.
Test both the first blocked/acquisition call and later new calls, and
continue to require no false READY on a broker replacement while fsd
is absent. This correction is necessary regardless of permissions:
root-owned service exit must not strand clients behind an endpoint
whose only receiver no longer runs.

## Implemented outcome

The boot root retains the actual fsd pid and reaps its last-thread-exited
root process with `proc::destroy` + `spawn::forget` once, outside fsd's
address space. `test_m82_policy_negative.py` verifies actual fsd shutdown,
the kernel's endpoint-orphaned marker, stale-bearer retirement, typed
backend IO, absence of false broker readiness and same-platter recovery.
The full 40-suite run and final-image 100/100 qualification are recorded
in ADR-0048. This is not a general hot-restart facility for root services.
