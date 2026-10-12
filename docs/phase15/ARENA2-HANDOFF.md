# Arena 2 C runtime handoff

Phase 15 proposes enabling native x87 and SSE-family execution with eager
per-thread state preservation. Each scheduler thread receives a clean,
private FXSAVE64 image; the common switch saves/restores x87 state, XMM0–XMM15,
and MXCSR during yield, blocking/wake, timer preemption, and address-space
switching. New and recycled slots receive the clean template. See
`QUALIFICATION.md` for which behaviors have actually passed guest gates.
ADR-0113 remains proposed and Phase 15 is **PARTIAL / UNQUALIFIED**, so this is
not yet a release-guaranteed CPU contract. Historical diagnostic T2/T3, MMX,
and seeded OSXSAVE tests pass; strict `#XF` guest delivery still fails on the
available QEMU TCG backend.

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
