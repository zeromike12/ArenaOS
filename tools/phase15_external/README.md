# ArenaOS Phase 15 strict external qualification kit

This portable kit runs the existing strict Phase 15 guest test on an x86-64
Linux/KVM or Windows/WHPX host. It contains the reviewed strict EFI and ESP,
the compiled CPL3 `SQRTSS` fixture, the runner and verifier, required test
fixtures, and the exact OVMF code and vars firmware. It does not contain QEMU;
install QEMU from the host OS's official package source or the official QEMU
download page.

The strict acceptance test remains `tools/test_m15_guest.py`. Its assertions,
CPU profiles, and unmasked invalid-operation `SQRTSS` probe are unchanged. The
runner explicitly selects KVM or WHPX, initializes it before guest startup,
and records QEMU's process exit separately from the guest's semantic result.
The kit does not credit TCG, diagnostic images, host exceptions, or synthetic
interrupts as qualification.

## Pinned artifacts

| File | SHA-256 |
|---|---|
| `artifacts/arena-boot.efi` | `bbbc63df271c3d7d5109789369f72e093b02ef77c77ebe25273b6da4c00d238e` |
| `artifacts/arena-esp.img` | `631cc6452ecbacf58b2418e54b9c509bb717cc2211580ebef63f9aa4c13d866f` |
| `artifacts/arena-phase15-xf-probe` | `1f057068b020baefe06f7f3b36bc0beeafcb879bd93d60c5ea66a3cce616c8d3` |
| `artifacts/OVMF_CODE_4M.fd` | `7e34e3b83224968bb0714c0ebc88f75c5c37b4cc892c902e511ccc7a7c107f94` |
| `artifacts/OVMF_VARS_4M.fd` | `5d2ac383371b408398accee7ec27c8c09ea5b74a0de0ceea6513388b15be5d1e` |

The ESP must embed the exact EFI above. The builder, runner, and verifier each
check the identities and the embedded EFI. The prior kit ESP identity
`33dd1576ab597a7d0b3e9a9100bc618e9b251a7a645e77e79d8852a404c40a6a` is a
superseded image and is rejected.

The bundled OVMF code and vars are from the official Debian `ovmf-generic`
2026.08+ds-2 package. `FIRMWARE-SOURCE.json` records the official package URL
and package SHA-256; the Debian copyright file is included under `licenses/`.
The files are non-Secure-Boot x86-64 4 MiB firmware. QEMU, Python, and the
Python dependencies must come from official sources as described below.

## Host requirements

- x86-64 Linux with a working `/dev/kvm`, or 64-bit Windows with WHPX available.
- QEMU's x86-64 system emulator built with the selected accelerator.
- Python 3.10 or newer.
- `pyfatfs==1.1.0` and `setuptools==80.10.2` from PyPI.
- A host CPU exposing x87, SSE, SSE2, and AVX for the existing `max` profile.

The ArenaOS kernel continues to use eager per-thread FXSAVE64/FXRSTOR64 and
the existing x87/MMX/SSE state profile. OSXSAVE stays off and AVX state stays
unsupported in the guest. The AVX-capable CPU profile is an existing strict
test input; it is not an ArenaOS feature enablement or exception workaround.

The runner does not install host software or change BIOS, Hyper-V, WHPX,
drivers, Secure Boot, or security settings. Do not change host security or
virtualization settings to make a run proceed. A missing or unusable
accelerator produces a BLOCKED receipt and starts no guest.

## Download and verify

Download the committed `phase15-external-qualification-kit.tar.gz` from the
Phase 15 artifact link recorded in `docs/phase15/EXTERNAL-KIT.md`. Verify its
SHA-256 against that document before extraction. The archive contains
`KIT-MANIFEST.json`; the runner verifies every packaged path, size, and hash
before it will start QEMU.

Extract into a writable directory and change into it:

```sh
mkdir phase15-external
tar -xzf phase15-external-qualification-kit.tar.gz -C phase15-external
cd phase15-external
```

