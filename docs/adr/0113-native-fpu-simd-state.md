# ADR-0113: Native x87/SSE execution with eager per-thread state

Status: **Accepted** by M on 2026-10-24.
Date proposed: 2026-10-11; date accepted: 2026-10-24.
Milestone context: Phase 15

## Numbering

This proposal uses ADR-0113. Phase-14 mainline already accepts ADR-0110, and
the active C-runtime workstream has a cross-branch reservation in the 0112
range. The inspected Phase-15 base, Arena 1 head, and Arena 2 head contain no
ADR-0113. No accepted ADR was renumbered. Branch identities and the decision
are recorded in `docs/phase15/receipts/adr-numbering-review.md`.

## Problem

ArenaOS can enter ring 3 and schedule native threads, but the CPU execution
contract currently leaves x87/SSE facilities inherited from firmware and
saves no floating-point or vector state. A process can therefore observe
another thread's registers, lose its values at a scheduling transition, or
raise a CPU exception that must be contained without halting the kernel.

ADR-0012's accepted integer-only kernel rule remains in force. This proposal
adds only a reviewed architecture leaf for CPU state management and does not
approve compiler-generated floating-point or SIMD arithmetic in ordinary
kernel code.

## Decision

Use **eager per-thread FXSAVE64/FXRSTOR64** on every actual scheduler context
switch. The initial architectural profile requires CPUID x87 FPU, FXSAVE/FXRSTOR,
SSE, and SSE2. Additional SSE-family instructions may execute only when CPUID
reports them. AVX, AVX2, AVX-512, YMM upper halves, ZMM, and AMX state are
unsupported. If firmware has CR4.OSXSAVE enabled, initialization requires CPUID
leaf 0xD to advertise the x87 and SSE XCR0 components, writes XCR0 to the legacy
profile `0x3`, verifies it, and then clears CR4.OSXSAVE. When firmware already
has OSXSAVE clear, XCR0 is not accessible and is left untouched; extended
vector execution remains disabled by the cleared CR4 bit.

M accepted this architecture on 2026-10-24. The decision is scoped to the
documented x87/MMX/SSE/SSE2 state profile; AVX-family execution remains
unsupported. Qualification evidence and backend limits are recorded below.

## CPU initialization and refusal

At kernel entry, before scheduler threads or user execution begin:

1. Read CPUID leaf 0/1 and the extended feature leaf when present. Refuse
   initialization if x87, FXSAVE, SSE, or SSE2 is absent.
2. Set CR0.MP, CR0.NE; clear CR0.EM and CR0.TS. Read CR0 back and require the
   requested values.
3. Compare CPUID.OSXSAVE with the inherited CR4.OSXSAVE bit. If set, sanitize
   XCR0 to `0x3` (x87+SSE only) and verify the readback while OSXSAVE is active.
4. Set CR4.OSFXSR and CR4.OSXMMEXCPT; clear CR4.OSXSAVE. Read CR4 back and
   verify all three decisions.
5. Initialize x87, MXCSR, and XMM0–XMM15, then capture and validate the clean
   FXSAVE64 template. Use the architectural MXCSR mask fallback `0x0000FFBF`
   when the processor reports a zero mask; the initial value is
   `0x00001F80 & mask`.

Any missing required feature, failed readback, invalid template, or unsupported
MXCSR default is a boot failure with a diagnostic. The kernel must not start
user processes with partially configured state.

## Ownership, storage, and initialization

The scheduler owns one kernel-only state image for each stable scheduler slot.
An image is exactly 512 bytes, 16-byte aligned, and remains allocated for the
slot's lifetime. With `MAX_THREADS = 64`, storage costs 32 KiB plus at most
alignment padding. State images are not capabilities and are not mapped into
user address spaces.

The clean template has x87 control word `0x037F`, no pending x87 status,
empty x87 tags with cleared data, the validated initial MXCSR, and zeroed
XMM0–XMM15. Before a thread slot becomes Ready, its image is copied from this
template. Slot reuse repeats that initialization. The bootstrap thread's first
outgoing image is captured before any incoming thread is restored.

## Save/restore and scheduling order

The common `context::switch_context` leaf receives raw pointers to the outgoing
and incoming images. With interrupts disabled and all scheduler borrows ended,
it performs FXSAVE64 to the outgoing image, saves the existing GPR/RFLAGS frame,
FXRSTOR64 from the incoming image, then restores the incoming RFLAGS/GPR frame
and returns. The image pointers refer to kernel-half memory mapped in every
process CR3. No Rust reference may cross this boundary.

Yield, IPC blocking, synchronization waits, wake/resume, timer preemption,
thread exit, and address-space changes all continue through this common leaf.
Syscall and interrupt entry do not need separate FPU save code because ordinary
kernel code remains integer-only and every scheduler transition uses the leaf.

