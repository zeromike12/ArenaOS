# ArenaOS Phase 15 plan

Phase 15 adds a bounded x87/SSE execution contract to the Phase-14 mainline at
`1aa35f4c20c9bb386612298fe21574897cf14159`. Work is on
`arena/phase15-fpu-simd`. ADR-0112 is proposed pending architecture review;
implementation on this branch is experimental.

## Architecture decision

The proposal is eager per-thread FXSAVE64/FXRSTOR64. The scheduler has a fixed
64-slot table and one switch leaf shared by yield, blocking, exit, and timer
preemption. The per-slot image is 512 bytes aligned to 16 bytes (32 KiB total).
CPUID x87/FXSAVE/SSE/SSE2 are required. AVX-family state is not enabled.
ADR-0111 is reserved by the C runtime proposal on its development branch, so
this ADR is numbered 0112.

## Work sequence

1. **Audit and ADR:** verify CPU setup, interrupt/syscall paths, scheduler slot
   lifecycle, faults, Phase-14 initialization, and the existing ISA audit.
2. **CPU contract:** CPUID feature decoding; inherited OSXSAVE/XCR0 audit;
   checked CR0/CR4 configuration; clean x87/MXCSR/XMM template; refusal on
   unsupported or failed setup.
3. **Thread state:** aligned state storage, clean initialization before Ready,
   FXSAVE64/FXRSTOR64 in the common switch, and reset during reaping/reuse.
4. **Guest fixtures:** arithmetic, control-state, syscall, thread, blocking,
   timer-preemption, process-isolation, fresh-state, and fault cases using the
   current ET_EXEC/ET_DYN loader rules.
5. **Kernel ISA audit:** parse actual disassembly instructions, enforce the
   state-management allowlist, fail closed if disassembly is absent, and test
   positive and negative controls.
6. **Regression and evidence:** preserve historical suites and Phase 14, run
   T1/T2/T3/T4, package exact artifacts/receipts, extract and independently
   verify the checkpoint archive, then commit the final coherent changes.

## Qualification gates

| Gate | Required evidence | Status |
|---|---|---|
| T1 | Build, formatting, host tests, feature/template tests, instruction audit and ABI checks | Pass: canonical build and Phase-14 hash/signature checks; rustfmt; kernel host tests 8/8; 120,354-instruction full-section audit with negative controls |
| Targeted guest | Ring-3 state, process isolation, reuse, and exception cases on two CPU profiles | Partial: thread/process isolation, slot reuse, `#MF`, and `#UD` pass; the isolated `#XF` probe records `pre=0x1F00`, `post=0x1F01`, then falls through on both QEMU TCG profiles |
| T2 | 10/10 fresh QEMU boots | Not run; targeted guest suite fails on `#XF` |
| T3 | 25/25 fresh mixed-workload boots | Not run; targeted guest suite fails on `#XF` |
| T4 | 100/100 fresh boots of one exact release candidate, zero failures | Not run |
| Archive | Exact EFI/boot image/fixtures, source and toolchain identity, SHA-256 manifest, receipts, clean extraction and boot | Not run; no qualified candidate exists |

Any failed boot invalidates the current run of that gate. The guest failure is
preserved as a failure; it is not counted toward a pass total. The standalone
guest instruction sequence and matching QEMU 11.0.2 TCG source show that the
available emulator records the MXCSR flag but does not raise vector 19. The
native host probe corroborates hardware behavior but is not ArenaOS evidence.
The PIE hash blocker is resolved by using the official Rust 1.97.0 target
components used by Phase 14; the checked-in guard remains unchanged. See
`QUALIFICATION.md` and `receipts/phase15-targeted-results.json` for exact
source, artifact hashes, commands, and receipts. The two targeted guest runs
were one attempt per profile with no retries; both shut down cleanly but
returned semantic harness failure because the `#XF` assertion failed.

## Scope

Keep Phase-14 ET_EXEC and bounded ET_DYN support, signed APB1, W^X, capability
ownership, ABI v2, FS.base, GS/swapgs, SMEP/SMAP, scheduler fairness, and
resource reclamation. Do not add a dynamic linker, Linux ABI, libc/libm, SDL3,
GPU support, SMP, AVX/AVX-512, or general compiler SIMD in the kernel.
