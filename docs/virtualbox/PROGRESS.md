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
- Added a reproducible native `.vbox` plus VirtualBox-native VDI appliance generator
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
  the original download. Oracle's upstream parser implements this in
  [`Settings.cpp`](https://github.com/VirtualBox/virtualbox/blob/main/src/VBox/Main/xml/Settings.cpp#L7258-L7272)
  and requires the architecture and x86 child in
  [`readPlatform`](https://github.com/VirtualBox/virtualbox/blob/main/src/VBox/Main/xml/Settings.cpp#L5599-L5635).
- The generator now writes generic CPU, ICH9 chipset, and UTC clock settings
  under `Platform`, and the x86 CPU controls under `Platform/x86/CPU`. The
  offline verifier checks the required nodes, values, and absence of their
  obsolete locations. This corrects the reported parser error; it does not
  establish that the VM opens or boots on VirtualBox, which remains untested
  in this managed environment.

## Disk-open error follow-up

- C supplied the exact `ArenaOS.vbox` and `ArenaOS-Data.vmdk` that produced
  `VERR_INVALID_UUID_FORMAT` on VirtualBox 7.2.22 r175245. Their SHA-256 values
  match the published package exactly. The `.vbox` data-disk UUID matches the
  VMDK image UUID, and all four VMDK UUID strings are canonical. The supplied
  files therefore rule out an old or mismatched extraction.
- Oracle's 7.2.22 VMDK reader calls `RTUuidFromStr` on those DDB values, and the
  exact strings meet that parser's documented format. This source inspection
  could not explain C's reproduced error, and the managed environment cannot
  open a VM to produce a local runtime receipt.
- Replaced both text VMDKs with fixed native VDI 1.1 images. This bypasses the
  VMDK descriptor UUID parser that triggers the reported error. The VDI writer
  follows Oracle's 7.2.22 `VDICore.h` and `VDI.cpp` layout; the offline
  verifier checks header offsets, fixed block maps, logical disk hashes, UUID
  identity against the `.vbox`, GPT/ESP, and AFS1/AFS2 data. The new VDI package
  is not yet opened on a Windows host, so its GUI import and guest boot remain
  unverified.
- C reported the same `.vmdk` error after trying the VDI package. That exact
  disk path is absent from the VDI archive, whose settings refer only to
  `.vdi` disks, so VirtualBox was still opening the earlier VMDK settings or
  registered VM. To make the new package unmistakable, it now has a unique
  `ArenaOS-VDI.vbox` filename, package-specific `ArenaOS VDI <commit>` name,
  and machine UUID. Open that file from a fresh extraction; do not start the
  old `ArenaOS` entry.
- C then reported `E_INVALIDARG` while opening `ArenaOS-VDI.vbox`. Inspection
  showed that the new machine UUID alone was not enough: its VDI UUIDs were
  still identical to the preceding package, while the new extraction had a
  different path. VirtualBox 7.2's [`i_checkMediaForConflicts`](https://github.com/VirtualBox/virtualbox/blob/main/src/VBox/Main/src-server/VirtualBoxImpl.cpp#L4950-L5005)
  and [`i_registerMedium`](https://github.com/VirtualBox/virtualbox/blob/main/src/VBox/Main/src-server/VirtualBoxImpl.cpp#L5599-L5608)
  detect a registered disk UUID/path conflict and return `E_INVALIDARG`.
  This matches the reported open-time error and repeated extraction pattern.
- The package now derives machine UUID, both VDI UUIDs, and the adapter MAC
  from the package generator commit as well as the EFI/ESP identities. The
  offline verifier checks these values and their embedded VDI identities.
  Open one extraction; use its registered VM entry for later starts instead
  of opening duplicate copies of the same ZIP.
- C reported `VERR_VD_VDI_INVALID_HEADER` opening the new data disk. The VDI
  writer stored a valid creation UUID but left the modification UUID zero.
  Oracle's [`vdiValidateHeader`](https://github.com/VirtualBox/virtualbox/blob/main/src/VBox/Storage/VDI.cpp#L423-L535)
  explicitly rejects a null modification UUID; the official creator generates
  one at [`vdiCreateImage`](https://github.com/VirtualBox/virtualbox/blob/main/src/VBox/Storage/VDI.cpp#L766-L768).
  The writer now emits a deterministic nonzero modification UUID for each disk,
  and the verifier checks both UUID fields against the package manifest.

## ICH9 / I/O APIC startup failure follow-up

- The new `VBox.log` from VirtualBox 7.2.22 r175245 shows that the package
  passed settings loading and firmware initialization, then failed during
  device construction. At `00:00:04.092613`, `DevPciIch9.cpp(3364)` reports
  `Must use IO-APIC with ICH9 chipset`; the following `VERR_PDM_NO_PCI_BUS`
  occurs because device construction aborts. No ArenaOS guest code ran.
- The generated settings declare version `1.20-linux` but put `IOAPIC` under
  `Hardware/BIOS`. Oracle's 7.2.22 `Settings.cpp` reads `IOAPIC` from
  `Hardware/Firmware` when the settings version is 1.20 or newer
  ([reader](https://github.com/VirtualBox/virtualbox/blob/v7.2.22/src/VBox/Main/xml/Settings.cpp#L5900-L5920));
  it only reads the `BIOS` child for older settings versions. The ICH9 check
  therefore saw IOAPIC disabled, despite the misplaced XML element.
- The generator now stores IOAPIC and the other supported firmware options
  inside the version 1.20 `Firmware` element. It removes the legacy `BIOS`
  element and redundant keyboard/pointing-device fields; the actual PS/2
  devices remain configured under `Hardware/HID`. The offline verifier now
  requires IOAPIC under `Firmware` and rejects the legacy `BIOS` node. Oracle's
  7.2.22 writer uses the same location
  ([writer](https://github.com/VirtualBox/virtualbox/blob/v7.2.22/src/VBox/Main/xml/Settings.cpp#L7910-L7925)).
- This is a host-appliance configuration correction based on the exact logged
  failure. It does not change ArenaOS source; the following Windows trial
  confirms that the corrected VM configuration now starts.

## Black-screen guest run follow-up

- C's next `VBox.log` is from Windows 11 and VirtualBox 7.2.22 r175245. The
  corrected `ArenaOS VDI 8825a48` VM starts without the prior ICH9/IOAPIC
  error. UEFI loads the disk EFI image at `00:00:05.163548`, changes the
  display mode to 800x600, and VirtualBox records successful AHCI reads from
  both included VDIs. The guest remains running until the host pauses and
  powers it off; no VM device-construction error is logged.
- The CPU snapshot at host power-off has long mode enabled and GDT/IDT bases
  in ArenaOS's high kernel address range. RIP is `0x3e8e4302`, inside the
  loaded `ResetSystemRuntimeDxe.efi` image. This is consistent with execution
  reaching ArenaOS and then entering the firmware reset service, but the
  VirtualBox log cannot identify why that service was called. The screen was
  black, and the appliance did not route COM1 anywhere, so the guest's exact
  startup or halt message was not preserved.
- The next package enables the guest's existing 38400-baud COM1 output and
  writes it to `ArenaOS-serial.log` beside the `.vbox` file. The README asks
  the operator to send this file if the screen remains black. This changes
  only VM diagnostics; it does not change ArenaOS or count as a boot pass.

## Final downloadable artifact

`docs/virtualbox/artifacts/ArenaOS-VirtualBox-Interactive.zip` contains the
`ArenaOS-VDI.vbox` configuration and fixed VDI boot/data disks. Its manifest pins
interactive image source commit `6a2635a898e074575f9b62935d2fe710cc0be442`
and package generator commit `8825a48dc9e78165b36c2ef3144e11d45ed01a87`.
The ZIP is 909,359 bytes with SHA-256
`13427a35c7874747bc0766f54b9d083d152fdd524460f1f327ce59b7713e3c46`.
The VDI packaging change does not rebuild the EFI or ESP; their hashes remain:
EFI SHA-256 `58b9917f78766ff00412666b016858ebf5badb20aa2782af8c1a8a88f6355fb8`,
and ESP SHA-256 `334faed46322e48f4967d3cf844c991c07be91c07a044104aa390d634cc686a0`.

The VDI ZIP is generated and checked by the standard-library verifier. Offline
checks validate package identities and disk structures; the external
VirtualBox attempt confirms settings acceptance and early guest execution but
not a successful desktop boot. Extract the latest ZIP into a new folder and
open `ArenaOS-VDI.vbox` in VirtualBox 7.2.22. Use the exact VM name and
diagnostic instructions in the ZIP's README. Each release's machine UUID, VDI
UUIDs, and network MAC are unique from earlier package revisions. The settings
enable IOAPIC in the 1.20 `Firmware` element and capture COM1 output to a
package-local log.

The download URL at this branch is:
`https://raw.githubusercontent.com/zeromike12/ArenaOS/arena/virtualbox-support/docs/virtualbox/artifacts/ArenaOS-VirtualBox-Interactive.zip`.
