# Phase 15 thread-state isolation

Status: implementation and focused isolation cases pass on the recorded QEMU
runs; Phase 15 remains **PARTIAL / UNQUALIFIED** because unmasked SIMD `#XF`
delivery fails under both tested TCG CPU profiles. ADR-0112 remains proposed.
See [qualification](QUALIFICATION.md) for artifact identities and raw receipts.

## Ownership and transition rule

The scheduler slot owns the FP/SIMD image with its saved stack context. The
fixed table has 64 entries. Each image is exactly 512 bytes and 16-byte
aligned; the table is kernel-only and has stable lifetime. The bounded state
cost is `64 × 512 = 32,768` bytes, plus alignment padding.

Every actual context switch saves the outgoing architectural state before
loading the incoming image. The incoming image is restored before its saved
RFLAGS/GPR frame resumes. This is in the common reviewed assembly leaf; no Rust
borrow crosses the switch. The scheduler disables interrupts while choosing
and committing a transition.

That shared path covers voluntary yield, IPC and synchronization blocking and
wake, timer preemption, process CR3 changes, thread exit, and first entry into
a new thread. Direct syscalls that do not switch rely on the kernel's
integer-only instruction policy and leave user state untouched.

## Initialization and cleanup

The clean image is validated before scheduler startup. New threads receive the
startup template before becoming Ready. A thread image remains private even
when other threads share its process address space; separate processes retain
separate slot images across CR3 changes.

A zombie slot remains allocated until execution has switched to another
stack. Reaping clears the FP image and saved context; slot reuse copies the
clean template before publishing the new thread. Process teardown, joined and
detached threads, forced cleanup, and failed thread creation follow the same
lifetime rule. Source review found no restore from an uninitialized image or
save into a freed/recycled slot.

## Guest evidence

The ring-3 fixture checks x87 arithmetic and control/status, scalar SSE/SSE2
arithmetic, conversions and comparisons, MXCSR, and independent patterns in
XMM0–XMM15. It crosses syscalls, yield, a blocking join, timer preemption, and
separate process address spaces. The two worker contexts passed with 40
scheduler switches on `max` and 42 on `qemu64`. Distinct CR3 roots retained
state across 23 and 22 timer switches respectively; a fresh process entered
cleanly in a recycled slot with 18 switches on each profile. Teardown returned
frame and thread accounting to baseline.

The minimal `#XF` probe uses one faulting thread, a same-process sibling, and a
separate peer process. On both profiles it records pre-MXCSR `0x1F00`, then
falls through `sqrtss` with post-MXCSR `0x1F01`; the task returns that encoded
fallback state instead of vector-19 status `0x113`. The sibling and peer each
exit with status 42, the fault slot is released, and resource accounting
returns to baseline. Those receipts prove cleanup and continued execution,
but the `#XF` case is correctly a failure. `#MF` and `#UD` containment pass
independently.

The observed switch counts and exact fallback receipts are in
[`phase15-avx.log`](receipts/phase15-avx.log) and
[`phase15-qemu64.log`](receipts/phase15-qemu64.log). Neither run proves the
unobserved inherited-OSXSAVE/XCR0 sanitation path; both entered firmware with
OSXSAVE clear.
