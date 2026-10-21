# Arena 2 C runtime handoff

Phase 15 proposes enabling native x87 and SSE-family execution with eager
per-thread state preservation. Each scheduler thread receives a clean,
private FXSAVE64 image; the common switch saves/restores x87 state, XMM0–XMM15,
and MXCSR during yield, blocking/wake, timer preemption, and address-space
switching. New and recycled slots receive the clean template. See
`QUALIFICATION.md` for which behaviors have actually passed guest gates.
ADR-0113 remains proposed and Phase 15 is **PARTIAL / UNQUALIFIED**, so this is
not yet a release-guaranteed CPU contract. The uninterrupted historical
diagnostic aggregate passed 120/120. Fresh diagnostic T2/T3/T4 passed 10/10,
25/25, and 100/100 on QEMU 11.1.2 TCG. Strict `#XF` delivery still fails on
both tested CPU profiles; `/dev/kvm` is absent and Bochs did not reach ArenaOS,
so the official stability gates remain blocked.

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
verified x87 `#MF` and unsupported-instruction `#UD`. SIMD `#XF` delivery
remains unqualified because QEMU TCG sets the MXCSR status flag but does not
deliver vector 19. CPL0 exceptions remain fail-stop. AVX-family state is
unsupported, so binaries that require AVX are not compatible with this profile.

## Qualification status and constraints

- **SOURCE-REVIEWED** — the existing CPL3 invalid-SSE fixture and the
  scheduler-state lifecycle are included in the external test kit.
- **HOST-TESTED** — strict EFI/ESP and fixture hashes are pinned and verified.
- **TCG-GUEST-TESTED** — the historical diagnostic aggregate passes 120/120,
  diagnostic T2/T3/T4 pass, and x87/SSE/SSE2 state, MMX, `#MF`, and `#UD`
  cases pass. The genuine strict `#XF` case remains a TCG failure.
- **HARDWARE-BACKED-GUEST-TESTED** — **BLOCKED** pending a passing KVM or WHPX
  trial with complete contained-fault receipts.
- **NOT RUN** — Arena 2 C runtime/libc integration and official strict T2/T3/T4.

Keep Arena 2 code at the baseline x86-64 profile with x87/SSE2 and without AVX
or AVX-dependent libraries. The external runner requires AVX exposure only
for its existing `max` CPU-profile check; that is not an ArenaOS AVX support
promise.
