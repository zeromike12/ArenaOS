# ADR-0114: VirtualBox interactive guest profile

Status: Proposed
Date: 2026-10-22
Milestone context: VirtualBox compatibility work, based on Phase 15

## Problem

ArenaOS's bootable image is built around QEMU's modern VirtIO device set and
automatically runs diagnostic and qualification workloads before reaching the
interactive desktop. A VirtualBox user needs a normal UEFI boot with supported
storage, network, input, and display devices. A disk conversion or firmware
boot alone does not provide those drivers or an interactive startup path.

## Options considered

1. Keep the QEMU VirtIO profile and assume VirtualBox exposes identical PCI
   devices and features. This is not established by either source inspection or
   the current tests.
2. Use VirtualBox's VirtIO-SCSI feature as if it were VirtIO-blk. These are
   distinct device protocols, and Oracle's 7.2 guide labels VirtIO-SCSI
   experimental.
3. Keep the existing userspace driver boundaries and add a specific profile:
   AHCI for persistent storage, capability-mediated PS/2 input, existing
   VirtIO-net only after its PCI/MSI-X behavior is verified, GOP for display,
   and a checked CPU entropy source when present. Build an interactive path
   separately from qualification routines.

## Decision

Propose option 3. Target VirtualBox 7.2.22 with an ICH9 chipset, one vCPU,
1024 MiB RAM, UEFI with Secure Boot off, 800×600 EFI GOP, VGA-compatible
graphics, PS/2 input, AHCI SATA storage, and one VirtIO-net adapter on NAT.
The ordinary boot image will have its own identity and will not execute strict
#XF qualification or test-only fault injection. The strict qualification
profile, assertions, and Phase 15 status remain unchanged.

Userspace services continue to own device protocol handling. Kernel changes
remain limited to device discovery, exact device and I/O capability creation,
DMA authorization, and interrupt routing. The existing filesystem, network
stack, signed-package path, capability model, process isolation, and W^X rules
remain in place.

## Entropy contract

Where no virtual RNG device exists, an interactive entropy service may use
RDRAND only when CPUID advertises it and each instruction reports success
within a fixed retry bound. The service must check for a failed/repeated output
health condition and must submit seed bytes through the existing write-only,
non-copyable kernel entropy capability. The source is trusted only to the
extent the host CPU and hypervisor correctly implement RDRAND and provide
unpredictable state. Missing or unhealthy RDRAND keeps entropy unavailable;
the OS must not substitute time, counters, fixed seeds, or values it can
predict. This fallback does not alter the existing entropy qualification
claims.

## Downsides accepted

The initial VirtualBox profile depends on a new AHCI driver and on the actual
VirtualBox implementation of VirtIO-net; both require guest execution before
support can be claimed. PS/2 relative pointer input is less convenient than an
absolute USB tablet. The profile does not use VBoxSVGA/VMSVGA acceleration,
Guest Additions, shared folders, clipboard integration, or 3D acceleration.

## Future implications

If VirtualBox's VirtIO-net transport lacks a feature required by ArenaOS,
record the exact guest evidence and add a device-specific userspace driver
behind the existing network service contract. Do not weaken the network
service's authority, CSPRNG requirements, or Phase 15 #XF assertions. Accept
this ADR only after the imported artifact passes the documented interactive
and lifecycle tests on VirtualBox 7.2.22.
