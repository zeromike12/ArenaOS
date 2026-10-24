# Phase 15 architecture audit and accepted decision

Audit baseline: merged Phase-14 `main` at
`1aa35f4c20c9bb386612298fe21574897cf14159`. M accepted ADR-0113 on
2026-10-24. M and C accepted Phase 15 qualification for the exact pinned,
locally modified QEMU 11.1.2 TCG backend. The final source audit is recorded in
`receipts/final-kernel-audit.md`; it found no context-switch or CPU-policy
defect requiring redesign.

## Verified current contracts

- At the Phase-14 audit baseline, `arch/x86_64/context.rs` synthesized an
  80-byte Win64 callee-saved GPR and RFLAGS frame without saving x87, MMX, XMM,
  or MXCSR state. ADR-0113 updates this context-switch contract.
- `sched/mod.rs` owns a fixed 64-slot table on one bootstrap CPU. Yield,
  blocking, exit, and `sched/preempt.rs` timer rotation all use the same
  context-switch leaf. Zombie stack reclamation is deferred until another
  stack is active; join, detached-thread, and process teardown retire records
  through the scheduler.
- User register entry is handled by the syscall/interrupt assembly stubs.
  Syscalls that do not schedule return directly; the compiler-generated kernel
  remains integer-only. The actual EFI image audit is now fail-closed and
  permits only named, counted architecture leaves.
- `idt.rs` keeps CPL0 faults diagnostic and fail-stop. Existing user-mode
  exceptions, except NMI, double fault, and machine check, call
  `exit_on_user_fault`, which reports `0x100 + vector` and terminates the
  faulting user thread through the established process lifecycle.
- Before Phase 15, CPU discovery did not report the baseline FPU/SSE profile,
  no explicit CR0/CR4 or initial register-state setup existed, and the build
  check only searched disassembly text for MMX register names; it could skip
  when no disassembler was installed.

## Accepted decision

Use eager 512-byte, 16-byte-aligned FXSAVE64/FXRSTOR64 state per stable
scheduler slot. The existing common switch leaf gives one save/restore rule
for cooperative, blocking, waking, timer, exit, and address-space transitions.
The 64-slot bound costs 32 KiB, keeps ownership in kernel-only static memory,
and avoids lazy `#NM` ownership. New and reused slots start from a validated
clean image; retired images are reset only after the CPU has switched away.

Startup requires CPUID x87, FXSR, SSE, and SSE2. It explicitly configures and
reads back CR0/CR4, clears OSXSAVE, and when firmware had OSXSAVE active reduces
XCR0 to x87+SSE (`0x3`) before disabling it. AVX-family state remains
unsupported. Delivered CPL3 `#NM/#MF/#XF/#UD` uses existing thread-fault
containment; CPL0 faults remain fatal. Targeted guest tests verified `#MF` and
`#UD`. The unchanged strict guest later verified genuine vector-19 delivery on
the pinned, locally modified QEMU 11.1.2 TCG backend under both CPU profiles,
with status `0x113` and containment. Unmodified QEMU TCG did not deliver the
fault; the accepted result is not upstream or hardware-backed. Ordinary kernel
code stays integer-only.

This decision does not require a security-model change or scheduler redesign.
The unchanged strict guest and official strict T2/T3/T4 gates passed on the
pinned locally modified QEMU TCG backend. M accepted ADR-0113 and M and C
accepted scoped Phase 15 qualification on 2026-10-24; detailed evidence is in
[ACCEPTANCE.md](ACCEPTANCE.md). Unmodified QEMU TCG still lacks the required
exception delivery, and no upstream or hardware-backed result is claimed.
