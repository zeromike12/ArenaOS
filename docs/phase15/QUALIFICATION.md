# Phase 15 qualification record

Status: **PARTIAL / UNQUALIFIED**. The implementation and diagnostic guest
coverage are complete. The historical runner exercised all 120 groups but had
five failures in its first aggregate run; each failed group passed when rerun
individually. No clean one-shot 120/120 aggregate run was completed. Diagnostic
T2/T3/T4 passed on one immutable EFI/ESP, and genuine SIMD `#XF` remains blocked
because no hardware-backed or independently verified emulator backend is
available.

## Final recovery attempt (2026-10-12)

Evidence labels for this revision:

- **SOURCE-REVIEWED** — portable KVM/WHPX runners preserve the existing strict
  M15 assertions and pin the original strict EFI/ESP and CPL3 fixture.
- **HOST-TESTED** — image identity/embedded-EFI checks, kit assembly and
  extracted manifest checks, and Windows-mode TCP transport smoke pass.
  PowerShell is unavailable here; its runner is source-reviewed only.
- **TCG-GUEST-TESTED** — prior strict TCG trials fail vector 19; prior
  diagnostic coverage remains explicitly separate.
- **HARDWARE-BACKED-GUEST-TESTED** — **BLOCKED**: no `/dev/kvm`, QEMU, or OVMF
  is available in this workspace. WHPX is not applicable on this Linux host.
- **NOT RUN** — final-source aggregate 120/120, strict official T2/T3/T4, and
  final qualification archive.

The final-source aggregate was not attempted because `cargo`, `rustc`, QEMU,
and OVMF are absent. This does not replace the recorded initial 115/120 result
or its focused recoveries. See `tools/phase15_external/README.md` for exact
setup and run steps on a separate x86-64 host.

## Source and artifact identity

- Repository: `zeromike12/ArenaOS`.
- Branch: `arena/phase15-fpu-simd`.
- Base: merged Phase-14 mainline `1aa35f4c20c9bb386612298fe21574897cf14159`.
- Core implementation: `ad87bee8d199b285ca1a2f3a31c5dbdcf4994faa`.
- Image and diagnostic fixture source: `6586a0a18981932ae7048ca267fa425411f8e463`.
- M8 diagnostic parser correction: `87208dd6113e8970d3096601cde422d4fd91a104`.
- ADR: ADR-0113, **Proposed**.
- Rust: `1.97.0 (2d8144b78 2026-07-07)`; Cargo `1.97.0
  (c980f4866 2026-06-30)`; LLVM/LLD `22.1.6`.
- `rustc` SHA-256: `df13f58759c0662831983e3a6501c63c1fc12ea60ec4e1d1ac35e5fe43c500c0`.
- `rust-lld` SHA-256: `8e5130a02d8f3473ed4c4b971d43c301205c89854bb467257e64eafe4bf21ca8`.
- QEMU: `11.0.2`, TCG, binary SHA-256
  `0539587908d9a9b83ccdce7a48d968b50f827476b9852814e07445e9402df562`;
  source tag `v11.0.2`, commit
  `e545d8bb9d63e9dd61542b88463183314cff9482`.

| Image identity | EFI bytes | EFI SHA-256 | ESP bytes | ESP SHA-256 |
|---|---:|---|---:|---|
| Diagnostic image used by T2/T3/T4 | 11,028,992 | `e85e0b5dd9e96f3129e7f2385d548084c56c16801432a9d41243b9ebc23a2282` | 16,777,216 | `b7a50113498033dd488ccd62acfebeed96d6a483d891cffed997cb79ef09bf42` |
| Strict acceptance image (`#XF` enabled) | 11,038,208 | `bbbc63df271c3d7d5109789369f72e093b02ef77c77ebe25273b6da4c00d238e` | 16,777,216 | `33dd1576ab597a7d0b3e9a9100bc618e9b251a7a645e77e79d8852a404c40a6a` |

The diagnostic and strict ESPs each contain the byte-identical paired EFI.
T2/T3/T4 all used the same immutable diagnostic EFI/ESP identity above. That
image explicitly reports `m15: DIAGNOSTIC PASS (5/5; XF NOT RUN)` and is not a
strict Phase-15 acceptance image.

## Evidence classes

