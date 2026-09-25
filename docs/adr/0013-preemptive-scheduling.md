# ADR-0013: Preemptive scheduling — tick-driven switching from interrupt context

Status: accepted (M3.2, 2026-09)

## Problem

M3.1's scheduler is cooperative: a thread that never calls `yield_now`
owns the CPU forever. Milestone 3.2 requires timer-driven preemption,
per-CPU-ready run-queue *structures* (SMP-ready, single-core execution),
and a deterministic round-robin mode whose regression test proves
preemption — threads making progress **without a single yield call**.

Constraints specific to this kernel right now:

* All threads are kernel threads. There is no user/kernel boundary to
  hang preemption on ("preempt on return to user" — the classic easy
  case — does not exist yet), so a tight kernel loop must be switchable
  at an arbitrary instruction.
* Every line of kernel code so far ran with IF=0 outside bounded tested
  windows. Preemption makes IF=1 kernel execution the normal case for
  the first time; scheduler state must never be observable half-mutated.
* One 32 KiB stack per thread, no per-CPU IRQ stacks yet: the timer
  interrupt nests on the interrupted thread's stack.
* The 100 Hz PIT chain is reclaimed and proven (ADR-0011); vector 32
  currently lands in the M1-era absorb-and-EOI stub whose counter
  semantics the M1/M2 suites depend on byte-for-byte.
* The ADR-0012 switch frame (callee-saved + RFLAGS + RSP) is the only
  context format that exists; adding a second one is a real cost.

## Decision

**Preemption is a cooperative switch nested inside the interrupt's call
chain — no second context format.**

* **Vector 32 gets a dedicated stub** (`arena_irq_timer_stub`, modeled
  on the exception path's full caller-saved save + RBP framing +
  16-byte alignment). Its Rust handler EOIs both controllers (the M1
  dual-EOI discipline), bumps the same absorb counter the M1/M2 suites
  read, then calls an **optional tick hook** — an `AtomicU64`
  function pointer (the house pattern of `RESET_FN`/`LAPIC_EOI_ADDR`),
  unset through boot and M2, installed only by the M3.2 phase. With the
  hook unset the stub's observable behavior is exactly the old absorb
  stub's — every earlier marker is untouched.
* **The tick hook** (`sched::preempt::on_timer_tick`) counts the
  current thread's slice down in PIT ticks. On expiry with a non-empty
  ready ring it runs the *same* `plan_switch` + `arena_context_switch`
  the cooperative path uses. The interrupted thread's complete state is
  then: the hardware iretq frame + the stub's caller-saved saves + our
  standard 80-byte switch frame — all resting on its own stack. Resume
  is ordinary: pop the switch frame → return up through hook, handler,
  stub epilogue → `iretq` into the interrupted body. One context
  format, one switch routine, one resume path.
* **The IF=0 invariant makes it sound**: every scheduler-state mutation
  runs with interrupts masked — cooperative sections enter
  `without_interrupts` themselves, and the interrupt gate hands the tick
  hook IF=0 for free. A tick therefore can never interleave with a
  half-finished decision, and the hook needs no locking beyond the
  cell discipline. RFLAGS is saved IF=0 inside the hook's switch and
  the interrupted thread's IF=1 is restored by its own `iretq` — the
  switch frame never carries interrupt state across threads.
* **Strict RR**: `plan_switch` (shared by yield, exit, and preemption)
  refills the incoming thread's slice on every switch-in, and the ready
  ring is FIFO — rotation order is exactly deterministic, which the
  suite asserts transition-by-transition.
* **Per-CPU structures, single-core execution**: scheduler state moves
  into `[CpuSched; MAX_CPUS]` (`current`, ready ring, slice,
  remaining); `this_cpu()` is 0 until SMP lands (M5) — the structures
  are SMP-shaped now, the execution honestly is not, and nothing
  pretends otherwise.
