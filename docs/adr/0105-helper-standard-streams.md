# ADR-0105: Explicit helper standard streams

Status: Accepted
Date: 2026-10-07
Milestone context: Phase 13 native application lifecycle

## Problem

An installed app needs bounded byte communication with a signed helper. The
helper must not inherit the parent's standard-stream page or process
capabilities. A stream endpoint must be independently retired when the helper
or its owning AppInstance ends.

## Decision

AHL1 adds `FLAG_STREAMS`. It is valid only with the existing private timer and
owner-signal grants. This signed bit authorizes the manager to prepare a
dedicated standard-stream set when the application explicitly requests the
streamed launch operation. A normal helper launch and a streamed helper
launch are distinct service requests; the request must match the verified
AHL1 bit.

Desktop creates one fresh SharedRegion page per streamed helper. It places the
exact stream SharedRegion cap in Startup ABI v2 and supplies the helper's
standard stdin, stdout, stderr, and StreamWake roles. The helper receives the
ordinary stdin-reader/stdout-writer directions. Its StreamWake role is the
existing WRITE-only AppInstance owner signal, so output changes wake the
owner without giving the helper a way to consume the owner's event queue.

Only after verified launch and successful spawn does Desktop copy the exact
stream SharedRegion with READ|WRITE|COPY|DESTROY rights into the authenticated
parent's IPC reply. The checked IPC contract requires COPY to stage the reply;
the parent can only copy or destroy its own reference to this one stream page.
The parent-side runtime maps that region and offers stdin-writer,
stdout-reader, and stderr-reader endpoints. It receives no helper Process cap,
timer cap, Desktop authority, or other helper capability. To wake a helper
waiting for input, the parent asks the badge-authenticated Desktop endpoint
to wake the exact helper's private timer notification. Desktop checks the
opaque generation-checked group handle against the caller's AppInstance and
requires the helper to remain live and stream-enabled before notifying it.

On helper wait/reap, terminate, parent death, or AppInstance teardown, Desktop
closes all stream directions, unmaps the owner view, drops the owner cap, and
retires the helper's private timer. The parent may drain already queued output
and observe EOF through its separately held cap; normal parent process teardown
reclaims that cap and mapping. A helper crash or process kill is observed via
the existing Process-cap status and closes the stream page during exact reap.

## Reasoning

Every streamed helper has a distinct one-page ring set and exact SPSC
endpoints. Stream access is explicitly requested and authenticated through
both verified package policy and the caller's per-instance service cap. The
parent's write operation does not reveal timer authority; Desktop uses its
already held exact capability to wake only the selected child.

APB1 v1 and APKG v1 remain unchanged. AHL1 is signed package metadata, while
the StreamRegion and Notification capabilities are the only authorities.

## Downsides accepted

The stream page is one page and each channel has the existing 768-byte ring
bound. The parent wakes a blocked helper through a Desktop service call, adding
one IPC round trip. This keeps the parent's timer authority private and avoids
capability transfer of a child wait object. Helper readiness and wait/reap
remain separate lifecycle operations.

## Security invariants

- A helper without `FLAG_STREAMS` cannot request a parent-visible stream cap.
- A non-streamed helper cannot be woken through this operation.
- The application ID and helper handle select a scoped record only after the
  per-instance endpoint badge authenticates the caller.
- The parent receives only its dedicated SharedRegion with READ|WRITE rights
  plus COPY and DESTROY for the checked IPC reply's staging and cleanup. The
  helper still receives only READ|WRITE.
- No helper inherits the parent's standard stream page or full capability
  table.
- EOF and peer-close bits are page state, not authority; process and cleanup
  operations still use exact capabilities.
