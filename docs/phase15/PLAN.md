# ArenaOS Phase 15 plan and status

## Current qualification status (2026-10-24)

Phase 15 qualification is **ACCEPTED by M and C** for the exact pinned, locally
modified QEMU 11.1.2 TCG backend. See [the acceptance record](ACCEPTANCE.md) and
[current qualification report](QUALIFICATION.md). ADR-0113 remains Proposed;
this does not imply upstream-QEMU, hardware, Windows, SDL3, or C-runtime
qualification. The implementation plan below records the earlier recovery
baseline and is retained as historical context.

## Historical implementation plan and gates

Phase 15 adds a bounded x87/SSE execution contract to the Phase-14 mainline at
`1aa35f4c20c9bb386612298fe21574897cf14159`. Work is on
`arena/phase15-fpu-simd`; source tested in the latest local recovery is
`989646771a0e288cefdaf0dabc5f5baa9bf1ed5c`. ADR-0113 remains Proposed.

## Architecture decision

The proposal uses eager per-thread FXSAVE64/FXRSTOR64. The scheduler has a
fixed 64-slot table and one switch leaf shared by yield, blocking, exit, and
timer preemption. Each private image is 512 bytes and 16-byte aligned (32 KiB
total). CPUID x87/FXSAVE/SSE/SSE2 are required. AVX-family state is not enabled.
ADR-0113 avoids the active C-runtime reservation; ADR-0112 and accepted ADRs
were not renumbered. The proposal remains Proposed pending architecture review
and qualification.

## Completed work and latest local evidence

1. Reviewed CPU setup, interrupt/syscall paths, scheduler slot lifecycle,
   fault teardown, Phase-14 compatibility, and the whole-image ISA audit. The
   audit found no correctness defect requiring an architecture change.
2. Verified CR0/CR4 policy, feature checks, clean FXSAVE template, and the
   inherited OSXSAVE/XCR0 sanitation path. The seeded diagnostic entered with
   XCR0 `0x7` and verified XCR0 `0x3` with OSXSAVE disabled.
3. Restored official QEMU 11.1.2, OVMF 2026.08, and Rust 1.97 locally under
   `build/phase15-local-qualification/`; also recovered QEMU 10.0.13 for a
   comparison run. No host system packages or security settings changed.
4. Ran the historical diagnostic runner once, uninterrupted: **120/120
   suites passed** on commit `989646771a0e288cefdaf0dabc5f5baa9bf1ed5c`.
   The strict `#XF` acceptance test remains a separate gate.
5. Re-ran strict `#XF` on QEMU 11.1.2 TCG with both `max` and `qemu64` CPU
   profiles. `SQRTSS` sets the invalid-operation flag but QEMU returns instead
   of delivering vector 19; the guest test fails as designed.
6. Explicit KVM initialization failed because `/dev/kvm` and the host KVM
   module are unavailable. Bochs 3.1 was tried with two CPU profiles; OVMF
   page-faulted before ArenaOS, so it yielded no exception-delivery result.
7. Ran fresh diagnostic stability loops on one immutable EFI/ESP: T2 10/10 in
   95 seconds, T3 25/25 in 237 seconds, and T4 100/100 in 952 seconds. The
   image explicitly skips `#XF`; these results are not official Phase 15 gates.
8. Preserved the aggregate, per-test logs, strict serial/stderr, image pairs,
   per-boot stability receipts, KVM probe, and Bochs logs in the recovery
   archive linked below.
9. Reconciled the external strict kit to the recovery ESP, bundled the exact
   strict pair, CPL3 fixture, and official OVMF code/vars, and verified a
   fresh extraction with five host-only negative controls. The current host
   has no QEMU or `/dev/kvm`; its external-runner preflight is BLOCKED and
   starts no guest. See [the downloadable kit handoff](EXTERNAL-KIT.md).

## Qualification gates

| Gate | Evidence | Result |
|---|---|---|
| T1 | Canonical build, protected Phase-14 PIE and signed APB1, host tests, rustfmt, complete executable-section ISA audit | Pass |
| Targeted guest | x87/SSE/XMM isolation, process and slot reuse, MMX, `#MF`, `#UD`; strict `#XF` tested separately on two TCG profiles | Partial; `#XF` failed |
| Historical Phase-14 suite | One uninterrupted 120-suite diagnostic run | Pass 120/120 |
| OSXSAVE | Seeded guest entered with OSXSAVE=1/XCR0=7 and verified sanitation to XCR0=3/OSXSAVE=0 | Diagnostic guest pass |
| T2 | 10 fresh boots on the same diagnostic EFI/ESP | Diagnostic pass 10/10; official gate blocked |
| T3 | 25 fresh boots with desktop, services, lifecycle, and network checks | Diagnostic pass 25/25; official gate blocked |
| T4 | 100 fresh boots of the same diagnostic image | Diagnostic pass 100/100 in 952 seconds; official gate blocked |
| Archive | Raw current-run receipts, exact images, probes, firmware identity, and checksums | Verified diagnostic evidence; not a release candidate |
| Strict hardware-backed `#XF` | No KVM device; TCG fails; Bochs did not reach ArenaOS | Blocked; external capable host required |

The acceptance gate remains **PARTIAL / UNQUALIFIED** until genuine vector 19
delivery and containment pass under a supported backend. Diagnostic stability
does not qualify Phase 15. No Phase 16 work, merge, pull request, or release is
part of this plan. See [qualification](QUALIFICATION.md) and the
[final report](FINAL-REPORT.md) for exact receipts and current gate status.