* **Observability as the test surface**: a capped (from → to) transition
  log plus a total-preemption counter. `preempt_rotation` asserts the
  exact cycle `0→1→2→3` for 12+ consecutive transitions *and* that all
  three busy threads — which contain no yield call at all — made
  progress. `preempt_coexist` runs threads that mix 15 ms busy-waits
  (> the 10 ms slice, so mid-wait rotation is guaranteed) with
  cooperative yields, proving the two switch triggers compose.
* **IRQs nest on the interrupted thread's stack** (no IST for vector
  32): the added depth is a few hundred bytes against 32 KiB, the
  bottom canary is checked on every switch-away including preemptive
  ones, and IST1 remains reserved for #DF/NMI/#MC as since M2.1.
* **Phase discipline**: the M3.1 cooperative tests still run with IF=0
  (ticks cannot perturb their exact assertions); the preemptive phase
  enables the hook + `sti` only for its own bounded windows, suppresses
  *all* serial output while IF=1 (two threads mid-character would
  interleave the log), and re-masks + unhooks before asserting.

* **New threads start interruptible** (`INITIAL_RFLAGS = 0x202`, amending
  ADR-0012's 0x2): the switch's `popfq` installs the frame's RFLAGS on
  first run, so a thread synthesized with IF=0 would receive no ticks
  until its first voluntary yield — a yield-free busy loop would wedge
  the CPU forever (this hang was observed live in the rotation test
  before the constant changed). With IF=1 the first switch into a new
  thread is preemptible from its first instruction, and cooperative
  sections keep their own posture because `without_interrupts`
  save/restores flags rather than forcing them.

## Approaches considered

1. **Nested cooperative switch from the tick hook (chosen)** — zero new
   context formats, zero new assembly shapes, reuses the ADR-0012 frame
   and its register-coverage proof; the cost is call-chain depth on the
   interrupted stack (audited, bounded).
2. **`need_resched` flag + checked safe points** — the flag-setting is
   trivial, but kernel-only code has no natural safe points; a tight
   loop that never checks is never preempted, which fails the roadmap's
   own test by construction. Rejected as the *mechanism* (the flag
   pattern returns in M4 as an optimization on top: preemption at
   user-boundary crossings).
3. **Switch-at-iretq (Linux-style exit path rewriting the interrupt
   frame to resume the next thread)** — a single resume path through
   one epilogue, but it requires a second context format (full
   interrupted state) and epilogue-mutating assembly; strictly more
   machinery for the same observable behavior today. Revisit if IRQ
   nesting depth or wakeup latency ever demands it.
4. **Dedicated per-thread IRQ stacks / IST for vector 32** — removes
   the nesting-depth concern but multiplies stack memory and complicates
   the switch (stack change across contexts); deferred to SMP/user
   preemption when nesting genuinely grows.
5. **LAPIC timer as the tick source** — the right per-CPU source for
   SMP (M5) and finer slices; the PIT is the reclaimed, calibrated,
   already-vector-32 chain (ADR-0011). The hook is source-agnostic: any
   tick can drive it, so this is a swap, not a redesign.
6. **Preemption always-on from kernel entry** — would perturb the M3.1
   cooperative assertions and every boot-phase marker's environment;
   the hook stays unset until the suite arms it. Rejected.

## Consequences

* Kernel code now runs preemptively (IF=1) during armed phases — safe
  today because the only IF=1-sensitive state is the scheduler's
  (IF=0-invariant) and the ADR-0010 irqsave sections already mask
  interrupts; every future subsystem must state its own IF=1 posture
  (convention note added).
* Serial output is a shared resource with no locking yet: the "no
  logging while IF=1" phase rule is load-bearing until a proper console
  lock exists (M5 driver model); it is encoded in the suite's
  structure, not left to discipline.
* Slice granularity is one PIT tick (10 ms) — coarse by design; finer
 * quantum arrives with the LAPIC timer swap.
* The m3 suite grows to 7 markers; `test_m3.py` and the 100-boot
  stability gate move to `m3: RESULT PASS (7/7)`.
* The transition log gives future scheduler work (priority bands,
  aging, per-CPU balancing in M3.3+/M5) a ready-made deterministic
  assertion surface.
