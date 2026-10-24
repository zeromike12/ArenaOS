# Phase 15 locally modified QEMU TCG qualification

Phase 15 qualification was **ACCEPTED by M and C on 2026-10-24** for the
explicitly pinned, locally modified QEMU TCG backend. ADR-0113 remains Proposed
as a separate architecture decision. No ArenaOS kernel source or strict
acceptance assertion was changed for exception delivery. This result is neither
upstream-QEMU nor hardware-backed; see the [acceptance record](ACCEPTANCE.md).

## Tested backend and immutable artifacts

- Official QEMU v11.1.2 source tag `ceaea6cb3e2a7d966f9ca97c7ebb5b88cd764dd0`,
  peeled source commit `4fc49f46dc95d4a27de2509e7fceb2931e91faeb`; official source
  archive SHA-256 `fc3d810fc6b27f60df77eaf6748004ca76de2edc2c4ff3a9f9a9ada33066c242`.
- Local TCG patch SHA-256
  `c2909ea974f4ccaa9526bd5b70aef5599971bc567dd87d6ba8d9b06b327c8dd2`.
- Selected patched QEMU executable SHA-256
  `e0ea2b1f5ae2a5aa65a455eb232e46e1743b5c327c48ca678630c214c69f26ff`.
  The same-source unpatched comparison executable SHA-256 is
  `443c81fc9482493297ae7a3f37300bef3a82a2873e428f72884cf52d16e27284`.
- Accelerator selection is explicitly `-accel tcg`. QMP initialization passed;
  the binary supports TCG only and KVM was disabled at build time. `/dev/kvm`
  is absent.
- Strict EFI SHA-256:
  `bbbc63df271c3d7d5109789369f72e093b02ef77c77ebe25273b6da4c00d238e`.
- Its matching ESP SHA-256:
  `631cc6452ecbacf58b2418e54b9c509bb717cc2211580ebef63f9aa4c13d866f`.
- OVMF Code SHA-256:
  `7e34e3b83224968bb0714c0ebc88f75c5c37b4cc892c902e511ccc7a7c107f94`.
- OVMF Vars template SHA-256:
  `5d2ac383371b408398accee7ec27c8c09ea5b74a0de0ceea6513388b15be5d1e`.
- The guest image is byte-identical to the strict Phase 15 image built from
  ArenaOS commit `13986b3dcf007b1721d85ef3509945131f8c6584`; kernel and strict
  guest sources remained unchanged through the tested source commit
  `f54797f2fb5bac0b3440f986d2120382b3611ecc`.

The complete backend pin set and test claims are in
[`patched-tcg-qualification.json`](patched-tcg-qualification.json). QEMU patch
scope and raw independent controls are in the
[prototype evidence record](experiments/qemu-tcg-xm-prototype/README.md).

## Exception controls and strict guest

The expanded independent SSE/SSE2 controls passed on the patched build and
failed the expected exception cases on the same-source unpatched build. They
check valid and masked operations, previously sticky flags, new exceptions
when a flag was already sticky, `CR4.OSXMMEXCPT`-disabled `#UD`, unmasked
`DIVSS`, exact saved fault RIPs, complete 128-bit destination preservation,
and no partial packed `SQRTPS` destination commit. The tested helper families
are documented in the source review; this does not claim every SIMD instruction
was implemented or tested.

The unchanged ArenaOS strict guest passed **6/6** on both
`max,+nx,+smep,+smap` and `qemu64,+nx,+smep,+smap`. The receipts show genuine
instruction-generated vector 19, fault status `0x113`, fault-thread termination,
same-process sibling and independent-process survival, scheduler-slot
reclamation, restored resource baselines, and clean shutdown. Each QEMU process
exited zero. The same-source unpatched control exited QEMU zero but returned
guest semantic result 97 / **FAIL 5/6** on both profiles.

## Registered regression and official strict stability

The current `tools/run_tests.sh` official runner completed **121/121 test
suites** with source commit `f54797f2fb5bac0b3440f986d2120382b3611ecc` clean at
start and end. The suite includes the Phase 15 strict guest on both CPU
profiles. The historical diagnostic aggregate **120/120** is also retained as
separate evidence; the current 121-suite result is the new registered total.

The strict stability harness required Phase 15 PASS 6/6, vector 19, status
`0x113`, containment/resource receipts, clean shutdown, and all prior stability
checks on every boot. All gates passed on fresh boots with explicit TCG and the
same exact image and firmware identities:

| Tier | Required | Passed | Failed | Timeouts | Retries |
|---|---:|---:|---:|---:|---:|
| T2 | 10 | 10 | 0 | 0 | 0 |
| T3 | 25 | 25 | 0 | 0 | 0 |
| T4 | 100 | 100 | 0 | 0 | 0 |

A prior T4 process was interrupted by the managed execution environment after
93 passing boots. It is preserved as an incomplete attempt and is not counted
as the T4 gate. A new uninterrupted T4 run passed 100/100 in 948 seconds. An
earlier zero-boot attempt is also preserved: the stability harness correctly
refused to start while the production desktop profile marker was `phase9`; a
canonical desktop build restored it before the qualifying run.

Final source checks passed: canonical build, host tests in the 121-suite
regression, and the kernel integer-only ISA audit over 120,911 decoded
instructions. Protected Phase 14 identities remain unchanged:

- PIE: `d7b0a8cf9c735c3898a867d824563f06b0d949df80fa1a9c96f9180a395fea2e`
- Signed APB1: `04add54f09042353fc42511978c82f8b8efb87f7d5f0657f9116f456276b3dfa`

## Download and verification

The review archive and its SHA-256 sidecar are committed at:

- [Download the locally modified TCG qualification archive](artifacts/phase15-patched-tcg-qualification.tar.gz)
- [Archive SHA-256 sidecar](artifacts/phase15-patched-tcg-qualification.tar.gz.sha256)

The archive includes the exact guest images and OVMF files, patched and
unpatched QEMU executables and required runtime libraries, ROM data, patch and
rebuild script, controls, strict and stability raw receipts, the full regression
log, final source checks, and an independent manifest verifier. The package
README describes its source-checkout prerequisite and official dependency
sources. Extract outside the checkout and run
`overlay/tools/verify_phase15_patched_tcg_archive.py` with the pinned Python
dependencies before relying on the evidence. A final extraction outside the checkout
verified all 2,319 manifest file hashes, both strict CPU-profile receipts, T2/T3/T4,
the independent controls, the 121-suite regression, and the protected Phase 14
identities. The host-side negative controls rejected missing evidence and altered
EFI, ESP, and QEMU identities; the semantic-failure control remained FAIL when
QEMU exit zero was supplied; and the accelerator-init control failed before any
ArenaOS guest started. See
[`receipts/package-verification`](receipts/package-verification/README.md) and
[`receipts/package-negative-controls`](receipts/package-negative-controls/README.md)
for commands and machine-readable results.

Two initial packaging checks found assembly issues (nested manifest files were
missing from the hash inventory, then the copied overlay lacked the independent
control-verifier script). Both were corrected before the final archive was built;
the final fresh extraction passed. These did not change guest evidence.

This archive is distinct from the hardware-only KVM/WHPX external kit in
[`EXTERNAL-KIT.md`](EXTERNAL-KIT.md). Windows guest execution and hardware
acceleration were not tested. M and C accepted this scoped backend result; ADR-0113
remains Proposed, and no branch merge or release is included.