| Evidence label | Result |
|---|---|
| SOURCE-REVIEWED | Scheduler ownership, switch paths, thread creation/reaping, CPU policy, exception frame, integer-only kernel rule, and external runner design; see [`final-kernel-audit.md`](receipts/final-kernel-audit.md). |
| HOST-TESTED | Prior kernel host tests 8/8, formatting, policy/template checks, and full-section ISA audit passed. Current package/image hash and extraction checks are recorded separately. |
| TCG-GUEST-TESTED | x87/SSE/SSE2 isolation, MMX, OSXSAVE sanitation, `#MF`, and `#UD` pass; strict `#XF` fails on the tested TCG version. |
| HARDWARE-BACKED-GUEST-TESTED | BLOCKED. No successful hardware-backed or independently verified emulator trial is claimed. |
| BLOCKED | Genuine vector-19 delivery/containment and official T2/T3/T4 remain open. |
| NOT RUN | Clean final-source 120/120 aggregate and final qualified archive. |
| Historical diagnostic evidence | 120 groups executed; 115 passed in the initial aggregate, and all five first-run failures passed focused reruns. No single clean aggregate pass. |
| Diagnostic stability evidence | T2 10/10, T3 25/25, and T4 100/100 on the same diagnostic image. |
| Diagnostic archive evidence | Extracted diagnostic archive workflows pass; this cannot close `#XF` or qualify a release. |

## Build, static checks, and compatibility

- Canonical command `tools/build.sh --image`: pass with the protected
  Phase-14 PIE and signed APB1 hashes unchanged.
- Kernel host tests: 8/8 pass. Kernel and fixture rustfmt checks pass.
- Full executable-section ISA audit: 120,911 decoded instructions with
  gap-free coverage; six exact reviewed state-management leaves; zero ordinary
  kernel FP/MMX/SSE/AVX instructions. Empty-disassembly and prohibited-code
  negative controls pass.
- Protected Phase-14 PIE: 23,256 bytes,
  `d7b0a8cf9c735c3898a867d824563f06b0d949df80fa1a9c96f9180a395fea2e`.
- Signed Phase-14 APB1: 24,040 bytes,
  `04add54f09042353fc42511978c82f8b8efb87f7d5f0657f9116f456276b3dfa`.
- The native syscall ABI, Startup ABI v2, signed APB1 verification, static PIE,
  address-space isolation, threading contracts, and capability boundaries
  remain unchanged. Neither Arena 1 nor Arena 2 runtime sources were modified.

## Historical regression run

The diagnostic runner executed 120 test groups against the Phase-15 kernel and
explicitly excluded only the strict M15 `#XF` acceptance case. The initial
aggregate run finished with **115/120 pass and five failed groups**:

| Initially failed group | Focused recovery result | Receipt |
|---|---|---|
| `test_phase14_negative_guest.py` | 7 signed malformed/unsupported PIE cases pass | `phase15-rerun-phase14-negative.log` |
| `test_m10_boundaries_red.py` | 12 production RED controls and restored GREEN guest pass | `phase15-rerun-m10-boundaries-red.log` |
| `test_m84_stage.py` | targeted guest stage/reboot/refusal result pass | `phase15-rerun-m84-stage.log` |
| `test_m8_dependencies.py` | active dependency probes pass | `phase15-rerun-m8-dependencies.log` |
| `test_m8_fault.py` | production fault and absent-device checks pass after correcting interleaved-log parsing | `phase15-rerun-m8-fault.log` |

Every group ran at least once. The affected suites were rerun individually and
passed; the M8 correction changes the test harness only. The five original
failures remain in the record, and no later 120-group aggregate rerun was
completed. Therefore this is recovered historical evidence, not a claim that a
single full runner invocation passed 120/120. Full runner logs and retests are
preserved in the diagnostic archive and under `build/phase15-*` in the
qualification workspace.

## `#XF` investigation and backend status

The strict ring-3 test still executes `sqrtss xmm0,xmm0` with a negative finite
operand. MXCSR reads `0x1F00` immediately before the instruction; on both tested
QEMU TCG CPU profiles it returns with `0x1F01` without delivering vector 19.
The guest records failure, while the same-process sibling and independent peer
survive, the fault slot is released, resources return to baseline, and UEFI
shuts down cleanly. Those containment results do not pass the `#XF` gate.

The QEMU 11.0.2 TCG source analysis matches the observed behavior: its SSE TCG
helper updates MXCSR status but does not check the mask and raise vector 19.
The vector-19 IDT gate and no-error-code stub pass the live-IDT audit. No kernel
workaround or synthetic `int 19` was added.

