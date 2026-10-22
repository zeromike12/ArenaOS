# ArenaOS VirtualBox compatibility

Status: **implementation package built; guest compatibility is ready for
external testing and is not yet verified**. This branch is based on Phase 15
checkpoint `13986b3dcf007b1721d85ef3509945131f8c6584`. Phase 15 remains
PARTIAL / UNQUALIFIED: the strict #XF gate is unresolved, official strict
stability is incomplete, and ADR-0113 remains Proposed.

## Interactive VM target

The appliance targets Oracle VirtualBox 7.2.22 on an x86-64 host. The
configuration in `ArenaOS-VDI.vbox` is self-contained with relative disk paths and
sets:

| Setting | Value |
| --- | --- |
| Virtual CPUs | 1 |
| Memory | 1024 MiB |
| Chipset / interrupt controller | ICH9 / I/O APIC |
| Firmware | VirtualBox EFI; Secure Boot off |
| Boot device | Included GPT disk with FAT EFI System Partition |
| Display | VBoxVGA, 16 MiB, 3D disabled; guest uses UEFI GOP |
| Input | PS/2 keyboard and mouse |
| Data storage | AHCI SATA: persistent 72 MiB disk, port 0 |
| Boot storage | AHCI SATA: GPT/FAT EFI disk, port 1 |
| Network | VirtualBox paravirtualized VirtIO adapter, NAT |
| Boot diagnostics | COM1, 38400 8N1, captured to `ArenaOS-serial.log` beside the VM settings |
| Guest Additions | Not required or used |
| Audio, clipboard, drag/drop, 3D | Disabled; not part of this profile |

VirtualBox supplies its own EFI firmware. An external Windows run on VirtualBox
7.2.22 r175245 opened the corrected configuration, read the EFI disk, and
entered 64-bit guest execution, but showed a black screen and was stopped from
the VirtualBox UI. The package did not capture guest COM1 output, so the exact
boot result is unknown. Desktop rendering, PCI and GOP use, AHCI execution,
PS/2 I/O, VirtIO-net, RDRAND exposure, timer behavior, application workflow,
shutdown, and disk persistence remain unverified. The profile relies on the
host CPU and VirtualBox exposing RDRAND; ArenaOS fails closed if its checked
entropy source is unavailable.

## Compatibility implementation

The build profiles have separate names and output identities:

- `tools/build.sh --image` builds the existing qualification image and does
  not enable VirtualBox fallbacks. Its separate identity is written to
  `build/arena-qualification.identity`.
- `tools/build.sh --virtualbox-interactive --image` builds a distinct
  ordinary-use image and writes `build/arena-virtualbox-interactive.identity`.
  It omits automated qualification suites and deliberate fault probes while
  keeping live-kernel startup checks, process/capability validation, service
  readiness checks, and fail-closed initialization.

The interactive profile adds a ring-3 AHCI storage driver behind the existing
block IPC contract, a capability-gated PS/2 controller syscall and ring-3
decoder, and a checked RDRAND-to-kernel-CSPRNG path. The kernel authorizes one
bounded native AHCI BAR and exact PS/2/RDRAND source capabilities; it does not
give userspace arbitrary PCI configuration or I/O-port access. Network and
display continue through the existing userspace VirtIO-net and UEFI GOP paths.
No AVX state or Phase 15 test behavior was changed.

## Appliance and first boot

The artifact at `docs/virtualbox/artifacts/ArenaOS-VirtualBox-Interactive.zip`
contains the `ArenaOS-VDI.vbox` configuration, fixed VDI 1.1 boot and data disks, README,
and JSON manifest. It is intended to be opened from
VirtualBox's normal interface after extraction; no host-side Python, Rust,
QEMU, scripts, or Guest Additions are required. The boot disk has a 16 MiB
FAT16 ESP containing
the exact interactive EFI. The 72 MiB data disk contains the existing 8 MiB
AFS1 format followed by a blank 64 MiB AFS2 region; `filesd` initializes AFS2
and imports the AFS1 contents on first boot. User-created files then use the
existing persistent AFS2 service.

The `.vbox` file uses the VirtualBox settings 1.20 platform layout. Its x86
architecture, CPU profile, chipset, and UTC clock are stored in `Machine/Platform`
as required by VirtualBox 7.1 and later. The offline verifier enforces that
layout and rejects the older `Hardware/CPU`, `Hardware/Chipset`, and
`BIOS/RTCUseUTC` locations.

