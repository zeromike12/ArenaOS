# ADR-0010: Synchronization — spinlock with owner tracking, irqsave critical sections

Status: accepted (M2.6, 2026-09-25)
Context refs: ADR-0003 (boot contract: BSP-only, IF=0), ADR-0009 (heap promised a lock wrap at M2.6), ROADMAP 2.6

## Problem

Every subsystem after M2.5 (scheduler, drivers, the kernel proper) mutates
shared state. The boot stage has gotten by on a *documented absence* of
synchronization (`SyncCell`, single CPU + IF=0), which must not migrate
past boot. We need: a lock primitive, a way to build interrupt-safe
critical sections, and debug support for the two classic lock bugs —
forgotten release and recursive acquisition.

## Approaches considered

1. **Ticket/queued spinlock (MCS, array-qlock)** — fair under SMP
   contention, but the fairness machinery cannot be *exercised* until SMP
   exists (M3+); testing what cannot trigger is ceremony, not safety.
2. **Test-and-set spinlock now, queued later behind the same API** —
   trivially correct on one CPU, already SMP-*shaped* (atomic CAS loop),
   and the ROADMAP explicitly defers the style decision to an ADR when
   SMP lands.
3. **Disable interrupts around all shared state (no lock type)** —
   workable single-core, but untestable semantics for nesting, hostile to
   the future SMP kernel, and hides rather than structures critical
   regions.
4. **Cooperative yielding primitives (sleeplocks)** — need a scheduler;
   out of scope until M3.

## Decision

Option 2, plus irqsave helpers, split by the now-established pattern:

**`kernel/libs/sync` (crate `arena-sync`)** — `Spinlock<T>` +
`SpinlockGuard`, zero deps, `no_std`, host-testable:

- Acquire: `compare_exchange_weak` loop + `spin_loop()` hint. Release:
  guard `Drop` (Release store). RAII everywhere; no manual unlock API
  exists, so "forgotten release" is unrepresentable.
- **Owner tracking**: the lock records a `cpu_id` token (function pointer
  supplied at construction — constant BSP id `0` at boot, per-thread
  tokens in host tests). `owner_token()`/`is_locked()` are diagnostic
  accessors; `lock()` fires a `debug_assert` when the current executor's
  token already owns the lock — recursive acquisition is a *debug panic*
  instead of a silent deadlock. Release builds deadlock by design
  (standard test-and-set semantics, documented).
- Ordering discipline: Acquire on the held transition, Release on drop;
  the owner token is Relaxed diagnostics and orders nothing. Owner is
  cleared *before* the held flag on release so a spinner can never
  observe its own stale token.
- Host tests run on real parallel threads: four threads × 50,000
  non-atomic increments through the lock yield exactly 200,000; a
  `Vec` handoff loses/duplicates nothing; `#[should_panic]` proves the
  recursion assert fires.

**Boot crate** — arch-level interrupt control, composed not embedded:

- `arch::x86_64::{read_flags, restore_flags, interrupts_enabled, cli,
  sti}` — pushfq/pop and push/popfq primitives (`RFLAGS_IF` named, no
  magic constants).
- `sync::without_interrupts(f)` — the only sanctioned irqsave/irqrestore
  critical section: saves full RFLAGS, `cli`, runs `f`, restores the
  *exact* saved flags — IF comes back **only** if it was set on entry, so
  nesting is correct and a section entered with IF=0 never accidentally
  enables interrupts on exit. (A panic inside `f` skips the restore —
  acceptable: the panic path halts the machine.)
- **The spinlock does not mask interrupts internally.** That is deliberate
  layering: whether a critical region needs IF control is a per-subsystem
  deadlock-policy question; composing `without_interrupts(|| lock…) ` or
  `lock()` alone stays explicit at the call site.
- **Heap is now lock-wrapped** (ADR-0009's promise discharged):
  `static HEAP: Spinlock<Heap<FrameChunkProvider>>`; the entire M2.5 test
  set (basics/guards/stress) re-passes through the lock, and
  `spinlock_basics` allocates under an *outer* lock to prove distinct
  locks nest cleanly.

**Boot interrupt posture is unchanged**: IF=0 except bounded, tested
windows (M1 interrupt test, M2.2 `tick_rate`, M2.6 `crit_section`). The
PIT tick still lands in the vectors-32..255 absorb-and-EOI stub; a real
per-vector dispatch table arrives with the first actual device driver —
building one now would be untestable scaffolding.

## Reasoning

- Single-core correctness first is not a shortcut: on the boot CPU the
  CAS loop is provably uncontended, so the in-guest tests assert the
  *state machine* (held/owner/try_lock/drop/persistence/composition)
  while the host suite asserts *real parallel behavior* — each side
  proves what it can prove honestly, and neither fakes the other.
- Owner-token recursion detection converts the worst spinlock failure
  mode (silent self-deadlock → watchdog-less hang) into a located debug
  panic — consistent with ADR-0005 (crashes must diagnose).
- Keeping IF control out of the lock avoids the classic
  lock-with-irqsave API split (`spin_lock` vs `spin_lock_irqsave`) until
  there is a scheduler to justify it.

## Measured behavior (evidence)

Host: arena-sync 6/6 — including exact 200,000 under four-thread
contention and the recursion `#[should_panic]`.

Guest (m2 16/16): `crit_section` — 2 ticks delivered with IF=1, **0**
crossed a 25 ms `without_interrupts` section *entered with IF=1*,
delivery resumed afterward (EOI discipline intact), RFLAGS restored
exactly; serial: `crit_section: 2 ticks delivered, 0 crossed a 25ms
section entered with IF=1, resumed, RFLAGS exact`. `spinlock_basics` —
full state machine + heap-under-outer-lock composition. All 14 prior M2
tests and 8 M1 tests re-pass with the heap behind the lock.

## Disadvantages / limits

- Test-and-set spins unfairly under future SMP contention (cache-line
  ping-pong); queued replacement is a scheduled ADR when SMP lands.
- Recursion detection is debug-only (release deadlocks — documented).
- Owner token is a single `u32` per lock: no lock-stack tracing, no
  deadlock *detection* (only recursion); lock ordering is by convention
  until the kernel proper grows a dependency audit.
- `without_interrupts` restores full RFLAGS — safe for the saved-context
  pattern used here, but must not be fed arbitrary flag values.

## Revisit triggers

- SMP enablement (M3+): queued/ticket decision, real CPU ids, lock-order
  validation, possibly irqsave variants of `lock()`.
- First device driver needing per-vector handlers → dispatch table ADR.
- Scheduler (M3): sleeplocks/channel primitives compose on top.
