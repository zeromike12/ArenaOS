# ADR-0012: Kernel threads and the context switch — callee-saved frame, no FPU state (build-enforced)

Status: accepted (M3.1, 2026-09); amended by ADR-0013 (M3.2): a brand-new
thread's synthesized frame now carries RFLAGS=0x202 (IF=1) instead of 0x2,
so threads are preemptible from their first instruction. Everything else
stands.

## Problem

Milestone 3.1 introduces kernel threads and the context switch between
them: the assembly fast path, and — per the roadmap's explicit deferral —
the decision on *what state a switch must carry* ("full state: GPRs,
FPU/SSE state lazily or XSAVE — ADR at the time"). This is that ADR.

Constraints inherited from earlier decisions:

* The kernel targets `x86_64-unknown-uefi`, whose "C" ABI is **Win64**:
  `rbx, rbp, rdi, rsi, r12–r15` are callee-saved (six registers, not the
  SysV five minus two), and the compiler may keep live thread state in
  any of them across a call — including across our switch.
* The kernel is `no_std`, zero-dependency (ADR-0004), built soft-float.
* The boot contract is single-CPU, IF=0 outside bounded tested windows
  (ADR-0003/0011); 3.1 scheduling is cooperative — preemption arrives in
  3.2 on top of whatever frame this ADR fixes.
* CODING-CONVENTIONS: `unsafe` quarantined in tiny leaf modules; nothing
  fake — every mechanism gets a machine-checked test.

## Empirical basis (measured, not assumed)

The whole-image disassembly of the M2 kernel (`objdump -d
build/arena-boot.efi`, 17.6k lines) contains **zero** `xmm`/`mm0–7`
operands and zero x87/SSE opcodes: the soft-float target plus
integer-only `no_std` code never touches FPU or SIMD state. The kernel
therefore has *no* FPU/SSE state of its own to preserve across a
switch — and this ADR converts that observation into a standing,
build-enforced invariant (below) rather than a hope.

## Decision

**Switch frame: the eight Win64 callee-saved registers + RFLAGS + RSP,
nothing else.** `arch/x86_64/context.rs` (the quarantined unsafe leaf)
provides `arena_context_switch(save_slot, restore_rsp)`:

```text
push rbx, rbp, rdi, rsi, r12, r13, r14, r15   (8 × 8 B)
pushfq                                        (RFLAGS)
mov [rcx], rsp        ; save into the outgoing thread's context slot
mov rsp, rdx          ; onto the incoming thread's stack
popfq; pop r15..rbx
ret                   ; resumes wherever the incoming stack says
```

Caller-saved registers need no frame: the switch is an ordinary Win64
call, so the *compiler* already spilled anything live around it. A new
thread's first context is a synthesized image of exactly this frame
(zeroed registers, RFLAGS=0x2 — reserved bit set, IF=0 like the rest of
the kernel — return address = `arena_thread_trampoline`, RSP 16-byte
aligned at the trampoline entry per Win64). The trampoline runs the
thread's Rust entry point and then the exit path; it never returns.

**No FPU/SSE state in the switch — enforced at build time.**
`tools/build.sh` disassembles the freshly linked image and **fails the
build** if any `xmm`/`mm[0-7]` instruction appears (ADR-0012 invariant).
If that guard ever fires — e.g. LLVM lowering a large `memcpy` to SIMD —
the build stops and this ADR is revisited in order: (1) add
`-C target-feature=-sse,-sse2,-mmx` (cheapest, keeps the frame), (2)
extend the switch with `fxsave`/`fxrstor` or XSAVE area swaps (512 B–4
KiB per thread, only if (1) is ever unacceptable). User-thread FPU state
is explicitly *not* this switch's problem: it crosses the kernel
boundary at syscall/interrupt entry and is an M4 concern (the kernel
preserves the *user's* state around the whole kernel excursion, not
around kernel-thread switches that by invariant use no FPU).

**Threads, stacks, lifetime.**

* `KThread`: id, name, state (`Ready`/`Running`/`Zombie`), context slot
  (saved RSP), entry+arg, and stack ownership (base, frame count).
  Threads live in a heap-backed `Vec<Option<KThread>>` — **stable
  indices**, slots freed to `None` on reap (Vec compaction would
  invalidate every saved index).
* Kernel stack: **32 KiB (8 contiguous frames)** from the M2.3 frame
  allocator — physically contiguous, directly mapped in the kernel view,
  and the right granularity (heap chunks are 64 KiB minimum and blocks
  cannot span them, ADR-0009). Bottom qword holds a **stack canary**;
  every voluntary switch and the reap path check the *outgoing* thread's
  canary, and a violation halts the machine with diagnostics — an
  overflowed kernel stack is never safe to continue on.
* The **bootstrap thread is `kmain` itself** (thread 0, registered by
  `sched::init()`), running on the reserved boot stack it does not own
  (`stack_frames = 0` ⇒ nothing to free, no canary).
* **Exit is deferred-reaped**: a thread cannot free the stack it is
  running on, so `exit` marks it `Zombie`, switches away, and the next
  scheduler entry frees zombie stacks (`free_contiguous`) and clears
  their slots. The churn test pins this down with exact accounting:
  after 64 threads run and are reaped, `frames::free_frames()` and
  `heap::bytes_reserved()` must return to the recorded baseline.

**Scheduler discipline (3.1).** One round-robin ready queue of thread
indices, `yield_now()` as the only switch trigger (cooperative), all
scheduler state mutated under `without_interrupts` (ADR-0010's irqsave
section) — and, critically, **no live borrow crosses the switch**: the
decision (pick next, update states, enqueue current) completes and the
borrow ends *before* the assembly switch runs on raw values
(`*mut u64` save slot + `u64` restore RSP). When the thread is later
resumed, it re-acquires. Determinism is a feature: the RR-interleave
test asserts the *exact* global sequence A,B,C,A,B,C,… — no heuristics,
matching ARCHITECTURE §5's "round-robin, no heuristics" test mode. 3.2
adds the timer-driven preemptive path and per-CPU queue structures on
top of this same frame.

## Approaches considered

1. **Callee-saved-only frame (chosen)** — ~20 register moves per switch;
   correctness rests on the Win64 ABI (which the compiler is already
   honoring at the call site) plus the build-enforced no-SIMD invariant.
2. **Save all 16 GPRs** — self-contained against ABI drift, but the
   switch is called from Rust, so caller-saved registers are provably
   dead at that point; the extra 8 saves/restores buy nothing.
3. **Eager XSAVE/fxsave per thread** — 512 B–4 KiB of save/restore per
   switch for state the kernel never touches; pure cost until userspace
   exists (and then it belongs at the user/kernel boundary, not here).
4. **Lazy FPU (CR0.TS + #NM trap switching)** — the classic optimization
   for *user* threads that may or may not use the FPU; adds a trap path
   and per-thread ownership bookkeeping to save nothing today (no kernel
   FPU use, invariant-enforced). Revisit at M4 for user threads.
5. **Hardware task switch (TSS task gates)** — removed in x86-64 long
   mode (task gates only survive for #DF; TSS is register storage).
6. **Full interrupt-context preemption frame now** — 3.2's job; building
   it before the cooperative mechanism is proven would stack two
   untested mechanisms.

## Consequences

* Accepted downsides: the no-FPU invariant is a *build-time* contract,
  not a type-system one — the objdump guard is the enforcement, and it
  must survive toolchain changes (it runs on every `build.sh`, including
  the release gate). Deferred reaping means zombie stacks linger until
  the next scheduler entry (bounded, and pinned by the churn test's
  exact accounting). 32 KiB stacks are a guess tuned by the deep-stack
  test; per-thread sizing becomes a spawn parameter when real kernel
  services (M3.3+/M5) have differing needs.
* `sched.rs` gives M3.2 a running substrate: the timer tick hook only
  needs to make `yield_now`'s decision at interrupt time (with the full
  interrupted-context frame that preemption requires — its own ADR).
* The thread table, RR queue, and stable-index discipline are the
  single-CPU instance of ARCHITECTURE §5's per-CPU run queues; 3.2
  widens the structure, not the contract.
* Tests (all machine-checked, `m3:test:*`): spawn/run-to-completion with
  side-effect verification, exact RR interleave, callee-saved register
  round-trip *through* a real switch (asm sets all eight registers,
  yields, reads them back), two-thread stack isolation with disjoint
  stack ranges and deep recursion, and 64-thread churn with exact
  frame/heap accounting back to baseline.
