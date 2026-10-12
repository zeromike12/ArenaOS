# Phase 15 external hardware qualification kit

This kit runs the existing strict Phase 15 guest test on Linux/QEMU/KVM or
Windows/QEMU/WHPX. It pins the strict image pair and preserves full serial
logs, QEMU stderr, QEMU's real process exit codes, backend initialization
evidence, firmware and emulator hashes, and a machine-readable receipt.

The acceptance authority remains `tools/test_m15_guest.py`. It has optional
EFI/ESP path overrides so the runner can test the pinned copies; its expected
markers, verdicts, fixtures, and two CPU profiles are unchanged. The runner
does not replace `SQRTSS` or accept TCG as hardware evidence. Both existing
profiles (`max,+nx,+smep,+smap` and `qemu64,+nx,+smep,+smap`) must pass.

## Pinned strict image

| File | SHA-256 |
|---|---|
| `artifacts/arena-boot.efi` | `bbbc63df271c3d7d5109789369f72e093b02ef77c77ebe25273b6da4c00d238e` |
| `artifacts/arena-esp.img` | `33dd1576ab597a7d0b3e9a9100bc618e9b251a7a645e77e79d8852a404c40a6a` |
| `artifacts/arena-phase15-xf-probe` | `1f057068b020baefe06f7f3b36bc0beeafcb879bd93d60c5ea66a3cce616c8d3` |

The ESP must contain the byte-identical EFI above. The runner checks both
hashes and that embedded copy before starting QEMU. If the input differs, it
refuses to boot it.

## Requirements

- x86-64 host with x87, SSE, SSE2, and AVX exposed to the first pinned CPU profile.
- Python 3.10 or newer and `pyfatfs==1.1.0` plus `setuptools==80.10.2` from
  PyPI (the pinned setuptools release retains the `pkg_resources` shim used by
  PyFilesystem).
- Official QEMU build with the requested accelerator.
- Matching x86-64 OVMF/EDK2 code firmware and a writable-vars template.
- Linux: working `/dev/kvm` and KVM enabled by the host administrator.
- Windows: QEMU built with WHPX and Windows Hypervisor Platform available.

Use QEMU and firmware packages from the host operating system's official
package repository, or sources linked by [QEMU's download page](https://www.qemu.org/download/)
and the [TianoCore EDK II project](https://github.com/tianocore/edk2).
The runner does not download or install host software. It never changes BIOS,
Hyper-V, WHPX, Secure Boot, drivers, or security settings. Enabling host
virtualization may require an administrator and a reboot; make that change
manually only if you choose to.

Linux QEMU/KVM is the primary technically suitable target: `-accel kvm` runs
the guest on the host's virtualization hardware, and the runner confirms KVM
is active through QMP. It still qualifies only if the exact strict guest
receives vector 19. Windows QEMU/WHPX is a candidate execution target; a
successful WHPX startup does not establish SIMD exception behavior, so the
same strict guest result is required there. No alternate emulator is credited
without a verifiable vector-19 guest trial.

## Prepare the kit

The self-contained `phase15-external-qualification-kit.tar.gz` is the
transfer package. Extract it on the host that will run qualification; the
destination does not need the ArenaOS Rust toolchain or a configured build
workspace:

```sh
mkdir phase15-external
tar -xzf phase15-external-qualification-kit.tar.gz -C phase15-external
cd phase15-external
```

The archive contains the exact strict artifacts. It contains no QEMU or UEFI
firmware; those come from the host's official package source. To assemble the
same kit again from a repository checkout with the original pinned artifacts,
run:

```sh
python3 tools/phase15_external/make_kit.py \
  --efi build/phase15-strict-efi-6586a0a.efi \
  --esp build/phase15-strict-esp-6586a0a.img \
  --xf-probe userspace/phase15-fpu/target/x86_64-unknown-none/release/arena-phase15-xf-probe \
  --output build/phase15-external-qualification-kit.tar.gz
```

Extract the kit to a writable local directory. It contains the exact strict
image pair, the standalone CPL3 `SQRTSS` fixture and its source, the minimal
existing test harness, runners, requirements, and source identity. It does
not contain firmware or QEMU. Install Python requirements in a virtual
environment using the official Python package index:

```sh
python3 -m venv .phase15-venv
. .phase15-venv/bin/activate
python -m pip install -r tools/phase15_external/requirements.txt
```

On Windows PowerShell, use `py -3 -m venv .phase15-venv`, activate with
`.\.phase15-venv\Scripts\Activate.ps1`, then run `python -m pip install -r
tools/phase15_external/requirements.txt`.

## Linux with KVM

From the extracted kit root:

```sh
tools/phase15_external/run_linux.sh \
  --ovmf-code /usr/share/OVMF/OVMF_CODE.fd \
  --ovmf-vars /usr/share/OVMF/OVMF_VARS.fd
```

Pass `--qemu /path/to/qemu-system-x86_64` if it is not on `PATH`. The runner
checks `/dev/kvm`, verifies QEMU advertises KVM, initializes a q35 guest with
`-accel kvm`, confirms QMP reports KVM enabled, then runs the strict guest
test. `-accel kvm` is explicit, so QEMU cannot silently fall back to TCG.

## Windows with WHPX

Open PowerShell 7 in the extracted kit root and run:

```powershell
.\tools\phase15_external\run_windows.ps1 `
  -OvmfCode 'C:\QEMU\share\edk2-x86_64-code.fd' `
  -OvmfVars 'C:\QEMU\share\edk2-i386-vars.fd'
```

Pass `-Qemu 'C:\path\to\qemu-system-x86_64.exe'` when QEMU is not on `PATH`.
The script checks QEMU's WHPX accelerator list and performs an actual
accelerator initialization probe before boot. It never toggles Windows
features. If QEMU or WHPX cannot initialize, it writes a BLOCKED receipt and
does not start the guest.

## Results and independent receipt check

Each invocation creates a new `build/phase15-external-<UTC timestamp>/`
directory containing the exact tested EFI/ESP, `receipt.json`, the complete
serial logs for both CPU profiles, QEMU stderr logs, and accelerator-probe
output. Run:

```sh
python tools/phase15_external/verify_receipt.py \
  --receipt build/phase15-external-<UTC timestamp>/receipt.json
```

The verifier recomputes image, firmware, emulator, source-fixture, and log
hashes; confirms both QEMU exits were zero; and checks each guest log for the
real CPL3 vector-19 path, `fault=0x113`, sibling and peer survival, scheduler
slot reclamation, restored resource baselines, all historical result lines,
and clean UEFI shutdown. A failing test produces a failing receipt.

WHPX remains a candidate until this strict guest actually passes there. A
successful QEMU startup alone is not a `#XF` pass. Linux TCG, diagnostic
images, host `SIGFPE`, synthetic interrupts, and log-only claims are not
accepted. The kit does not provide official T2/T3/T4 qualification or a final
release archive.
