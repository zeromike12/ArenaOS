# ADR-0049 — A verifiable bounded permission-service readiness witness

*Status: Accepted (2026-09-30). Narrow correction to ADR-0048's worker success evidence; **all other ADR-0048 decisions and completion gates remain in force**. Acceptance is not an implementation or Phase 8.2 completion claim.*

## Problem

ADR-0048 specified a bounded PING worker inheriting only mediator Endpoint/WRITE, with a kernel exit badge and manager timer. But an exit badge attests only that the child exited, **not its exit status**. `SYS_PROC_FINISH` does not report child exit status to userspace. A worker whose IPC failed, whose PING response was invalid, or that faulted also exits. Exit + no deadline is therefore insufficient to mark the broker READY. Calling PING directly from the single-threaded manager is unbounded when the receiver does not dispatch.

## Decision

The readiness worker inherits **exactly two attenuated caps**: mediator Endpoint/WRITE, and WRITE to the manager's **existing private result/restart Notification**. It must validate the entire typed PING reply (including returned-cap absence and fixed reply body) before sending a dedicated success badge. The manager requires success **and** kernel exit before its timer deadline, reaps the held Process cap, and marks the broker READY only then. Deadline, missing success, wrong/extra badge, failed reap, and worker/receiver death are failures; deadline wins if observed even alongside success/exit. On timeout, the manager forcibly stops a still-live worker and broker using their held Process caps. Reuse the existing notification object with disjoint badges; do not create a new notification or modify the one blocking mediator receive endpoint. Existing ADR-0045 `depcheck` result/exit/timer mechanics provide the implementation pattern. Adapt its image20 worker with a separately audited mode only if its cap shape and behavior can be distinguished safely; a distinct image requires a fresh explicit image/resource audit.

This changes **only** the probe worker's grants (one → two) and its success witness. The broker still inherits exactly four caps, the app exactly one; `MAX_GRANTS=MAX_INHERIT=5` remains unchanged. The worker gets no approval marker, raw FS authority, rngd cap, or broker service side. This introduces no thread, IPC primitive, kernel cap kind, or notification object. The additional grant occupies one temporary cap slot; measure actual live resource peaks rather than claiming the pre-code envelope as observed fact.

## Rejected alternatives

- Trust exit alone, a debug log, or the Process handle as a successful PING: these cannot distinguish a failed reply.
- Add a Process-exit-status syscall: changes the kernel ABI for a case existing private notifications already solve.
- Give the broker a fifth readiness notification: needlessly broadens its service authority and invalidates ADR-0048's exact four-grant audit.

## Proof obligations

Audit the actual two worker cap grants, distinct badge bits and existing manager-owned notification; prove a correctly typed PING signals success, wrong/short reply or missing result signal does not, and a stalled worker cannot strand its or the broker's Process cap. Require both badges, no deadline, and successful reap before app startup; preserve all historical dependency-probe behavior. The rest of ADR-0048's guest, persistence, restart, negative-space, delegation, ordering, resource, full-suite and artifact-bound qualification gates remain open.

## Implemented witness (partial 8.2 checkpoint only)

The manager reuses image20 in an exact two-grant notification mode: the
worker cap-describes mediator Endpoint/WRITE and the existing manager
Notification/WRITE, refuses a third grant, validates complete typed PING
reply plus cap absence, then sends the dedicated success bit and exits.
The manager observes both success and kernel exit before deadline and
reaps the held Process cap. Guest diagnostics independently reject wrong
PING result, blocked PING with broker-first timeout and blocked caller-first
teardown/restart (ADR-0050). Both failure orderings return no READY and
recover exact resources; caller-first deliberately exercises the kernel
repair. The fresh 35-suite historical run and final-EFI-bound 100/100
ordinary boots qualified the **volatile** integration checkpoint. The
persistent permission policy and the remaining ADR-0048 completion gates
are not implemented or claimed by this witness.

Phase 8.2 was subsequently completed with persistent policy, real
broker-restart, absent-backend and crash-model proofs; the independently
qualified final image and receipt are recorded in ADR-0048. This ADR's
partial-checkpoint evidence remains historical, not the final-image proof.
