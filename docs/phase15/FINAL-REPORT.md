# ArenaOS Phase 15 final report

Status: **PHASE 15 — IMPLEMENTED / HISTORICAL REGRESSIONS TESTED / EXCEPTION
QUALIFICATION BLOCKED**. The branch remains **PARTIAL / UNQUALIFIED** and is
**NOT READY FOR INTEGRATION REVIEW**. ADR-0113 is Proposed. No PR, merge,
release, or Phase 16 work was created.

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
- **Backend search:** `/dev/kvm` is absent. Explicit QEMU `-accel kvm`, `mshv`,
  and `nitro` each failed because the required host device/backend is absent.
  WHPX/HVF do not apply to this Linux host. TCG is the only available backend;
  no hardware-backed run is claimed.

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

A reproducible **diagnostic checkpoint**, not a release candidate, is prepared
with the exact diagnostic and strict images, compiled Phase-15 state/MMX/`#XF`
fixtures, signed Phase-14 PIE/APB1, source snapshot, bundled QEMU/OVMF runtime,
historical and stability receipts, manifest, and extracted-only verifier.
Independent extraction and execution of that verifier is the remaining archive
step; it cannot change the blocked `#XF` result.

Remaining qualification blockers are genuine vector-19 delivery and
containment under a trustworthy backend, a clean one-shot 120/120 historical
regression run, and official rather than diagnostic T2/T3/T4 evidence. The
implementation remains experimental. Arena 1's SDL3 application and Arena 2's
runtime were not run or modified; no SDL3 compatibility, C runtime, libc/libm,
or graphics integration is established.

See [the qualification record](QUALIFICATION.md),
[`phase15-qualification-recovery.json`](receipts/phase15-qualification-recovery.json),
and [the alternate-backend receipt](receipts/phase15-alternative-backends.json)
for exact raw and machine-readable evidence.
