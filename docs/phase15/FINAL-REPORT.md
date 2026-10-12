# ArenaOS Phase 15 final report

Status: **PHASE 15 — IMPLEMENTATION COMPLETE / EXTERNAL HARDWARE VERIFICATION
REQUIRED**. The branch remains **PARTIAL / UNQUALIFIED** and is **NOT READY FOR
INTEGRATION**. ADR-0113 is Proposed. No PR, merge, release, or Phase 16 work
was created.

## Current recovery evidence (2026-10-12)

- **SOURCE-REVIEWED** — the external kit preserves the strict test assertions,
  the CPL3 `SQRTSS` source fixture, and the vector-19 exception/cleanup source.
- **HOST-TESTED** — strict EFI/ESP and fixture hashes match their pinned
  identities; the ESP contains the exact EFI; the extracted kit manifest and
  Windows-mode TCP transport smoke pass. PowerShell is unavailable here, so
  the PowerShell runner is source-reviewed only.
- **TCG-GUEST-TESTED** — prior strict TCG trials still fail genuine `#XF`;
  historical diagnostic coverage remains diagnostic-only.
- **HARDWARE-BACKED-GUEST-TESTED** — **BLOCKED**. This workspace has no
  `/dev/kvm`, QEMU executable, OVMF firmware, or Rust toolchain. No hardware
  trial is claimed.
- **NOT RUN** — a clean final-source 120/120 aggregate, official strict T2/T3/T4,
  and a qualified release archive.

The portable runner and kit are under `tools/phase15_external/`. The kit pins
the strict EFI, ESP, and CPL3 fixture; its Linux and Windows runners require
explicit KVM or WHPX and save raw serial, QEMU stderr, actual process exits,
and a structured receipt. The kit is an execution aid, not a qualification
archive.

## Source and images

- Branch: `arena/phase15-fpu-simd`; base: merged Phase-14 mainline
  `1aa35f4c20c9bb386612298fe21574897cf14159`.
- Core kernel implementation: `ad87bee8d199b285ca1a2f3a31c5dbdcf4994faa`.
- Diagnostic fixture/image source: `6586a0a18981932ae7048ca267fa425411f8e463`.
- ADR-0113: **Proposed**, not accepted.

| Image | EFI SHA-256 | ESP SHA-256 | Use/result |
|---|---|---|---|
| Diagnostic | `e85e0b5dd9e96f3129e7f2385d548084c56c16801432a9d41243b9ebc23a2282` | `b7a50113498033dd488ccd62acfebeed96d6a483d891cffed997cb79ef09bf42` | T2/T3/T4; explicitly reports `XF NOT RUN` |
| Strict acceptance | `bbbc63df271c3d7d5109789369f72e093b02ef77c77ebe25273b6da4c00d238e` | `33dd1576ab597a7d0b3e9a9100bc618e9b251a7a645e77e79d8852a404c40a6a` | Preserves the failing genuine `#XF` test |

The Phase-14 PIE remains at SHA-256
`d7b0a8cf9c735c3898a867d824563f06b0d949df80fa1a9c96f9180a395fea2e`; the
signed APB1 remains at `04add54f09042353fc42511978c82f8b8efb87f7d5f0657f9116f456276b3dfa`.

## Results

- **Source/host:** final ownership and lifecycle audit found no correctness
  defect; kernel host tests pass 8/8; rustfmt passes; canonical build passes;
  whole executable-section ISA audit covers 120,911 instructions gap-free
  with six reviewed state-management leaves and no ordinary kernel FP/MMX/
  SSE/AVX instructions.
- **Historical regression:** the complete diagnostic runner executed all 120
  groups; its initial aggregate result was **FAIL, 115/120**. Five first-run
  failures were `test_phase14_negative_guest.py`, `test_m10_boundaries_red.py`,
  `test_m84_stage.py`, `test_m8_dependencies.py`, and `test_m8_fault.py`. Each
  affected suite passed its isolated recovery run. The M8 correction is a test
  parser fix. No clean one-shot 120/120 aggregate rerun was completed; the
  initial failures remain recorded.
