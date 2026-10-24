# Phase 15 final kernel audit

Reviewed source: Phase 15 implementation commit `6586a0a18981932ae7048ca267fa425411f8e463`, based on merged Phase-14 `main` commit `1aa35f4c20c9bb386612298fe21574897cf14159`.

## Context-switch ownership

- Every scheduler slot owns one private, 512-byte, 16-byte-aligned FXSAVE64 image. The fixed 64-slot table accounts for 32 KiB plus alignment padding and is not user- or capability-visible.
- The bootstrap slot starts from the validated clean template. New threads receive the same template before becoming Ready. Process spawn and reaped-slot reuse cannot inherit the previous thread's x87, MMX, XMM, or MXCSR values.
- Cooperative yield, timer preemption, blocking/wake, process address-space changes, exit, and first entry all use the common save/restore leaf. The scheduler selects the outgoing and incoming raw state pointers with interrupts disabled. The assembly saves outgoing state and restores the incoming state before the incoming saved register frame resumes.
- CR3 can be changed before the assembly state switch because the kernel's static context table remains mapped in every process view. Interrupts are disabled across the transition, so no observer can use the intermediate mapping/state pairing.
- A zombie remains owned until execution has left its stack. The subsequent reap clears context and FP images before slot reuse. Process teardown, joined/detached-thread cleanup, and failed spawn paths retain that lifetime rule.

No save into a released slot, restore from an uninitialized slot, or alternate context-switch path bypassing the common leaf was found.

## CPU initialization and validation

- Startup requires CPUID x87, FXSR, SSE, and SSE2; optional SSE-family and MMX features are reported from CPUID.
- CR0.MP/NE and CR4.OSFXSR/OSXMMEXCPT are set; CR0.EM/TS and CR4.OSXSAVE are cleared and read back.
- Inherited OSXSAVE is checked for consistency with CPUID. When set, leaf `0xD` is validated, XCR0 is reduced to x87+SSE (`0x3`), read back, and then OSXSAVE is cleared and confirmed disabled.
- The MXCSR mask comes from FXSAVE, with the architectural fallback `0xFFBF` only when the reported mask is zero. Template and input values are validated before use; unsupported high MXCSR bits are refused.
- The clean template is produced internally after x87 initialization, MXCSR setup, and XMM clearing. It is validated for FCW `0x037F`, no pending x87 status, empty tags, permitted MXCSR bits, and zero XMM contents.

The OSXSAVE-seed diagnostic boot supplied guest evidence for the inherited path: it entered with `CR4.OSXSAVE=1`, `XCR0=0x7`, and CPUID.XSAVE/AVX set; initialization observed `XCR0=0x3`, cleared OSXSAVE, and reported AVX state unsupported. This is diagnostic coverage, not the strict `#XF` qualification.

## Exceptions and instruction audit

Vectors 16 (`#MF`) and 19 (`#XF`) use no-error-code stubs. The common exception frame adds the synthetic zero error code expected by the handler. CPL3 user faults terminate the faulting thread through the established scheduler teardown; CPL0 faults remain fatal. The guest verified CPL3 `#MF` and `#UD` containment, sibling/process survival, and exact resource return. The #XF gate is wired but remains unverified because the available QEMU TCG does not deliver vector 19 for the unmasked SSE instruction.

The production EFI audit decoded 120,911 instructions across complete executable sections with gap-free coverage. It matched six exact reviewed state-management leaves and found no ordinary kernel x87/MMX/SSE/AVX use. Empty-disassembly and prohibited-instruction controls passed.

## Review conclusion

The source audit found no scheduler, context-switch, slot-reuse, CPU-policy, or exception-frame correctness defect requiring a redesign. The implementation adds bounded per-slot state and initialization around the existing shared switch path. Qualification work added MMX, inherited-OSXSAVE, and diagnostic regression coverage. The only open architectural acceptance gate is successful guest delivery and containment of unmasked SIMD `#XF`; no production or release claim should pass that gate based on a TCG source inspection or host `SIGFPE` result.
