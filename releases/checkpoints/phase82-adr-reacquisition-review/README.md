# Phase 8.2 ADR-0048 revision — deployable design-review snapshot

This commit revises the **proposed** ADR and adds a host-only Rust-layout
measurement tool; no kernel/userspace runtime code or boot image was
modified. Its tarball is byte-for-byte identical to the Phase 8.1
complete archive first qualified at commit `c33b46b` (and to the first
8.2 ADR-proposal review tarball). The embedded `QUALIFICATION.txt` and
`RUNNING.md` correctly describe **Phase 8.1**, not an implemented
Phase 8.2. This is not a newly completed milestone; no fresh 100-boot
qualification is claimed for the documentation revision.

Unchanged archive SHA-256:
`e114888dce52d3f0dd4dacc76791caca62e594b9bf71c7db64e4a47404b6c2d4`

Unchanged EFI SHA-256:
`21f9df0b7444acb52b7e9cb908cec4945c0461b59ee9420655af63a18a9335c8`

The identical EFI already passed 31 historical suites and an
artifact-bound 100/100 boot loop; the original extracted archive was
boot-verified. To use the deployable review image:

```sh
sha256sum -c arenaos-phase82-adr-reacquisition-review-qemu-x86_64.tar.gz.sha256
tar xzf arenaos-phase82-adr-reacquisition-review-qemu-x86_64.tar.gz
sha256sum -c sha256sums.txt
cp ovmf-vars-template.img ovmf-vars.img
cp scratch-template.img scratch.img  # first boot only; retain thereafter
# Run the QEMU command in the extracted RUNNING.md.
```
