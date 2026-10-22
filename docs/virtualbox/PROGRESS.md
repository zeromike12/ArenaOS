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

- Source commit: `6a2635a898e074575f9b62935d2fe710cc0be442`.
- Interactive build: `tools/build.sh --virtualbox-interactive --image` passed
  from that clean commit. EFI size: 10,668,032 bytes; SHA-256
  `58b9917f78766ff00412666b016858ebf5badb20aa2782af8c1a8a88f6355fb8`.
  ESP SHA-256 `334faed46322e48f4967d3cf844c991c07be91c07a044104aa390d634cc686a0`.
  Kernel ISA audit: 75,163 decoded instructions; x87/MMX/SSE/AVX exclusion
  and audit negative controls passed.
- Qualification build: `tools/build.sh --image` passed from the same clean
  commit. EFI size: 11,042,816 bytes; SHA-256
  `7b4ddb99007cc4e31d36586d52944a0a24ad062c4ee7dd9597e6cbc78271ab30`.
  ESP SHA-256 `f14a7931e6cd4d1a1774178f6a1fc983430c4c0daacba45d28e9f9d20658cf2f`.
  Kernel ISA audit: 122,182 decoded instructions; x87/MMX/SSE/AVX exclusion
  and audit negative controls passed.
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
- The interactive image identity is `profile=virtualbox-interactive`; the
  qualification image identity is `profile=qualification`. Both record the
  same clean source commit and distinct EFI/ESP hashes.

## Native settings parse correction

- The initial ZIP put CPU, chipset, and UTC clock settings in the pre-7.1
  `Hardware`/`BIOS` layout while declaring settings version `1.20-linux`.
  VirtualBox 7.1+ requires `Machine/Platform` for that version; the parser
  explicitly throws `Required Machine/@Platform element is missing` when it
  is absent. Its x86 settings also require `Platform/@architecture="x86"`
  and a `Platform/x86` child. This matches the error reported while opening
  the original download.
- The generator now writes generic CPU, ICH9 chipset, and UTC clock settings
  under `Platform`, and the x86 CPU controls under `Platform/x86/CPU`. The
  offline verifier checks the required nodes, values, and absence of their
  obsolete locations. This corrects the reported parser error; it does not
  establish that the VM opens or boots on VirtualBox, which remains untested
  in this managed environment.

## Final downloadable artifact

`docs/virtualbox/artifacts/ArenaOS-VirtualBox-Interactive.zip` is the native
`.vbox` package with portable relative disk files. It is 907,771 bytes with
SHA-256 to be updated after the schema correction is rebuilt. Its manifest
will pin the correction source commit and matching interactive EFI/ESP
identities.
The prior archive SHA-256 was
`9823317f7b18e374ce8e3c48f074d8796470a9bf05c77d009032337ec68d23c8`.
Its manifest pinned source commit `6a2635a898e074575f9b62935d2fe710cc0be442`,
EFI SHA-256 `58b9917f78766ff00412666b016858ebf5badb20aa2782af8c1a8a88f6355fb8`,
and ESP SHA-256 `334faed46322e48f4967d3cf844c991c07be91c07a044104aa390d634cc686a0`.

The exact ZIP was freshly extracted under `/tmp/arenaos-vbox-final`, outside
the working checkout. The standard-library verifier passed. Negative controls
rejected a missing disk descriptor, a mismatched EFI hash, and an IDE
controller substitution even after the modified `.vbox` file hash was
recomputed. These checks validate package integrity and intended settings;
they do not validate VirtualBox parsing or guest execution.

The download URL after this branch is pushed is:
`https://raw.githubusercontent.com/zeromike12/ArenaOS/arena/virtualbox-support/docs/virtualbox/artifacts/ArenaOS-VirtualBox-Interactive.zip`.
