# ADR-0089: Phase-12 IPC burst capacity for 32 ordinary clients

Status: Accepted for Phase-12 implementation; M12 queue and 32-session guest proofs pass
Date: 2026-10-06
Authors: ArenaOS project / Phase 12

## Context

ADR-0075 raised the per-endpoint caller queue from 8 to 16 for the twelve-client
Phase-11 workload. ADR-0088 initially kept that queue unchanged while raising
the managed Desktop target to 32. The real M12 guest exposed that assumption:
with 32 ordinary apps starting together, a child repeatedly received the
kernel's `STATUS_BUSY` from `SYS_IPC_CALL` and exited during service startup,
even with bounded timer retry. This was queue backpressure, not a process,
capability-slot, or SharedRegion refusal. Because the kernel rejects a full
queue before accepting a call or its lent capability, the failure is
mutation-free but still prevents the required workload from reaching a stable
32-session state.

The correction must remain bounded and preserve the IPC v1.1 call/reply,
cap-transfer, and `STATUS_BUSY` contracts. It must not add an endpoint per
window, change notification waiters, make a full queue silently drop calls, or
retry a service-level error that may follow a completed mutation.

## Decision

1. Raise only `ipc::QUEUE_DEPTH`, from 16 to 32. Keep
   `MAX_ENDPOINTS=16`, `MSG_BYTES=64`, per-process timer quotas, and all other
   IPC limits unchanged. A 33rd queued caller is still refused with
   `STATUS_BUSY`; queue-full refusal remains non-accepting and mutation-free.
2. Native graphical clients retry only when the `SYS_IPC_CALL` return code is
   `STATUS_BUSY`. They do not retry a nonzero service status in the reply
   buffer. Retry uses the app's existing private timer/notification, cancels
   every armed timer, uses capped exponential backoff with small time-derived
   jitter, and stops after 512 queue refusals (at most about 17.4 seconds of
   requested waits). Other syscall, timer, decode, and service errors remain
   fail-closed. The Desktop-backed service adapter and ordinary window-client
   adapter use the same bound.
3. Preserve and test the exact-acceptance boundary: the queue admits exactly
   32 blocked callers; caller 33 sees `STATUS_BUSY`; all 32 accepted requests
   remain intact and are each received and replied to exactly once; callers,
   threads, and the proof endpoint return to baseline.

## Resource cost and evidence

`CallSlot` remains 240 bytes and `Notif` remains 32 bytes. On the production
x86_64 target, `Endpoint<32>` is 7,728 bytes, so the fixed 16-endpoint table
uses 123,648 bytes, an increase of 61,440 bytes over the depth-16 table. The
largest `fail_calls_for_server` wake list is now 32 × 16 `u64` thread IDs, or
4 KiB (2 KiB more than before). It remains bounded and allocation-free. The
96 KiB kernel-thread stack is still provisional; its high-water/adequacy proof
is a separate release obligation and this added local storage must be included
in that review.

The M12 boot suite reports `m12:ipc_queue32: PASS` only after the live kernel
queue test above passes. `tools/probe_capspace_layout.py` independently checks
the on-target layout. The end-to-end `tools/test_m12_scale.py` guest boot then
starts 32 real ordinary applications across all six built-in kinds, observes
all 32 sessions stable, and performs the 33rd-launch refusal, half-close,
slot reuse, window operations, and final teardown. Its resource receipts
measured a full-session baseline/working set of 470/30,550 SharedRegion pages,
66 total region records (64 session-owned), 112 maps, 108 steady Desktop caps,
and a 109-cap transient refusal inventory. The full refusal compares all 128
cap descriptors and all seven `SYS_OBSERVE` fields before and after while the
input request's temporary capability is landed; both inventories are exactly
equal. The final teardown returned records, processes, regions, pages, maps,
and caps to the settled baseline. The retained 77 free frames are page-table
frames and remain explicitly measured, not attributed to an application leak.

The 32-session guest uses the six built-in applications only. Signed/dynamic
applications, multiple windows per application, separate headless helpers,
thread/runtime lifecycle breadth, stability, and artifact/archive qualification
remain open Phase-12 gates; this ADR is not a claim that Phase 12 is complete.

## Consequences

The Phase-11 queue depth and its twelve-client qualification remain historical
evidence under ADR-0075. The global kernel IPC table grows by about 60 KiB, and
server-death cleanup reserves an additional 2 KiB of stack in its fixed wake
list. Callers can wait longer under sustained queue pressure, but waits are
bounded, timer-safe, and limited to calls the kernel explicitly did not accept.
A full queue is still a typed refusal, never silent loss or authority transfer.
