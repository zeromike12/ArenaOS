# Arena 2 C runtime handoff

Phase 15 qualification is accepted by M and C for the pinned locally modified
QEMU 11.1.2 TCG backend. The native x87/SSE-family profile preserves each
scheduler thread's private FXSAVE64 image across yield, blocking/wake, timer
preemption, and address-space switching. See `QUALIFICATION.md` and
[the scoped acceptance record](ACCEPTANCE.md). M accepted ADR-0113 on
2026-10-24.
This acceptance does not qualify Arena 2 libc/runtime integration, application
packages, upstream QEMU, or hardware execution.

## C compiler profile

GCC and Clang should target a baseline x86-64 CPU, for example:

```text
-march=x86-64 -msse2 -mno-avx -mno-avx2 -mno-avx512f
```

Use CPUID before selecting optional SSE3, SSSE3, SSE4.x, or SSE4a code. Avoid
`-march=native` for portable application output. The kernel still rejects
compiler-generated SIMD outside reviewed state-management leaves; this
restriction applies to the kernel build, not native user applications.

## Scope and remaining runtime work

This CPU contract does not implement or alter Arena 2's libc subset, allocator,
TLS model, C thread API, C runtime startup, formatting, or math libraries. It
does not establish Linux syscall or ELF loader compatibility. GCC/Clang
applications still need ArenaOS's native startup ABI, package verification,
capability setup, and syscall wrappers.

Delivered unmasked x87 exceptions and unsupported instructions in ring 3 use
the existing user-thread fault path, with status `0x100 + vector`; guest tests
verified x87 `#MF`, `#UD`, and genuine SIMD `#XF` vector-19 delivery on the
pinned locally modified QEMU TCG backend. This does not claim upstream or
hardware behavior. CPL0 exceptions remain fail-stop. AVX-family state is
unsupported, so binaries that require AVX are not compatible with this profile.

## Qualification status and constraints

- **SOURCE-REVIEWED** — the existing CPL3 invalid-SSE fixture and the
  scheduler-state lifecycle are included in the external test kit.
- **HOST-TESTED** — strict EFI/ESP and fixture hashes are pinned and verified.
- **LOCALLY-MODIFIED-TCG-GUEST-QUALIFIED** — the strict guest passed on both
  CPU profiles and strict T2/T3/T4 passed 10/10, 25/25, and 100/100.
- **HARDWARE-BACKED-GUEST-TESTED** — not claimed; KVM/WHPX was not used.
- **NOT RUN** — Arena 2 C runtime/libc integration or upstream/hardware runs.

Keep Arena 2 code at the baseline x86-64 profile with x87/SSE2 and without AVX
or AVX-dependent libraries. The external runner requires AVX exposure only
for its existing `max` CPU-profile check; that is not an ArenaOS AVX support
promise.
