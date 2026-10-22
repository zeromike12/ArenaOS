# VirtualBox compatibility progress

Base commit: `13986b3dcf007b1721d85ef3509945131f8c6584`.
Working branch: `arena/virtualbox-support`.
Phase 15 qualification remains PARTIAL / UNQUALIFIED; strict #XF is unresolved,
official strict stability is incomplete, and ADR-0113 is Proposed.

## Completed

- Reviewed the official Oracle VirtualBox 7.2.22 User Guide. The selected
  target profile is documented in [COMPATIBILITY.md](COMPATIBILITY.md).
- Added Proposed ADR-0114 for the guest profile and entropy assumptions.
- Recorded the source/device compatibility inventory and implementation
  sequence.
- Restored an official Rust 1.97.0 toolchain into the workspace for builds.
  This does not install or modify a host hypervisor.

## Environment and evidence

- The current environment has no `VBoxManage`, `/dev/vboxdrv`, or `/dev/kvm`.
- No ArenaOS source changes or VirtualBox guest tests have been completed yet.
- No appliance has been built. Do not describe this branch as VirtualBox
  compatible or the proposed profile as verified.
- The earlier Phase 15 diagnostic and strict-failure evidence is preserved on
  the base commit and its original branch; this work does not change it.

## Next implementation work

1. Add a separate interactive build identity and skip only boot-time test
   suites and fixture-dependent qualification clients in that profile.
2. Implement and build an AHCI backend for the existing ring-3 block service.
3. Add least-privilege PS/2 input and checked entropy-source handling.
4. Build a persistent disk, assemble an importable VirtualBox package, and
   verify its identities and settings offline.
5. Run and record real VirtualBox tests on a host with the official runtime
   and usable hardware backend; otherwise report the exact external actions.