Install the pinned Python dependencies from the official PyPI index in a
virtual environment:

```sh
python3 -m venv .phase15-venv
. .phase15-venv/bin/activate
python -m pip install -r tools/phase15_external/requirements.txt
```

On Windows PowerShell:

```powershell
py -3 -m venv .phase15-venv
.\.phase15-venv\Scripts\Activate.ps1
python -m pip install -r tools/phase15_external/requirements.txt
```

## Linux with KVM

Install QEMU from the Linux distribution's official repository. Run from the
kit root:

```sh
tools/phase15_external/run_linux.sh
```

Pass `--qemu /path/to/qemu-system-x86_64` if QEMU is not on `PATH`. The runner
checks `/dev/kvm`, QEMU's advertised backend, and an actual QMP KVM
initialization (`query-kvm` must report present and enabled). It then runs
both existing CPU profiles with explicit `-accel kvm`; it cannot fall back to
TCG.

## Windows with WHPX

Install an official QEMU build that includes WHPX and enable Windows Hypervisor
Platform only through your normal administrator-controlled process. Open
PowerShell 7 in the kit root and run:

```powershell
pwsh -NoProfile -File .\tools\phase15_external\run_windows.ps1
```

Pass `-Qemu 'C:\path\to\qemu-system-x86_64.exe'` if needed. Bundled firmware
is selected by default; optional `-OvmfCode` and `-OvmfVars` overrides are
recorded in the receipt. The script checks WHPX support, then the Python
runner initializes `-accel whpx` through QMP before guest startup. It never
enables Windows features or changes security policy. The Windows path is
source-reviewed, but has not been executed in this Linux environment.

## Receipts and verification

Every invocation creates a new `build/phase15-external-<UTC timestamp>/`
directory. It includes the exact tested EFI/ESP and firmware copies, fixture,
`receipt.json`, complete serial logs, QEMU stderr logs, accelerator-probe
transcript, runner output, source review, and the reported QEMU/firmware hashes.
The receipt separates QEMU exit status from semantic guest status and marks
firmware boot, ArenaOS boot, vector 19, fault-thread termination, sibling and
peer survival, slot reclamation, resource restoration, continued kernel
operation, and shutdown independently.

Verify the result from the kit root:

```sh
python tools/phase15_external/verify_receipt.py \
  --receipt build/phase15-external-<UTC timestamp>/receipt.json
```

The verifier recomputes hashes, opens the ESP and compares its embedded EFI,
checks the unmodified fixture source and guest logs, and requires both CPU
profiles to pass with zero QEMU and semantic exits. A QEMU process exit of zero
with a failed guest assertion remains a failed qualification. A strict guest
pass is only the vector-19 gate; official Phase 15 T2/T3/T4 stability and the
final source, protected Phase 14, ISA, and qualification-archive gates remain
separate.

## Rebuilding the package from the repository

From a checkout containing the preserved recovery artifacts, use the exact
strict pair and official Debian firmware:

```sh
python3 tools/phase15_external/make_kit.py \
  --efi build/phase15-local-qualification/final-receipts/strict-image/arena-boot.efi \
  --esp build/phase15-local-qualification/final-receipts/strict-image/arena-esp.img \
  --xf-probe userspace/phase15-fpu/target/x86_64-unknown-none/release/arena-phase15-xf-probe \
  --ovmf-code build/phase15-local-qualification/qemu-11-root/usr/share/OVMF/OVMF_CODE_4M.fd \
  --ovmf-vars build/phase15-local-qualification/qemu-11-root/usr/share/OVMF/OVMF_VARS_4M.fd \
  --output phase15-external-qualification-kit.tar.gz
```

The builder refuses a missing or wrong hash, including the superseded ESP,
and refuses to package an ESP that does not embed the exact strict EFI.
Rebuilding requires `pyfatfs==1.1.0` to perform the embedded EFI check.
