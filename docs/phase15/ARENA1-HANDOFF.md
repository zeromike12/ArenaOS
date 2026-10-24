# Arena 1 CPU handoff

Phase 15 qualification is accepted by M and C for the pinned locally modified
QEMU 11.1.2 TCG backend. The native x86-64 contract uses eager per-thread
FXSAVE64/FXRSTOR64 and private x87, MMX, XMM0–XMM15, x87 control/status, and
MXCSR state across syscalls and scheduler switches. Arena 1's SDL3 executable
has **not** been run by this Phase-15 guest fixture, so this handoff does not
claim SDL3 compatibility. ADR-0113 remains Proposed; see
[the scoped acceptance record](ACCEPTANCE.md). No hardware-backed result or
application integration is implied.

## Supported profile

- Required CPU features: x87, FXSR, SSE, and SSE2.
- Additional SSE-family instructions are available only when that CPU reports
  them through CPUID.
- AVX/VEX, YMM upper halves, AVX2, AVX-512, ZMM, and AMX are unsupported.
  OSXSAVE is cleared; unsupported user instructions are delivered as contained
  thread faults.
- The per-thread context is qualified for the scoped x87/SSE state contract
  and tested genuine vector-19 path on the pinned modified TCG backend. Consult
  `QUALIFICATION.md` for backend limits and the untested SDL3 integration.

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
- **LOCALLY-MODIFIED-TCG-GUEST-QUALIFIED** — strict `#XF` passed on both CPU
  profiles and T2/T3/T4 passed 10/10, 25/25, and 100/100 on the pinned backend.
- **HARDWARE-BACKED-GUEST-TESTED** — not claimed; `/dev/kvm` is absent and
  Windows/WHPX was not run.
- **NOT RUN** — Arena 1 SDL3 launch and native library/application integration.

Keep distributable code at the x86-64 baseline with CPUID-gated optional SSE
features and no AVX requirement. Do not advertise Arena 1 compatibility from
the diagnostic image or from this handoff. The strict external runner's first
QEMU profile needs a host that exposes AVX, although ArenaOS itself continues
to disable AVX state.
