# ADR-0069 — The idle loop halts only when nothing is runnable

Status: implemented on `arena/phase10-desktop-maturity`; complete historical
suite 97/97 on clean source `1231e5a` (receipts in
docs/phase10/DESKTOP-MATURITY.md §8). Not a Phase-10 qualification claim.

## Problem

Interactive profiling of the Phase-10 desktop (`tools/profile_desktop.py`)
showed a near-constant ~9 ms cost for every compositor → displayd PRESENT,
independent of the presented area (480k or 130 pixels), and ~23 ms for every
client → compositor poll. The kernel tick is 100 Hz.

`sched::wake` only re-enqueues a woken thread ("the waker does NOT switch"),
and `block_current` takes the next thread from the round-robin ready ring.
The boot thread's idle loop is always in that ring:

```rust
crate::sched::yield_now();
asm!("sti", "hlt");
```

When a caller blocked after waking its server, round-robin often handed the
CPU to the idle thread first. The idle thread returned from `yield_now` and
executed `hlt` **with the woken server still in the ready ring**, so the
server ran only after the next PIT tick: up to 10 ms per IPC hop, every hop.

## Decision

The idle loop disables interrupts, asks the scheduler whether this CPU's
ready ring holds any entry (`sched::ready_pending`), and halts only when it
is empty. Otherwise it re-enables interrupts and loops (supervisor poll,
then `yield_now` again). `sti; hlt` keeps its atomicity: the check runs with
IF=0, and `sti` takes effect only once `hlt` begins, so a wake raised by an
interrupt between the check and the halt resumes the loop instead of being
lost.

Stale ring entries (threads killed while Ready) are counted until the next
`plan_switch` pops and discards them; they cost at most one extra idle
iteration, never a busy loop or a hang.

## Consequences

* No IPC, capability, timer, spawn or authority semantics change. Threads
  are scheduled in the same round-robin order; the CPU merely stops halting
  while work is queued.
* Measured (TCG): PRESENT 9.5 → 0.6 ms; compositor input-to-frame 11.6 →
  1.2 ms; client poll round trip 23 → 5 ms (together with ADR-0070).
* An idle CPU still halts on every iteration in which nothing is ready, so
  idle power behaviour is unchanged.
* Remaining latency comes from userspace polling (the compositor cannot yet
  be woken by an IPC arrival); Phase 11 adds endpoint-bound notifications
  and a direct-handoff IPC fast path (docs/phase11/PLAN.md, workstream K).
