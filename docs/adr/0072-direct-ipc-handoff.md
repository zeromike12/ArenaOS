# ADR-0072 — Direct IPC handoff to the woken server

Status: accepted; implemented on `arena/phase11-desktop-maturity` (Phase 11.0).

## Problem

`sched::wake` appends the woken thread to the round-robin ready ring. When a
client CALLs a server parked in RECV, the client blocks immediately, but the
server then runs only after every other ready thread has had its turn. On an
interactive desktop each request crossed the whole ring; the client poll
round trip measured about 4.7 ms on the Phase-10 desktop.

## Decision

`sched::wake_handoff(tid)` puts the woken thread at the **front** of the
ready ring. It is used only where the waker blocks immediately afterwards:

* `ipc::call` waking a server parked in RECV;
* a bound-notification signal (ADR-0071) raised by a call, which wakes the
  server waiting on that notification before the caller blocks.

The woken thread is therefore simply the very next thread to run. Nothing
accumulates at the front, no priority is stored, the FIFO order of every
other ready thread is untouched, and the woken thread receives an ordinary
fresh quantum, so timer preemption is unchanged. IPC queue order,
cancellation tombstones (Phase 8.5) and caller/server death handling are
not touched: handoff only changes which ready thread runs next.

Two variants were tried and rejected on evidence:

* **Handoff on REPLY** (front-inserting the caller while the server keeps
  running) livelocked the system under the phase-9 graphics fixture, whose
  two windows poll the compositor in tight loops: the busy group kept
  re-entering at the front ahead of the whole ring and the shell never ran
  (`test_m4` timed out with the CPU in kernel code). Replies are ordinary
  FIFO wakes.
* **Quantum donation** (the woken server inheriting the caller's remaining
  quantum) brought no measurable latency gain and caused the display server
  to be preempted mid-copy; the woken thread gets a normal quantum.

## Consequences

* Same host, TCG: built-in client poll IPC round trip 4.7 ms → 0.93 ms mean;
  Terminal key-to-photon p50 35.0 → 17.3 ms; pointer motion-to-photon p50
  14.1 → 12.3 ms (together with ADR-0071; host screendump ~4.8 ms included).
* `m11:handoff_order` proves a handoff wake runs before an earlier ordinary
  wake; the RED control that turns `wake_handoff` back into an ordinary wake
  fails it. The historical suites (phase-9 polling fixture included) are the
  regression guard against the livelocking variant.
