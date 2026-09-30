# Phase 8.2 ADR proposal — bootable review snapshot

This commit changes **documentation only**. Its deployable archive is
**byte-for-byte identical** to the previously qualified Phase 8.1 complete
archive from commit `c33b46b`; it does not implement any Phase 8.2
permissions. The original archive's `QUALIFICATION.txt` and `RUNNING.md`
remain embedded, intentionally identify `phase81-complete`, and accurately
describe what actually boots. Do not mistake the outer review-snapshot
filename for new runtime functionality or a new completed milestone.

SHA-256 of either tar.gz:
`e114888dce52d3f0dd4dacc76791caca62e594b9bf71c7db64e4a47404b6c2d4`

Previously qualified unchanged EFI SHA-256:
`21f9df0b7444acb52b7e9cb908cec4945c0461b59ee9420655af63a18a9335c8`

The **identical executable** previously passed all 31 historical suites,
a fresh final-EFI-bound 100/100 boot loop, and verification/boot from the
extracted Phase 8.1 archive. There is no new 8.2 milestone qualification
or test claim. The proposed design in `docs/adr/0048-*.md` requires
review *before* implementation.

To boot this snapshot:

```sh
sha256sum -c arenaos-phase82-adr-proposal-qemu-x86_64.tar.gz.sha256
tar xzf arenaos-phase82-adr-proposal-qemu-x86_64.tar.gz
sha256sum -c sha256sums.txt
cp ovmf-vars-template.img ovmf-vars.img
cp scratch-template.img scratch.img  # only on first boot; retain thereafter
# Then use the QEMU command in the extracted RUNNING.md.
```
