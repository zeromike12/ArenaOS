# ADR-0039 — Manager-owned initial stack startup and checkpoint builds

*Status: Accepted. Phase 8.0 remains incomplete until production restart proof.*

## Context

ADR-0037 assigns the production stack's lifecycle to the ring-3 manager,
not the kernel's device supervisor. ADR-0038 bootstraps seven fixed caps,
separate driver-readiness channels and a bounded manifest plan, but
starts no child. `MAX_INHERIT=4` exactly fits the stack's four existing
caps. An additional inherited readiness cap would require widening the
spawn ABI for a channel needed only during startup. A successful spawn
is not proof that the server reached its receive loop.

Users also need a deployable boot image **for each new code commit**.
Previously the qualified EFI and ESP lived under ignored `build/` and
would vanish with the workspace, leaving only a test receipt in chat.
The existing milestone release script can make another commit when
asset upload fails, which would defeat the per-commit artifact promise.

## Decision

After two independent, driver-originated ready signals and fail-closed
live-cap resolution, the manager requests four attenuated grants through
`SYS_SPAWN`. It uses its held Image/READ cap, a fixed-size inheritance
spec and its own event notification (a separate exit badge bit).
It locates the unique child Process cap through the caller-only cap
descriptor ABI; the returned pid is never authority. The *kernel*
independently audits the child's installed slots against the fixed
bootstrap policy and denies any unknown cap. An audit observation is
not a new grant and gives the manager no access to a child's cspace.

For the initial readiness handshake, reuse the stack's existing
READ|WRITE backoff notification (child slot 2, manager slot 4): after
setup, the stack signals a distinct ready badge on it, immediately
before entering its service loop. During startup only the manager
waits on this object; after the badge it stops waiting and the stack
may use it for its own reattach timers. The manager arms a bounded
one-shot timer on this same notification and refuses a late/coalesced
ready event. No fifth inherited cap or shared driver-readiness channel
is introduced. The old M7 fixture may leave the ready bit pending on
its own notification; it is never treated as an exit or a device-ready
signal. The manager stays the sole production spawner of `netstackd`.

Initial startup and a Process handle are **not yet a supervision proof**:
exiting, deliberate stop, repeated reap/restart, stale bearer refusal,
real post-restart wire work, and flat record/frame accounting require
separate tests. Until those pass, no `m8: RESULT PASS` and the Phase 7
production stack-supervision obligation remains OPEN. Do not install a
kernel supervisor entry for this stack as a fallback.

A qualified checkpoint is packaged *before* its one commit under
`releases/checkpoints/<checkpoint>/` as a compressed, self-contained
QEMU bundle with the ESP image, tested EDK2 firmware pair, a formatted
AFS1 scratch-disk template, run instructions, and checksums. The
bundler must reject a missing/non-100/100 receipt or an EFI inside
the ESP that differs from the qualified final EFI, then extract and
boot the bundle before commit. Generated payload is deliberately
included here only because a deployable build per commit is an
explicit user requirement; other build outputs remain ignored. Each
future code commit must update the checkpoint bundle in that same
commit or produce its own qualified bundle, never an extra
asset-only commit lacking its own build.

## Rejected options and costs

- A fifth child grant just for ready: broadens `MAX_INHERIT` without
  necessity. Reusing the *stack's own* backoff object is bounded and
  does not give device drivers another way to signal the manager.
- A debug log line or fixed sleep: not a capability-gated readiness
  event and not a bound.
- Kernel-first production spawn then manager adoption: two lifecycle
  owners, an authority hole, and a pid masquerading as a Process cap.
- `build/`-only delivery, or an untested GitHub upload as the only
  distribution path: neither guarantees a deployable artifact from
  the exact commit.

The bundle adds a few MB per checkpoint to the repository. It is an
explicit exception, not permission to commit the entire build tree.
