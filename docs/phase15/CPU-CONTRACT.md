# Phase 15 CPU contract

Status: Phase 15 is **qualified and accepted by M and C** for the pinned,
locally modified QEMU 11.1.2 TCG backend. M accepted ADR-0113 on 2026-10-24.
The accepted profile is the existing x87/MMX/SSE/SSE2
contract; it does not establish upstream-QEMU, hardware, SDL3, or C-runtime
compatibility. See [the acceptance record](ACCEPTANCE.md).

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
tests pass with sibling and separate-process survival. The unmodified QEMU TCG runs failed the strict unmasked
`sqrtss xmm0,xmm0` probe. The unchanged guest passed on the pinned locally
modified QEMU 11.1.2 TCG backend, delivering vector 19 on both `max` and
`qemu64` profiles with expected fault status `0x113`. C independently replayed
both profiles; see [the acceptance record](ACCEPTANCE.md). The host `SIGFPE`
probe is corroborating evidence only and is not part of the accepted guest gate.

The historical 120/120 diagnostic aggregate remains distinct from strict
qualification; its image prints `m15: DIAGNOSTIC PASS (5/5; XF NOT RUN)`. The
strict image separately passed the official T2 10/10, T3 25/25, and T4 100/100
gates on pinned modified TCG. This host has no `/dev/kvm`; hardware acceleration
was not used. Bochs did not reach ArenaOS. No hardware-backed result is claimed.

## Current qualification labels

- **SOURCE-REVIEWED** — x87/SSE/SSE2 state policy, the CPL3 `SQRTSS` probe,
  vector-19 dispatch, and cleanup assertions are preserved in the external kit.
- **HOST-TESTED** — pinned strict EFI/ESP/fixture identity and the ESP's
  embedded EFI are checked before any guest boot.
- **LOCALLY-MODIFIED-TCG-GUEST-QUALIFIED** — strict 6/6 passed on both CPU
  profiles; official strict T2/T3/T4 passed 10/10, 25/25, and 100/100; M and C
  accepted the exact pinned backend after independent review.
- **HARDWARE-BACKED-GUEST-TESTED** — not claimed; KVM/WHPX guest execution was
  not part of this accepted backend result.
- **NOT RUN** — Windows/WHPX and broad SIMD compatibility such as SDL3 workloads.

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