## Exceptions and lifecycle

An unexpected CPL0 `#NM`, `#MF`, `#XF`, or `#UD` remains a diagnostic kernel
failure. A CPL3 `#NM`, `#MF`, `#XF`, or `#UD` follows the existing user-thread
fault teardown path; sibling threads and other processes continue. Eager mode
keeps CR0.TS clear, so `#NM` is not an ownership-switch mechanism. Tests must
prove an unmasked user floating-point fault cannot halt the machine.

Zombie images remain allocated until the scheduler has switched away. Reaping
resets the slot image before it can be assigned to a new thread. Process exit,
forced cleanup, detached-thread cleanup, and slot reuse use the same reset rule.

## Kernel instruction policy

The full EFI image audit continues to reject ordinary kernel x87 arithmetic,
MMX, SSE, AVX, and other vector instructions. Only the explicitly reviewed
`x86_64::fpu` initialization, conditional XCR0 sanitation, and FXSAVE64/
FXRSTOR64 switch leaves may contain the allowlisted state-management
instructions. Missing or failed disassembly is a qualification failure. Host
negative controls must prove that prohibited instructions are detected.

## Options considered

1. **Eager FXSAVE64/FXRSTOR64 per thread (chosen proposal):** simple ownership,
   bounded 512-byte images, and one rule for cooperative and preemptive paths;
   costs one save and restore per actual switch.
2. **Lazy `CR0.TS`/`#NM` ownership:** avoids saves for threads that never use
   FP, but introduces ownership transfer, kernel-entry, and fault recovery
   state that must be correct for both user and kernel threads.
3. **XSAVE/XRSTOR:** supports larger future state but adds CPUID/XCR0 sizing,
   variable per-thread storage, and AVX/AVX-512 policy outside this milestone.

The current single-BSP scheduler and fixed thread table make the first option
the smallest stateful design. SMP or AVX support requires a new decision.

## Qualification and rollback

Qualification requires source and host checks, actual ring-3 guest execution,
thread and process isolation, syscall/block/wake/timer transitions, fault
containment, Phase-14 compatibility, historical suites, fresh T2/T3/T4 boots,
and independent extraction of the exact tested archive. T4 requires 100/100
fresh boots with zero failures. No historical receipt can qualify this change.

**Current decision and qualification (2026-10-24).** M accepted ADR-0113.
Phase 15 qualification was accepted by M and C for the exact locally modified
QEMU 11.1.2 TCG backend, using explicit `-accel tcg`. The unchanged strict guest
passed 6/6 under both `max,+nx,+smep,+smap` and `qemu64,+nx,+smep,+smap`, with
genuine instruction-generated vector 19, status `0x113`, fault-thread
termination, sibling and independent-process survival, scheduler-slot and
resource reclamation, continued kernel operation, and clean shutdown. The
strict T2/T3/T4 gates passed 10/10, 25/25, and 100/100; the current registered
suite passed 121/121. Expanded independent legacy SSE/SSE2 controls, canonical
build, host checks, and the integer-only kernel ISA audit passed. C reviewed the
evidence and confirmed the PR-head kernel, userspace, and tools match qualified
source commit `f54797f2fb5bac0b3440f986d2120382b3611ecc`. See
[`docs/phase15/QUALIFICATION.md`](../phase15/QUALIFICATION.md) and
[`docs/phase15/ACCEPTANCE.md`](../phase15/ACCEPTANCE.md) for the review record,
artifact identities, and receipts.

This acceptance is limited to the exact pinned, locally modified TCG backend
and tested instruction subset. It is not upstream-QEMU, hardware-backed,
Windows/WHPX, or comprehensive SIMD qualification. KVM was unavailable because
`/dev/kvm` was absent. The external KVM/WHPX kit remains a separate optional
hardware execution path and did not produce the accepted result.

**Historical unsuccessful attempts (superseded by the results above).**
Unmodified QEMU 8.2.2, 10.0.13, and 11.1.2 TCG trials failed the strict
unmasked `SQRTSS` delivery check: with MXCSR `0x1F00` before execution, the
instruction returned with `0x1F01` rather than delivering vector 19. Diagnostic
stability on the earlier image passed 10/10, 25/25, and 100/100 but omitted
`#XF` and did not satisfy strict stability. Bochs 3.1 faulted inside OVMF
before reaching ArenaOS. These results remain preserved as historical evidence
in the qualification record and receipts; they are not the final pinned-backend
result.

If future evidence invalidates state ownership, exception containment, the
integer-only audit, or a required gate, suspend qualification and revise this
accepted decision through a new ADR. Do not ship a build that partially enables
the facilities.
