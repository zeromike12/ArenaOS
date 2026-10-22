# ArenaOS VirtualBox compatibility

Status: implementation in progress; no VirtualBox guest execution has been
completed. This branch is based on Phase 15 checkpoint
`13986b3dcf007b1721d85ef3509945131f8c6584`. Phase 15 remains PARTIAL /
UNQUALIFIED, with the #XF gate unresolved and ADR-0113 Proposed.

## Compatibility inventory

The Phase 15 source currently assumes modern VirtIO PCI for block, network,
entropy, and input. Its display service already has a capability-owned UEFI
GOP fallback. The kernel owns PCI enumeration, DMA authorization, and interrupt
routing; device protocol drivers run in ring 3.

| Subsystem | Current implementation | VirtualBox profile | Work needed |
| --- | --- | --- | --- |
| Firmware | x86-64 UEFI application; firmware facts cross `ExitBootServices` as scalars | VirtualBox EFI, Secure Boot off | Verify the firmware handoff and boot order in VirtualBox |
| Display | Ring-3 GOP scanout or VirtIO-GPU; no VBoxSVGA driver | 800×600 EFI GOP, VGA-compatible controller | Use GOP; confirm mode and framebuffer mapping in VirtualBox |
| Storage | Ring-3 VirtIO-blk; no VirtIO-SCSI or AHCI | SATA AHCI disk | Add AHCI support in `storaged`, retain block-service IPC |
| Keyboard/mouse | Ring-3 VirtIO-input keyboard/tablet | Built-in PS/2 keyboard and mouse | Add narrowly mediated PS/2 port access and ring-3 event decoding |
| Network | Ring-3 modern VirtIO-net, including MSI-X relay | Paravirtualized VirtIO-net, VirtualBox NAT | Confirm VirtualBox's exposed PCI transport, MSI-X, and real packet path |
| Entropy | Ring-3 VirtIO-RNG seeds the kernel CSPRNG | CPU RDRAND when VirtualBox exposes it | Add a checked RDRAND source; fail closed if unavailable or unhealthy |
| Timer/interrupts | PIT, LAPIC, IOAPIC, HPET, one CPU | ICH9, I/O APIC, one CPU | Verify routing, monotonic time, sleep, and scheduler under VirtualBox |
| Shutdown | Capability-gated UEFI `ResetSystem` | VirtualBox EFI shutdown | Verify a clean guest shutdown and reboot |
| Startup | Boot path runs qualification and diagnostic workloads, including strict #XF | Dedicated interactive image | Separate ordinary startup from test and failure-injection workloads |

## Selected reproducible target

Use Oracle VirtualBox **7.2.22**, the official stable version listed on
2026-10-22 by `https://download.oracle.com/virtualbox/LATEST-STABLE.TXT`.
The official 7.2 User Guide was downloaded from
`https://download.oracle.com/virtualbox/7.2.22/UserManual.pdf`; its SHA-256 is
`044213c16850ea2b247247e6b6c483611f7ac99b91d17af63b94f274cbdabf2a`.
Relevant sections are 5.8, 5.13.1, 5.14.1, 5.22, 8.1, 9.2, and 16.1.5.
That guide documents EFI GOP resolution selection, PS/2 input, AHCI, and
VirtualBox's paravirtualized VirtIO network card. It labels VirtIO-SCSI an
experimental feature, so the target profile uses AHCI and requires a distinct
AHCI implementation; it does not treat VirtIO-SCSI as VirtIO-blk.

Initial machine target (external execution still required): x86-64 guest, one
vCPU, 1024 MiB RAM, ICH9 chipset, I/O APIC enabled, VirtualBox EFI with Secure
Boot disabled, boot from the bundled disk, 800×600×32 EFI GOP, VGA-compatible
graphics controller, PS/2 keyboard/mouse, SATA AHCI persistent disk, one
paravirtualized VirtIO-net adapter in NAT mode, and no Guest Additions. The
final `.vbox`/OVA must encode these settings. The exact VirtualBox PCI identity,
interrupt behavior, and GOP mapping must be recorded from a real run before
calling this profile supported.

## Implementation sequence

1. Preserve the reviewed Phase 15 checkpoint on its existing branch; use this
   dedicated branch for compatibility work.
2. Add the Proposed architecture decision and the separate interactive build
   profile while retaining the existing strict qualification build.
3. Implement userspace AHCI block I/O, capability-mediated PS/2 input, and an
   explicit fail-closed entropy fallback. Preserve the existing service APIs.
4. Build the interactive EFI and persistent-disk package, then validate the
   package structure and host-side invariants.
5. Run the exact imported appliance in VirtualBox 7.2.22 when a host with the
   official VirtualBox runtime and kernel backend is available. Until then,
   label the package ready for external testing, not verified support.

## Host execution limitation

The current managed Linux environment has no `VBoxManage`, `/dev/vboxdrv`, or
`/dev/kvm`, and the current user cannot install or load the VirtualBox host
kernel module. Source builds, package inspection, and QEMU regression tests do
not establish VirtualBox guest compatibility. Keep VirtualBox boot, desktop,
input, storage persistence, network, lifecycle, and 100/100 fresh-boot evidence
explicitly unexecuted until tested on an actual supported VirtualBox host.
