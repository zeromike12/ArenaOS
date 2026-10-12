# ADR-0113: Native x87/SSE execution with eager per-thread state

Status: **Proposed** — implementation on `arena/phase15-fpu-simd` is
experimental until the architecture-review process accepts this ADR.
Date: 2026-10-11
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

This is a proposed decision, not an accepted production architecture.

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

Current branch evidence isolates an unmasked `sqrtss` invalid operation with
MXCSR `0x1F00` immediately before execution. QEMU 11.0.2 TCG returns with
MXCSR `0x1F01` without delivering vector 19 on both tested CPU profiles;
matching TCG source lacks the SSE mask-check/vector-19 raise path. The guest
test remains failed, with sibling/peer and resource-cleanup receipts recorded.
This observation does not accept the ADR or establish production `#XF`
containment. T2/T3/T4 diagnostic runs reached 10/10, 25/25, and 100/100 on the
diagnostic image, which explicitly does not run `#XF`; the official stability
gates remain open. The 120-group historical diagnostic runner had five first-
run failures (115/120 first-run pass); each affected suite passed its focused
recovery rerun, but no clean 120/120 aggregate rerun was completed. The
Phase-14 PIE hash mismatch was independently resolved by restoring its
official Rust 1.97.0 target libraries; the existing hash guard and signed
package identity remain intact. See `docs/phase15/QUALIFICATION.md` for exact
identities and evidence classes.

If state ownership, exception containment, the integer-only audit, or a
required guest gate cannot be proven, do not qualify this phase. The rollback
is to revert Phase-15 commits and restore the accepted ADR-0012 no-FP execution
contract; do not ship a build that partially enables the facilities.
