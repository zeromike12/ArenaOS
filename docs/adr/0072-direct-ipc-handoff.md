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

## Amendment (Phase 11.3): causal order

The complete historical suite on the first 11.0 commit failed
`test_m8_dependencies` (restart scenario): the shell notifies the service
manager to STOP the network stack and then CALLs the stack's endpoint,
expecting the old bearer to be refused. With unconditional handoff the
CALL put the still-parked old stack server at the front of the ready
ring, ahead of the manager the shell had woken a moment earlier, so the
old server accepted the old bearer (`m8: stackstop FAIL (old bearer
accepted or wrong transport)`). Before handoff, FIFO order ran the
manager first.

Rule: a handoff never overtakes a wake the calling thread itself caused
earlier in its current run. The scheduler keeps one per-CPU flag,
`woke_others`, set by every ordinary wake (including wakes raised from
interrupt context during the run — conservative) and cleared when a
thread is switched in. A handoff goes to the front only while the flag is
clear; otherwise it is an ordinary back-of-ring wake, exactly the pre-11.0
order. The desktop's hot paths (client → compositor CALL, inputd →
compositor CALL) wake nobody before calling, so they keep the handoff.

Proof: `m11:test:handoff_order` checks both halves (a handoff runs ahead
of a thread another thread woke; it never runs ahead of the caller's own
earlier wake), and `test_m11_event_red.py` carries the `handoff-causal`
RED control that removes the flag check.

## Amendment (Phase 11 completion): bounded reply handoff

**Evidence.** The latency investigation traced the remaining pointer
cost to the reply wake. On the final feature set the broker's synchronous
display call took 3.5 ms per cursor frame. The copy itself is about
0.1 ms for a 2 kpx GOP rectangle. The rest was displayd's FIFO reply
wake: the broker waited behind every ready thread. A temporary
reply-handoff build changed, on the same host:
* cursor PRESENT: 3.5 → 0.63 ms;
* guest input-to-frame: 5.0 → 1.7 ms;
* host motion-to-photon p50: 12.2 → 6.5 ms, including about 4.5 ms of
  screendump.

**Why the original variant livelocked.** `plan_switch` gives every
incoming thread a fresh quantum, and production runs without preemption
(the m3 suite arms it only for its own tests). A client and a server that
hand the CPU to each other by CALL and REPLY handoffs therefore never
leave the front of the ring. The phase-9 polling fixture did exactly
that, and the shell never ran.

**Rule.** A REPLY wakes the caller with `wake_handoff`. The causal rule
still applies: a server that already made another thread ready in its
current run replies FIFO. Handoffs are additionally bounded by a chain
budget:
* A chain is every run since the ring last delivered an ordinary pick.
  `plan_switch` ends it when the picked thread is not the one a handoff
  placed at the front.
* The first handoff starts the chain on the TSC clock.
* Once the chain is `HANDOFF_CHAIN_US` (10 ms) old, every handoff is an
  ordinary FIFO wake. The ring then turns at the next block, so a
  rendezvous pair gets at most one budget of CPU per ring rotation,
  whatever the preemption setting.

Front insertions within one run are LIFO: the most recent rendezvous runs
first. FIFO order among all other ready threads is unchanged.

The broker also wakes the clients a request gave events to *before*
replying. By the causal rule that makes the reply FIFO behind them, so a
typed key reaches its application before inputd delivers the key's
release.

**Proof.**
* `m11:test:handoff_chain`: two kernel threads hand off to each other for
  up to 400 000 round trips. A thread made ready behind them must run
  first; it runs after about 4 000 trips, roughly one budget.
* `test_m11_event_red.py` carries two new mutants:
  * `handoff-chain` lifts the budget, and the bystander starves
    (`handoff_chain: FAIL`);
  * the `handoff-causal` needle follows the new condition.
* The historical suite, including the phase-9 polling fixture, guards
  against the livelock.
