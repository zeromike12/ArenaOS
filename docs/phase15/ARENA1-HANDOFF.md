# Arena 1 CPU handoff

Phase 15 proposes a native x86-64 execution profile with eager per-thread
FXSAVE64/FXRSTOR64. The user thread owns private x87, MMX, XMM0–XMM15, x87
control/status, and MXCSR state across direct syscalls, yield, blocking/wake,
and scheduler switches. Arena 1's SDL3 executable has **not** been run by this
Phase-15 guest fixture, so this handoff does not claim SDL3 compatibility.
ADR-0113 remains proposed and Phase 15 is **PARTIAL / UNQUALIFIED**; these CPU
capabilities are not release guarantees. Historical diagnostic T2/T3/T4 and
MMX/OSXSAVE coverage pass, but strict unmasked SIMD `#XF` delivery still fails
under the available QEMU TCG backend. T4 was 100/100 diagnostic boots; its
official gate remains blocked.

## Supported profile

- Required CPU features: x87, FXSR, SSE, and SSE2.
- Additional SSE-family instructions are available only when that CPU reports
  them through CPUID.
- AVX/VEX, YMM upper halves, AVX2, AVX-512, ZMM, and AMX are unsupported.
  OSXSAVE is cleared; unsupported user instructions are delivered as contained
  thread faults.
- The proposed per-thread context is tested for x87, scalar SSE/SSE2, MXCSR,
  all sixteen XMM registers, MMX, yield, blocking join, timer preemption,
  separate address spaces, fresh slots, and OSXSAVE/XCR0 sanitation. Consult
  `QUALIFICATION.md` for the strict `#XF` blocker and diagnostic-only status.

## Compiler and runtime guidance

Use an x86-64 baseline target and let CPUID gate any optional SSE-family path.
For GCC or Clang, an explicit baseline such as
`-march=x86-64 -msse2 -mno-avx -mno-avx2 -mno-avx512f` avoids emitting vector
state ArenaOS does not preserve. Do not use `-march=native` for distributable
applications. The target's native startup ABI, syscall numbers, capability
model, and package format remain ArenaOS-specific; these compiler flags do not
provide a Linux ABI or desktop services.

Phase 15 adds no SDL3 integration and no libc/libm, graphics, threading API, or
application package changes. Arena 1 must still validate its executable's
startup/runtime dependencies and run the actual application under the
Phase-15 qualification image before declaring it compatible.

## Qualification status and constraints

- **SOURCE-REVIEWED** — the proposed x86-64 x87/SSE2 contract and per-thread
  state ownership are documented; no SDL3 application source was changed.
- **HOST-TESTED** — strict image identity is pinned in the external test kit.
- **TCG-GUEST-TESTED** — diagnostic FP/SIMD coverage passes, while strict
  unmasked `#XF` fails under the previously tested TCG backend.
- **HARDWARE-BACKED-GUEST-TESTED** — **BLOCKED**; no KVM/WHPX trial is available.
- **NOT RUN** — Arena 1 SDL3 launch, native library integration, the final
  120/120 regression, and official strict T2/T3/T4.

Keep distributable code at the x86-64 baseline with CPUID-gated optional SSE
features and no AVX requirement. Do not advertise Arena 1 compatibility from
the diagnostic image or from this handoff. The strict external runner's first
QEMU profile needs a host that exposes AVX, although ArenaOS itself continues
to disable AVX state.
