# ArenaOS Phase 15 local qualification recovery

Phase 15 remains **PARTIAL / UNQUALIFIED**. The local historical diagnostic
regression now passes 120/120 in one uninterrupted run, and fresh diagnostic
T2/T3/T4 stability runs pass. Strict SIMD `#XF` delivery still fails under
QEMU TCG, KVM is unavailable in this host, and Bochs did not reach ArenaOS.
ADR-0113 remains **Proposed**. No PR, merge, release, or Phase 16 work was
created.

## Runtime restored locally

On 2026-10-21 I restored the runtime under the ignored
`build/phase15-local-qualification/` directory. I used official Debian packages
from the signed `deb.debian.org` trixie and sid repositories and the official
Rust distribution bootstrap, whose SHA-256 was verified before use. QEMU,
OVMF, Rust, and Bochs were extracted locally; no system packages were
installed, and no host security settings were changed.

- QEMU **11.1.2** (`qemu-system-x86 1:11.1.2+ds-3`), with OVMF
  **2026.08+ds-2**.
- Comparison QEMU **10.0.13** and OVMF **2025.02-8+deb13u1**.
- Rust **1.97.0**, Cargo **1.97.0**, LLVM **22.1.6**; targets include
  `x86_64-unknown-none` and `x86_64-unknown-uefi`.
- Bochs **3.1+dfsg-1** for an independent emulator trial.
- Existing project Python dependencies were reused from
  `build/onboarding/python`.

QEMU 11.1.2 advertises TCG, MSHV, Nitro, and KVM. `/dev/kvm` is absent, and an
explicit `-accel kvm` initialization exits 1 with “Could not access KVM kernel
module: No such file or directory.” Every guest test in this recovery used
`-accel tcg`; the separate Bochs trial used Bochs 3.1. No hardware
acceleration is claimed.

## Qualification results

| Gate | Result | Details |
|---|---|---|
| Historical diagnostic regression | **PASS 120/120** | One uninterrupted run on source commit `989646771a0e288cefdaf0dabc5f5baa9bf1ed5c`; strict `#XF` acceptance is intentionally excluded. |
| Strict `#XF` test | **FAIL** | QEMU 11.1.2 TCG, both `max` and `qemu64` CPU profiles. The guest sets MXCSR to `0x1F00`, `SQRTSS` returns with `0x1F01`, and vector 19 is not delivered; expected status `0x113`. |
| Diagnostic T2 | **PASS 10/10** | 95 seconds; zero failed boots. |
| Diagnostic T3 | **PASS 25/25** | 237 seconds; zero failed boots. |
| Diagnostic T4 | **PASS 100/100** | 952 seconds; zero failed boots, retries, or timeouts. |
| Bochs 3.1 trial | **NO `#XF` RESULT** | Tiger Lake and Core 2 profiles page-fault in OVMF before an ArenaOS marker. |

T2/T3/T4 used one fresh Phase 15 diagnostic image and the QEMU CPU profile
`qemu64,+nx,+smep,+smap`. That image explicitly says `XF NOT RUN`. These are
useful stability results, but they do not close the official Phase 15 gates.
The strict test records that the sibling and peer survive, the fault slot is
released, resources return to baseline, and the guest shuts down cleanly; none
of those results replaces the missing vector-19 delivery.

The diagnostic EFI SHA-256 is
`e85e0b5dd9e96f3129e7f2385d548084c56c16801432a9d41243b9ebc23a2282`; its fresh
ESP SHA-256 is
`e2a06c116fc51a90ffb70a9e14a1e0f714d6bafc10900266782cb1d4e61cbb3f`. The
strict EFI SHA-256 is
`bbbc63df271c3d7d5109789369f72e093b02ef77c77ebe25273b6da4c00d238e`; its
fresh ESP SHA-256 is
`631cc6452ecbacf58b2418e54b9c509bb717cc2211580ebef63f9aa4c13d866f`.

## Complete receipts

The machine-readable result is
[`phase15-local-recovery.json`](receipts/phase15-local-recovery.json). The
compressed raw evidence bundle is
[`phase15-local-recovery-2026-10-21.tar.gz`](receipts/phase15-local-recovery-2026-10-21.tar.gz),
SHA-256
`fb1ff147b35373be554f0056930d9c7ddeb7ea00877d4a682ee19b17ec2a7033`.
It contains the 120-suite aggregate and 2,000 per-test artifacts, strict TCG
serial and stderr receipts for both CPU profiles, exact tested images, all
T2/T3/T4 per-boot pixel/lifecycle and DNS receipts, the KVM probe, and both
Bochs trial logs and the QEMU/toolchain version probes. Its internal
`SHA256SUMS` manifest lists 2,368 files, all verified after extraction.

## External execution handoff

A self-contained strict-runner kit is available from the
[external kit handoff](EXTERNAL-KIT.md). It pins the recovery strict EFI and
ESP, includes their fixture and exact OVMF code/vars, and is downloadable as a
committed archive with its SHA-256 recorded there. A fresh extraction
verified all manifest entries and passed five host-only negative controls.
The current execution environment has no QEMU binary or `/dev/kvm`; the
external runner correctly returned BLOCKED without starting a guest. The
Windows/PowerShell path remains unexecuted here.

## Gate decision

Keep Phase 15 **PARTIAL / UNQUALIFIED** and ADR-0113 Proposed. This environment
cannot satisfy the remaining strict gate: QEMU TCG fails the vector-19 check,
`/dev/kvm` is unavailable, and Bochs 3.1 did not boot ArenaOS. A strict guest
run on a host with a successfully initialized hardware accelerator, or on an
independently functioning emulator, is now required. No exception-delivery
workaround or synthetic interrupt was added.

Arena 1 SDL3 and Arena 2 runtime integration remain outside the tested scope.
See the [qualification record](QUALIFICATION.md),
[CPU contract](CPU-CONTRACT.md), and
[thread-state isolation notes](THREAD-STATE-ISOLATION.md).