The package verifier uses Python's standard library and checks every file
hash, the `.vbox` device configuration, relative disk references, VDI 1.1
headers including nonzero creation and modification UUIDs, fixed block maps,
image UUID matches, GPT headers/tables, exact embedded EFI
and ESP hashes, AFS1 geometry, and the blank AFS2 region. This offline
validation does not prove that VirtualBox accepts the native XML or that a
guest boots.

### Run

1. Download the latest `ArenaOS-VirtualBox-Interactive.zip` and extract it into a new folder.
2. Open `ArenaOS-VDI.vbox` from that folder in VirtualBox.
3. Select the VM named **ArenaOS VDI** followed by the package commit's short ID, then click **Start**. The exact name appears in the ZIP's `README.txt`.
4. If the screen stays black or startup stops, power off the VM, then send `ArenaOS-serial.log` from the same folder. It contains ArenaOS's boot and failure messages.

This package uses a new settings filename, VM name, and machine UUID. If an
error still names `ArenaOS-Data.vmdk`, VirtualBox opened an older VM
configuration; the current package contains `ArenaOS-Data.vdi` only. Open
`ArenaOS-VDI.vbox` from the fresh extraction instead of starting the older
**ArenaOS** entry in the VM list.

VirtualBox registers disk UUIDs globally. Each package revision now derives
its machine and VDI UUIDs, plus the network MAC, from that package revision.
Extract one copy and open it once; for later launches, choose the matching
**ArenaOS VDI** entry from VirtualBox's VM list rather than registering
another extraction of the same ZIP.

The selected host must run 64-bit VirtualBox 7.2.22 with hardware
virtualization available to the hypervisor and a CPU whose RDRAND source is
exposed to the guest. Other VirtualBox versions remain unverified. No
administrator script or host security change is needed.

## Verification matrix

| Area | Current evidence | Status |
| --- | --- | --- |
| Qualification and interactive build separation | Both profiles compiled; qualification profile unchanged | Build verified |
| Kernel instruction profile | Interactive kernel ISA audit: 75,303 decoded instructions; x87/MMX/SSE/AVX policy passed | Build verified |
| AHCI protocol encoding | Three host tests passed; userspace driver compiled | Host/build verified; no HBA execution |
| PS/2 decoding | Three host tests passed; input service compiled | Host/build verified; no controller execution |
| EFI and package identities | Freshly extracted package verifier checked EFI/ESP hashes and GPT/FAT layout | Offline verified |
| VM settings and storage files | Opened and ran on VirtualBox 7.2.22; EFI VDI was read | Runtime startup passed; guest boot incomplete |
| UEFI boot and desktop rendering | EFI image loaded and guest entered 64-bit state; user saw a black screen; no serial output was captured | Boot progress observed; desktop unverified |
| Keyboard/mouse and window interaction | No VirtualBox guest run | Unexecuted |
| AFS2 first boot, file edits, and persistence | No VirtualBox guest run | Unexecuted |
| VirtIO-net/NAT and existing network workflows | No VirtualBox guest run | Unexecuted |
| Timers, application lifecycle, shutdown/reboot | No VirtualBox guest run | Unexecuted |
| 100 fresh successful boots | No VirtualBox guest run | Unexecuted |
| Phase 15 genuine vector 19 and strict stability | No qualifying result from this work | PARTIAL / UNQUALIFIED |

## Host execution limitation

This managed Linux environment has no `/dev/vboxdrv` or `/dev/kvm`. An
official Debian-signed VirtualBox 7.2.22 package was inspected and its version
binary runs from an isolated extraction, but the COM service fails to start
and `VBoxSVC` crashes in that unsupported isolated runtime. Oracle's direct
binary download endpoint returned HTTP 403. This environment did not reach VM
creation or guest boot. C's external Windows `VBox.log` confirms firmware and
kernel entry, but there is no screenshot or guest serial log, and no
successful-boot or VirtualBox compatibility claim is made.

The exact package is ready for M to open on an ordinary x86-64 host with the
specified official VirtualBox runtime. After opening it, verify the desktop,
keyboard/mouse, application launch/exit, create/edit/read a user file, restart
and confirm persistence, confirm a network request, and shut down from ArenaOS.
Save the VirtualBox version, host CPU/RDRAND availability, imported settings,
`VBox.log`, guest serial/debug output, screenshots, and test results. A
qualified compatibility milestone still needs 100/100 fresh successful
ArenaOS boots. These are compatibility checks and do not satisfy Phase 15's
strict #XF gate.