- **Guest FP/SIMD:** diagnostic tests pass x87/SSE/SSE2 and XMM0–XMM15 thread
  isolation, process isolation and slot reuse, MMX state across timer/yield
  plus `EMMS`, optional CPUID feature reporting, and `#MF`/`#UD` containment.
- **OSXSAVE:** one seeded diagnostic boot entered with CR4.OSXSAVE set and
  XCR0=`0x7`, then verified XCR0=`0x3`, OSXSAVE clear, and native SSE
  execution. The ordinary firmware entry starts with OSXSAVE clear.
- **`#XF`: BLOCKED.** The strict guest executes `sqrtss xmm0,xmm0` with MXCSR
  `0x1F00`; stock QEMU 11.0.2 TCG returns with `0x1F01` and does not deliver
  vector 19 on the `max` and `qemu64` profiles. The sibling and peer survive
  and resources return to baseline, but delivery remains failed. No kernel
  workaround or synthetic `int 19` was added.
- **Earlier backend search:** that QEMU host lacked `/dev/kvm`. Explicit QEMU
  `-accel kvm`, `mshv`, and `nitro` each failed because the required host
  device/backend was absent.
  WHPX/HVF do not apply to this Linux host. TCG is the only available backend;
  no hardware-backed run is claimed.
- **Final-source aggregate rerun:** **NOT RUN** in the current workspace
  because QEMU, OVMF, `cargo`, and `rustc` are absent. The previously recorded
  115/120 aggregate and five focused recoveries remain historical evidence;
  they are not upgraded to a clean 120/120 result.

## Stability evidence

All three runs used the same immutable diagnostic image, QEMU 11.0.2 TCG, and
CPU profile `qemu64,+nx,+smep,+smap`.

| Gate | Diagnostic result | Duration | Official Phase-15 result |
|---|---:|---:|---|
| T2 | 10/10 | 134 seconds | Blocked by `#XF` |
| T3 | 25/25 | 318 seconds | Blocked by `#XF` |
| T4 | 100/100 | 1,326 seconds | Blocked by `#XF` |

All diagnostic stability boots had zero failures, timeouts, and retries. T4
retained 100 per-boot pixel/lifecycle receipts. These diagnostic runs do not
satisfy official T2/T3/T4, whose Phase-15 acceptance includes genuine `#XF`
containment.

## Archive and remaining blockers

A reproducible **diagnostic checkpoint**, not a release candidate, is available
at [phase15-diagnostic-checkpoint.tar.gz](/workspace/ArenaOS/build/phase15-diagnostic-checkpoint.tar.gz).
It is 137,994,198 bytes with SHA-256
`8fd857c9c46a6c370fc639bbacaa43c46dcba7a5fb3fe0be2ea2274d00eecc9d`. The
extracted-only verifier passed 3,335 manifest checks, exact image and fixture
hashes, a diagnostic boot with clean shutdown, and the signed Phase-14 PIE
install/launch/teardown workflow. The first extraction caught a one-character
hash pin typo before guest boot; that receipt is preserved, the pin was
corrected, the archive rebuilt, and the second extraction passed. Archive
verification does not change the blocked `#XF` result.

Remaining qualification blockers are genuine vector-19 delivery and
containment under a trustworthy backend, a clean one-shot 120/120 historical
regression run, and official rather than diagnostic T2/T3/T4 evidence. Run the
external strict test on an x86-64 KVM or WHPX host, independently verify its
receipt, then complete the full regression and official qualification gates
with the final immutable image before requesting integration review. Arena 1's
SDL3 application and Arena 2's runtime were not run or modified; no SDL3
compatibility, C runtime, libc/libm, or graphics integration is established.

See [the qualification record](QUALIFICATION.md),
[`phase15-qualification-recovery.json`](receipts/phase15-qualification-recovery.json),
[`phase15-diagnostic-checkpoint.json`](receipts/phase15-diagnostic-checkpoint.json),
and [the alternate-backend receipt](receipts/phase15-alternative-backends.json)
for exact raw and machine-readable evidence.
