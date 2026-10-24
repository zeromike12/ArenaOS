# QEMU TCG SIMD exception prototype

**Evidence class:** experimental modified-emulator result. **Phase 15 remains
PARTIAL / UNQUALIFIED.** ADR-0113 remains Proposed.

This record contains a source patch against official QEMU v11.1.2, the source
for a separate UEFI exception-control program, and raw command and execution
receipts. The ArenaOS kernel and strict test were not changed. The 44 MiB
original evidence bundle also contained the QEMU release source archive,
firmware, guest images, and full build products; those binaries are not copied
into this repository. The source archive and all tested image identities are
pinned below and in `manifest.json`. The existing
[external qualification kit](../../EXTERNAL-KIT.md) contains the same strict
EFI, ESP, and OVMF identities.

## Result

An unmodified build of official QEMU v11.1.2 failed the unchanged strict guest
test on both `max,+nx,+smep,+smap` and `qemu64,+nx,+smep,+smap`: ArenaOS reported
5/6, MXCSR changed from `0x1f00` to `0x1f01`, and vector 19 was not delivered.
QEMU's process exit was zero, but the guest semantic exit was 97 and the test
runner failed.

With the local prototype patch, the same strict EFI/ESP pair reported 6/6 on
both profiles. Raw guest serial shows vector `0x13`, expected fault status
`0x113`, fault-thread cleanup, sibling and peer survival, resource recovery,
and declared shutdown. The actual accelerator was explicitly `-accel tcg`;
the patched binary listed TCG only, KVM support was disabled in its build, and
`/dev/kvm` was absent.

The independent UEFI controls passed valid SQRTSS, masked invalid SQRTSS,
unmasked invalid SQRTSS with destination preservation, `CR4.OSXMMEXCPT=0`
selecting vector 6, and unmasked DIVSS divide-by-zero. The unmodified control
failed at unmasked invalid SQRTSS after the valid and masked controls passed.
See the complete raw receipts in `logs/` and original invocation records in
`commands/`.

This establishes vector-19 delivery only in this patched TCG experiment. It is
not a pass on upstream QEMU, physical hardware, or a trusted independent
emulator. The patch handles selected common SSE arithmetic helpers and scalar
or packed square-root helpers; it is not a complete implementation of x86 SIMD
floating-point exceptions. The result does not qualify Phase 15 and does not
complete strict stability gates. M and C must review the emulator patch and
decide whether it is acceptable evidence before any qualification credit is
considered.

## Identities

- ArenaOS source baseline: `13986b3dcf007b1721d85ef3509945131f8c6584`, branch
  `arena/phase15-fpu-simd`.
- QEMU: official tag `v11.1.2`; source archive SHA-256
  `fc3d810fc6b27f60df77eaf6748004ca76de2edc2c4ff3a9f9a9ada33066c242`.
- Patch SHA-256:
  `c2909ea974f4ccaa9526bd5b70aef5599971bc567dd87d6ba8d9b06b327c8dd2`.
- Unmodified QEMU binary SHA-256:
  `5c79ebb680b564f661c82cee6849428930d12ebf2f77647b252e6cbe8314e7ed`.
- Patched QEMU binary SHA-256:
  `69a91bba3e81c3505575170571888b01e0a585d2a257b2366fe72b3aa983d6cc`.
- Strict EFI SHA-256:
  `bbbc63df271c3d7d5109789369f72e093b02ef77c77ebe25273b6da4c00d238e`.
- Matching strict ESP SHA-256:
  `631cc6452ecbacf58b2418e54b9c509bb717cc2211580ebef63f9aa4c13d866f`.
- OVMF Code SHA-256:
  `7e34e3b83224968bb0714c0ebc88f75c5c37b4cc892c902e511ccc7a7c107f94`.
- OVMF Vars template SHA-256:
  `5d2ac383371b408398accee7ec27c8c09ea5b74a0de0ceea6513388b15be5d1e`.
- Original full evidence bundle SHA-256:
  `a962c27c56d54e0ce67391a0fbc7049539237daf1c21e4698594d389f1265aab`.

## Rebuilding the experiment

Fetch QEMU only from the official QEMU project source at tag `v11.1.2` and
verify the release archive against the hash above. Apply
`source/qemu-tcg-xm.patch` in that source tree. Configure and build the
`x86_64-softmmu` target with TCG; do not apply the patch to ArenaOS. Use the
strict images and OVMF files from the existing external kit only after
verifying every hash above, then run `tools/test_m15_guest.py` with the two
CPU profiles and explicit `-accel tcg`. The captured commands in `commands/`
record the original tested invocations and environment paths; replace those
paths with the verified local artifact locations when reproducing.

The standalone control source is `source/independent-xm-controls.s`. Its
original QEMU commands, build receipts, raw logs, and exit-status files are
retained here. `SHA256SUMS` covers every file in this committed experiment
directory except itself.

## Evidence limit

The historical diagnostic aggregate remains 120/120 and diagnostic stability
remains T2 10/10, T3 25/25, and T4 100/100. Those runs used diagnostic images
and are not strict `#XF` qualification. No official strict T2/T3/T4 stability,
final qualification archive, or acceptance of ADR-0113 is claimed here.
