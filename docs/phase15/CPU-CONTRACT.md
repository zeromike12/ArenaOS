# Phase 15 CPU contract

Status: the CPU policy and state-management code are implemented on
`arena/phase15-fpu-simd`; ADR-0113 remains Proposed. The overall phase is
**PARTIAL / UNQUALIFIED** because unmasked SIMD `#XF` delivery has not passed
the strict guest gate.

## Required processor profile

Kernel startup requires CPUID x87, FXSR, SSE, and SSE2. It reports MMX and
additional SSE-family flags only when CPUID advertises them. The FXSAVE64
format preserves x87 control/status/registers, legacy MMX-aliased state,
MXCSR, and XMM0–XMM15. Optional SSE-family instructions remain CPUID-gated;
SSE3, SSSE3, SSE4.x, and SSE4a are not unconditional guarantees.

AVX/VEX, YMM upper halves, AVX2, AVX-512, ZMM, and AMX are unsupported. If
firmware leaves CR4.OSXSAVE enabled, startup checks CPUID leaf `0xD`, reduces
XCR0 to x87+SSE (`0x3`), verifies the readback, clears OSXSAVE, and confirms
that CPUID no longer advertises enabled OSXSAVE. AVX execution remains
disabled. Ring-3 unsupported instructions use the existing contained fault
path; kernel faults remain fatal.

## Explicit initialization

Before scheduler threads or user code:

- CPUID checks the mandatory features; missing features cause startup refusal.
- CPUID.OSXSAVE must agree with inherited CR4.OSXSAVE. An enabled XCR0 is
  validated and reduced to `0x3` before OSXSAVE is cleared.
- CR0.MP and CR0.NE are set; CR0.EM and CR0.TS are cleared.
- CR4.OSFXSR and CR4.OSXMMEXCPT are set; CR4.OSXSAVE is cleared.
- CR0 and CR4 are read back and checked against policy.
- x87 is initialized; MXCSR is set to `0x1F80 & MXCSR_MASK`, with `0xFFBF`
  used only when FXSAVE reports a zero mask. XMM0–XMM15 are cleared.
- The aligned clean image is validated for FCW `0x037F`, no pending x87
  status, empty tags, permitted MXCSR bits, and zero vector contents.

Host-policy tests cover feature inconsistencies, XCR0 sanitation, MXCSR
validation, and refusal paths. The OSXSAVE-seed guest receipt adds hardware
state coverage: the diagnostic entered with OSXSAVE enabled and XCR0 `0x7`,
then observed XCR0 `0x3` and OSXSAVE disabled. See
`receipts/phase15-targeted-results.json` for the exact image and run receipt.

## Exceptions and current limitation

The live-IDT audit checks selector, gate attributes, IST, and handler offset
for all vectors. Vectors 16 (`#MF`) and 19 (`#XF`) use no-error-code stubs;
the common frame adds the synthetic zero error code. CPL3 `#MF` and `#UD`
tests pass with sibling and separate-process survival. The strict unmasked
`sqrtss xmm0,xmm0` `#XF` test remains failed: QEMU 11.0.2 TCG changes MXCSR
from `0x1F00` to `0x1F01` without delivering vector 19. Matching TCG source
inspection finds no SSE vector-19 raise path. The host `SIGFPE` probe is
corroborating evidence only and does not satisfy the guest gate.

The historical qualification diagnostic intentionally omits #XF and prints
`m15: DIAGNOSTIC PASS (5/5; XF NOT RUN)`; it is not the official M15 PASS
marker. Diagnostic T2 10/10, T3 25/25, and T4 100/100 used that image; none
qualifies the official gate. The earlier QEMU host advertised KVM, MSHV, and
Nitro, but explicit startup probes failed because that host lacked their
device backends.
Official T2/T3/T4 and release qualification remain blocked until a suitable
backend delivers and contains `#XF`.

## Current qualification labels

- **SOURCE-REVIEWED** — x87/SSE/SSE2 state policy, the CPL3 `SQRTSS` probe,
  vector-19 dispatch, and cleanup assertions are preserved in the external kit.
- **HOST-TESTED** — pinned strict EFI/ESP/fixture identity and the ESP's
  embedded EFI are checked before any guest boot.
- **TCG-GUEST-TESTED** — strict `#XF` remains a TCG failure; the prior
  diagnostic x87/SSE/SSE2, MMX, `#MF`, and `#UD` results remain diagnostic.
- **HARDWARE-BACKED-GUEST-TESTED** — **BLOCKED** until Linux/KVM or Windows/WHPX
  completes the genuine vector-19 guest test.
- **NOT RUN** — official strict T2/T3/T4 and final-source aggregate regression.

The external runners use the existing two M15 CPU profiles. Their successful
receipt must show `fault=0x113`, the ring-3 vector-19 dispatch, sibling and
independent-peer survival, fault-slot release, resource-baseline restoration,
and clean UEFI shutdown. x87/SSE/SSE2 remain the ArenaOS application profile;
the AVX-capable first QEMU profile is a test requirement, while AVX execution
inside ArenaOS remains unsupported.

## Kernel restriction and memory

Ordinary kernel execution remains integer-only. Six exact reviewed
state-management leaves are the only ISA-audit exceptions. The production EFI
audit decoded 120,911 instructions with gap-free executable-section coverage
and found no ordinary FP/MMX/SSE/AVX instructions.

One exact 512-byte, 16-byte-aligned image belongs to each of the 64 scheduler
slots. The 32 KiB image table is private kernel memory and is reset from the
validated clean template on thread creation and slot reuse.
