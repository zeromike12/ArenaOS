# ArenaOS Phase 15 plan and status

Phase 15 adds a bounded x87/SSE execution contract to the Phase-14 mainline at
`1aa35f4c20c9bb386612298fe21574897cf14159`. Work is on
`arena/phase15-fpu-simd`. The core implementation is in
`ad87bee8d199b285ca1a2f3a31c5dbdcf4994faa`; the Phase-15 diagnostic fixtures
and kernel audit were extended at
`6586a0a18981932ae7048ca267fa425411f8e463`. ADR-0113 remains Proposed.

## Architecture decision

The proposal uses eager per-thread FXSAVE64/FXRSTOR64. The scheduler has a
fixed 64-slot table and one switch leaf shared by yield, blocking, exit, and
timer preemption. Each private image is 512 bytes and 16-byte aligned (32 KiB
total). CPUID x87/FXSAVE/SSE/SSE2 are required. AVX-family state is not enabled.
ADR-0113 avoids the active C-runtime reservation; ADR-0112 and accepted ADRs
were not renumbered. The proposal remains Proposed pending architecture review
and qualification.

## Completed work

1. Reviewed CPU setup, interrupt/syscall paths, scheduler slot lifecycle,
   fault teardown, Phase-14 compatibility, and the whole-image ISA audit. The
   audit found no correctness defect requiring an architecture change.
2. Verified CR0/CR4 policy, feature checks, clean FXSAVE template, and the
   inherited OSXSAVE/XCR0 sanitation path. Added a guest entry seeded with
   OSXSAVE enabled and XCR0 `0x7`; it read back XCR0 `0x3` and OSXSAVE disabled.
3. Preserved the strict unmasked `sqrtss` vector-19 acceptance test. QEMU
   11.0.2 TCG updates MXCSR from `0x1F00` to `0x1F01` and does not deliver
   `#XF`. KVM, MSHV, and Nitro acceleration were explicitly unavailable on the
   host. No hardware-backed or independent-emulator guest result exists.
4. Ran all 120 groups in the Phase-14 historical diagnostic runner. The first
   aggregate run passed 115/120; five affected groups were then rerun alone and
   all five passed. There is no clean one-shot 120/120 run, so the aggregate is
   recorded as recovered with a qualification limitation.
5. Added a bounded MMX test across timer preemption and `EMMS`; optional
   SSE-family reporting remains CPUID-gated. Both guest checks pass in
   diagnostic mode.
6. Ran T2 10/10, T3 25/25, and T4 100/100 on the same immutable diagnostic
   EFI/ESP. These results do not satisfy the official Phase-15 gates because
   the image says `XF NOT RUN`.
7. Built the self-contained diagnostic checkpoint and ran its verifier from a
   clean `/tmp` extraction, without a repository checkout or external QEMU.

## Qualification gates

| Gate | Evidence | Result |
|---|---|---|
| T1 | Canonical build, protected Phase-14 PIE and signed APB1, host tests, rustfmt, complete executable-section ISA audit | Pass |
| Targeted guest | x87/SSE/XMM isolation, process and slot reuse, MMX, `#MF`, `#UD`; genuine `#XF` separately tested on two TCG profiles | Partial; `#XF` failed |
| Historical Phase-14 suite | 120 groups executed; 115 passed in first aggregate run; all five first-run failures passed focused recovery reruns | Recovered evidence; no single clean 120/120 aggregate |
| OSXSAVE | Seeded guest entered with OSXSAVE=1/XCR0=7 and verified sanitation to XCR0=3/OSXSAVE=0 | Diagnostic guest pass |
| T2 | 10 fresh boots on diagnostic EFI/ESP | Diagnostic pass 10/10; official gate blocked |
| T3 | 25 fresh boots with desktop, services, lifecycle, FP/MMX test coverage | Diagnostic pass 25/25; official gate blocked |
| T4 | 100 fresh boots of one immutable diagnostic image | Diagnostic pass 100/100 in 1,326 seconds; official gate blocked |
| Archive | Exact images, guest fixtures, source, QEMU/OVMF runtime, receipts, extracted-only boot and PIE verifier | Diagnostic checkpoint verified; not a release candidate |

The acceptance gate remains **PARTIAL / UNQUALIFIED** until genuine vector 19
delivery and containment pass under a supported backend. Diagnostic results do
not become official qualification. No Phase 16 work, merge, pull request, or
release is part of this plan. See [qualification](QUALIFICATION.md) and the
[final report](FINAL-REPORT.md) for exact receipts and current gate status.
