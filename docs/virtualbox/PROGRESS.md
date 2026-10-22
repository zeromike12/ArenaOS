# VirtualBox compatibility progress

Base checkpoint: `13986b3dcf007b1721d85ef3509945131f8c6584`.
Branch: `arena/virtualbox-support`.
Phase 15 remains PARTIAL / UNQUALIFIED; strict #XF is unresolved, official
strict stability is incomplete, and ADR-0113 remains Proposed.

## Completed in this branch

- Recorded the compatibility inventory and proposed the one-vCPU VirtualBox
  7.2.22 profile in ADR-0114.
- Added a distinct interactive EFI/ESP build profile. It omits automated
  qualification suites and deliberate fault probes while retaining live
  kernel checks, service readiness, runtime capability checks, and
  fail-closed initialization. The qualification build remains separate.
- Added an AHCI fallback to `storaged` using its existing block IPC, with the
  protocol driver in ring 3 and bounded kernel BAR authorization.
- Added least-privilege PS/2 input mediation and ring-3 keyboard/mouse decoding.
- Added a capability-gated, checked RDRAND source for the existing kernel
  CSPRNG. Startup fails closed if CPUID, carry, or health checks fail.
- Added a reproducible native `.vbox` plus relative VMDK appliance generator
  and a standard-library offline verifier. The data disk uses the existing
  AFS1 + blank AFS2 first-boot import layout.
- Pinned the build dependency for the repository's `pyfatfs` helper to
  `setuptools<81`; the runtime VM package needs no Python or host script.

## Build and host checks

- Interactive build: `tools/build.sh --virtualbox-interactive --image` passed.
  EFI size: 10,668,032 bytes. Kernel ISA audit: 75,163 decoded instructions;
  x87/MMX/SSE/AVX exclusion and audit negative controls passed.
- Qualification build: `tools/build.sh --image` passed. EFI size:
  11,042,816 bytes. Kernel ISA audit: 122,182 decoded instructions;
  x87/MMX/SSE/AVX exclusion and audit negative controls passed.
- AHCI command encoder host tests: 3/3 passed. PS/2 decoder host tests: 3/3
  passed. All modified Rust files passed `rustfmt --check`; Python compile,
  shell syntax, and `git diff --check` passed.
- Build profile guard: `ARENA_GRAPHICS_FIXTURE=phase9` is rejected for the
  interactive artifact.
- Both builds retained the protected Phase 14 PIE and signed APB1 fixture
  hashes: `d7b0a8cf9c735c3898a867d824563f06b0d949df80fa1a9c96f9180a395fea2e`
  and `04add54f09042353fc42511978c82f8b8efb87f7d5f0657f9116f456276b3dfa`.

## Reproduced environment blocker and remaining gaps

- `/dev/vboxdrv`, `/dev/kvm`, `qemu-system-x86_64`, and a usable VirtualBox
  COM service are unavailable here. The signed Debian VirtualBox 7.2.22
  package's version binary ran from an isolated extraction, but `VBoxSVC`
  crashed there and the manager could not create a VM. Oracle's direct
  download endpoint returned HTTP 403. No attempt reached firmware or guest
  boot.
- No actual VirtualBox open/import, guest boot, desktop/input, network, disk
  persistence, application lifecycle, shutdown, or fresh-boot trial has run.
  Offline package verification is not runtime compatibility evidence.
- QEMU guest regressions were not rerun because no QEMU executable is
  installed in this environment. The original Phase 15 evidence remains on
  the base branch and was not restarted.
- The final appliance is being assembled from the committed interactive
  source. Its exact ZIP hash and source/EFI/ESP identities will be recorded
  below after clean-source packaging.

## Final downloadable artifact

Pending final clean-source package and independent extraction verification.
