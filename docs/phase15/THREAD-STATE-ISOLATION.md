# Phase 15 thread-state isolation

Status: design proposed; implementation and targeted guest evidence are
present on the Phase-15 branch. Full qualification is blocked by the SIMD
exception case documented in `QUALIFICATION.md`.

## Ownership

The scheduler slot index owns the FP/SIMD image alongside its saved stack
context. The fixed table has 64 entries. Each image is exactly 512 bytes and
16-byte aligned; the table remains kernel-only and has stable lifetime.

New threads receive the startup template before they become Ready. A thread's
state is private even when another thread shares its process address space.
Different processes use the same scheduler table but retain distinct slot
images across CR3 changes.

## Transition rule

Every actual context switch stores the outgoing architectural state before
loading the incoming image. The incoming image is restored before its saved
RFLAGS/GPR frame resumes. The operation lives in the reviewed assembly leaf;
no Rust borrow crosses the switch. The scheduler continues to disable
interrupts while it chooses and commits a transition.

The shared path covers voluntary yield, IPC/synchronization blocking and wake,
timer preemption, user/kernel transitions, process CR3 changes, thread exit,
and first entry into a new thread. Syscalls that do not switch rely on the
integer-only kernel and leave user state untouched.

## Cleanup

An outgoing zombie is saved while its scheduler slot is still allocated. It is
reaped only after execution has moved to another stack. Reaping clears the FP
image; slot reuse copies the clean template again before publishing the new
thread. Process teardown, joined/detached threads, forced cleanup, and failed
thread creation must not leave an image attached to a later owner.

The bounded memory cost is `64 × 512 = 32,768` bytes, excluding alignment
padding. No dynamic allocation is introduced for architectural state.

Targeted native guest runs passed two distinct worker contexts across yield,
syscall, blocking join, and timer preemption (39–42 scheduler switches), with
all sixteen XMM registers and x87/MXCSR state checked. Two CR3 roots retained
independent images, a new process entered cleanly in a reused slot, and
resource accounting returned to baseline. These results passed on both the
baseline `qemu64` profile and AVX-capable `max` profile. See the qualification
receipt for the outstanding `#XF` failure and exact image hashes.

## Required guest oracle

Independent assembly fixtures must give two or more scheduled threads distinct
patterns in XMM0–XMM15, x87 stack/control state, and MXCSR. Each must perform
arithmetic, cross syscalls, yield, block and wake, and be timer-preempted before
verifying the expected register contents and control/status values. Repeat
across two process CR3 roots, then launch a fresh process after FP-using threads
exit and assert the clean initial state. Thread and process teardown must
return all measured resources to baseline.