The earlier emulator host was x86-64 Linux. Its QEMU advertised TCG, KVM, MSHV,
and Nitro; explicit KVM startup failed because the KVM module/device is absent,
MSHV failed opening `/dev/mshv`, and Nitro failed opening
`/dev/nitro_enclaves`. WHPX and HVF were not applicable to that Linux host and
were not advertised by its QEMU binary.
Only TCG was available. See
[`phase15-alternative-backends.json`](receipts/phase15-alternative-backends.json)
and the raw startup logs. The same unchanged TCG experiment was not repeated
for the stability loops; those loops use the diagnostic image and do not run
the `#XF` acceptance case.

The external qualification runner requires explicit `-accel kvm` or
`-accel whpx`, verifies accelerator initialization, checks the strict image
hashes before boot, and captures both CPU-profile trials, actual QEMU exit
codes, raw serial and stderr, and JSON evidence for vector 19, status `0x113`,
sibling/peer survival, slot reclamation, resource restoration, and clean
shutdown. The Linux runner additionally checks `/dev/kvm` and QMP's live
`query-kvm` result. The Windows runner uses loopback TCP QMP/virtconsole
transports for the existing harness; it makes no Windows feature changes.
Neither backend has run successfully here. The external package is not a
qualified archive.

Linux QEMU/KVM is the primary technically suitable target because guest SSE
instructions execute through the host virtualization hardware rather than
QEMU's TCG SSE helpers. It still requires a successful strict guest trial.
Windows QEMU/WHPX is feasible to evaluate, but no exception-delivery behavior
is assumed from WHPX startup; only its exact guest receipt can qualify it.

## Additional architectural coverage

- The seeded OSXSAVE diagnostic entered with `CR4.OSXSAVE=1`, CPUID XSAVE/AVX
  enabled, and XCR0 `0x7`; kernel startup observed XCR0 `0x3`, cleared
  OSXSAVE, verified readback, and continued to native SSE. One diagnostic boot
  passed in 8.697 seconds. Receipt: `build/phase15-osxsave-seed-receipt.json`.
- The bounded MMX fixture checked distinct MM0–MM7 patterns across timer
  preemption and cooperative yield, executed `EMMS`, and passed exact process
  teardown. Optional SSE-family reporting follows CPUID; AVX and OSXSAVE are
  reported disabled after sanitation.
- The normal firmware boot started with OSXSAVE clear; the separate seeded
  diagnostic supplies guest evidence for the inherited-on entry path.

## Stability and archive

- T2: **10/10 diagnostic boots**, 134 seconds, zero failures, one immutable
  diagnostic EFI/ESP identity.
- T3: **25/25 diagnostic boots**, 318 seconds, zero failures, same EFI/ESP
  identity and desktop/service/lifecycle checks.
- T4: **100/100 diagnostic boots** in 1,326 seconds, zero failures, timeouts,
  or retries. All 100 per-boot QMP pixel and lifecycle receipts were retained.
  Its official Phase-15 verdict remains **BLOCKED** because `#XF` is not run in
  the diagnostic image.
- The archive is a **diagnostic checkpoint**, not a release candidate:
  `build/phase15-diagnostic-checkpoint.tar.gz`, 137,994,198 bytes,
  SHA-256 `8fd857c9c46a6c370fc639bbacaa43c46dcba7a5fb3fe0be2ea2274d00eecc9d`.
  The extracted-only verifier passed all 3,335 manifest entries, exact image
  hashes and embedded EFI checks, a diagnostic guest boot with clean shutdown,
  and the signed Phase-14 PIE workflow with two distinct placements and exact
  cleanup. The first extraction stopped before boot on a one-character hash
  pin typo; the verifier and receipt were corrected, the archive was rebuilt,
  and the next clean extraction passed. The first failed attempt is preserved
  in the receipts. This does not qualify genuine `#XF`.

The earlier [`phase15-targeted-results.json`](receipts/phase15-targeted-results.json)
and targeted logs remain immutable receipts for the original focused run; this
document and [`phase15-qualification-recovery.json`](receipts/phase15-qualification-recovery.json)
are the recovery/stability summary. The archive result is recorded in
[`phase15-diagnostic-checkpoint.json`](receipts/phase15-diagnostic-checkpoint.json)
with its raw extracted-run log.

## Gate decision

Report **PHASE 15 — IMPLEMENTED / HISTORICAL REGRESSIONS TESTED / EXCEPTION
QUALIFICATION BLOCKED**. The historical run and all five focused recoveries
are preserved, but the aggregate first run is 115/120 and has no clean full
rerun. Diagnostic stability results do not satisfy official T2/T3/T4.
ADR-0113 remains Proposed. **PARTIAL / NOT READY FOR INTEGRATION REVIEW.**
