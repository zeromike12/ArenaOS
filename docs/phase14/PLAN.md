# ArenaOS Phase 14 plan

## Goal

Add one bounded static ELF64 `ET_DYN` profile to the production native Image
validator and launch path. Apply `R_X86_64_RELATIVE` relocations in the kernel,
choose each image location from genuine kernel-seeded entropy, and carry that
launch's actual base and entry through Startup ABI v2. Keep the verified APB1
path and fixed-address `ET_EXEC` contract intact. This phase does not add a
dynamic linker, shared libraries, symbol resolution, or Linux compatibility.

The scope is fixed by [ADR-0110](../adr/0110-static-native-pie-and-kernel-placement.md).
No loader or placement implementation begins before that contract is accepted.

## Milestones

| Milestone | Work | Exit evidence |
| --- | --- | --- |
| 14.0 | Audit and bounded ELF, relocation, placement, entropy, and rollback contract | ADR-0110 plus this plan and the progress log |
| 14.1 | Reproducible Rust native PIE fixture | Linker-produced `ET_DYN`, recorded program headers and relocation table, observable runtime result |
| 14.2 | Extend the production validator | Valid fixture accepted; malformed headers, dynamic tags, relocations, and W^X/page layouts rejected by production code |
| 14.3 | Private mapping, relocation, final protections, startup publication, rollback | Actual entry/base and relocated value verified; no RWX; refusal restores process, Image, cap, and mapping counts |
| 14.4 | Kernel placement entropy | RNG readiness gating, measured placement choices, exhaustion/unavailable-entropy refusal, no app-controlled address |
| 14.5 | Image and Startup ABI v2 integration | One Image launches repeatedly at independent bases through the exact verified Image-cap path |
| 14.6 | Signed installed application proof | Positive and negative APB1 guest cases, launcher discovery, execution, exit, and teardown |
| T2/T3 | Preservation checkpoints | Affected historical suite, 10 clean boots, then broader regressions and 20–25 boots after address-space integration |
| T4 | Freeze and release | Full historical suite, Phase-14 proofs, exact-artifact fresh 100/100 boots, release archive and independently extracted boot |

## Regression dependency map

| Changed subsystem | First regressions |
| --- | --- |
| ELF parser / Image registration | M4 parser and mutation suite; M8.5 Image caps and lifecycle; M12 package authority |
| Spawn / image mapping / protections | M4 spawn; M8.5; M9 process memory and resource accounting; M13 VM, heap, threads, synchronization, mixed workload |
| Startup ABI fields / launch handoff | ABI-v2 startup tests; M10/M11 Desktop lifecycle; M12 installed launch; M13 installed applications |
| Kernel RNG interface / rngd startup | RNG device and rngd tests; boot/service-manager readiness; fixed `ET_EXEC` boot services |
| APB1 positive and negative launch | M12 exact package path; M13 Image pin/refcount and installed-app lifecycle |
| Release boot profile / packaging | OVMF/QEMU artifact boot; extracted witness; Windows manual command |

## Qualification policy

Use staged testing: T0 targeted formatting/check and parser mutations; T1 real
QEMU PIE and failure-path guests; T2 affected preservation regressions plus 10
clean boots; T3 broader regression plus 20–25 boots after loader/address-layout
contracts change; T4 full historical suite and fresh exact-artifact 100/100
stability. Investigate a failure's mechanism even if a later run passes. An EFI
change invalidates stability evidence for the prior EFI. Documentation-only or
host-tool-only changes require only affected evidence to be refreshed.

The final boot profile will include UEFI/OVMF, virtio block, virtio-net with
QEMU user networking, virtio RNG, virtio keyboard, virtio tablet, and virtio
console when the selected test needs it. The release report will give a Windows
QEMU command that uses a distinct persistent-disk path.
