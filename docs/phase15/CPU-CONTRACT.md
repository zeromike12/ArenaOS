# Phase 15 CPU contract

Status: proposed with ADR-0112; CPU setup passed in targeted QEMU boots, while
the full Phase-15 qualification remains incomplete.

## Required processor profile

Kernel startup requires CPUID.1:EDX.FPU, FXSR, SSE, and SSE2. It records
additional SSE-family flags only when CPUID reports them. x87, MMX register
state, XMM0–XMM15, x87 control/status, and MXCSR are covered by the 64-bit
FXSAVE format. Applications must use CPUID to select optional SSE-family
instructions.

AVX, AVX2, AVX-512, YMM upper halves, ZMM, and AMX are unsupported. If firmware
leaves CR4.OSXSAVE enabled, startup verifies CPUID leaf 0xD, restricts XCR0 to
the legacy x87+SSE components (`0x3`), reads it back, and then clears
CR4.OSXSAVE. If OSXSAVE was already clear, XCR0 is not accessible and AVX
execution is disabled by the CR4 policy. The kernel does not expose OSXSAVE
support. An unsupported instruction in ring 3 is a contained process-thread
fault.

## Explicit initialization

Before scheduler threads or user code:

- CPUID checks the mandatory features; absence is a diagnostic startup refusal.
- CPUID.OSXSAVE must agree with the inherited CR4.OSXSAVE bit. When enabled,
  XCR0 must support x87 and SSE; it is reduced to `0x3` and verified before
  OSXSAVE is cleared.
- CR0.MP and CR0.NE are set; CR0.EM and CR0.TS are cleared.
- CR4.OSFXSR and CR4.OSXMMEXCPT are set; CR4.OSXSAVE is cleared.
- CR0 and CR4 are read back and compared with the policy.
- x87 is initialized; MXCSR is set to `0x1F80 & MXCSR_MASK`, using `0xFFBF`
  when FXSAVE reports a zero mask; XMM0–XMM15 are cleared.
- An aligned FXSAVE64 image is validated for FCW `0x037F`, no pending status,
  empty x87 tags, valid MXCSR/mask, and zero vector contents.

No state is considered initialized from a successful compile or from inherited
firmware settings.

Targeted boot evidence on QEMU 11.0.2 covered both `qemu64` and `max`. Both
reported firmware OSXSAVE clear, so startup verified the non-OSXSAVE path and
logged `XCR0=None`; the inherited-OSXSAVE/XCR0 reduction path has host policy
tests but no guest evidence yet. The tested image read back CR0 `0x80010033`
and CR4 `0x668` (OSFXSR and OSXMMEXCPT set, OSXSAVE clear). `max` advertised
AVX, while OSXSAVE remained clear after initialization.

## Kernel restriction and exceptions

Ordinary kernel execution remains integer-only. The only approved FP/SIMD or
xstate instructions are the explicit initialization, FXSAVE/FXRSTOR, and
conditional XCR0-sanitization leaves listed by the architecture audit. A
delivered ring-3 `#NM`, `#MF`, `#XF`, or `#UD` is handled through the existing
process-thread teardown path. Guest tests verified `#MF` and `#UD`. The `#XF`
path is wired to the same containment mechanism, but its guest assertion is
currently unverified because the tested QEMU TCG does not deliver that fault.
The same vectors at CPL0 remain diagnostic fail-stop faults.

## Memory

One exact 512-byte, 16-byte-aligned image is owned by each scheduler slot. The
current `MAX_THREADS=64` bound adds 32 KiB plus alignment padding in kernel-only
memory. It is not capability-visible and is reset from the clean template on
thread creation and slot reuse.
