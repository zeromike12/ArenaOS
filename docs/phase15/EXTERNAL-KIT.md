# Phase 15 external execution kit handoff

Phase 15 remains **PARTIAL / UNQUALIFIED**. This archive is a runnable strict
`#XF` handoff for a suitable external host; it is not a qualification result,
release, or approval of ADR-0113.

## Download

The package is committed at
[`phase15-external-qualification-kit.tar.gz`](artifacts/phase15-external-qualification-kit.tar.gz)
and is downloadable from the branch at:

<https://raw.githubusercontent.com/zeromike12/ArenaOS/arena/phase15-fpu-simd/docs/phase15/artifacts/phase15-external-qualification-kit.tar.gz>

Archive size: **3,816,382 bytes**

SHA-256: **`134cbc228e39508ab93909b87c6f839c59d9e89d3030aae15ce672b651bf4993`**
([sidecar](artifacts/phase15-external-qualification-kit.tar.gz.sha256))

Verify before extraction:

```sh
sha256sum phase15-external-qualification-kit.tar.gz
```

PowerShell:

```powershell
Get-FileHash .\phase15-external-qualification-kit.tar.gz -Algorithm SHA256
```

The deterministic package was built from clean source commit
`081bcd1a1cba1fc54fbf38a80f6dab2c95176c06`. Its internal
`KIT-MANIFEST.json` covers 28 files and includes source and firmware identity.
`FIRMWARE-SOURCE.json` identifies official Debian `ovmf-generic` 2026.08+ds-2
package SHA-256
`05c1593998ba58700ff86c062bde306321af1f5a2943ea9ceef6c2332f3f3374`; the
matching OVMF code, vars template, and Debian copyright text are in the archive.
QEMU is deliberately installed from the host's official OS repository or the
official QEMU download source; it is recorded by exact binary hash in every
trial receipt.

## Artifact identity

| Artifact | SHA-256 |
|---|---|
| Strict EFI | `bbbc63df271c3d7d5109789369f72e093b02ef77c77ebe25273b6da4c00d238e` |
| Matching strict ESP | `631cc6452ecbacf58b2418e54b9c509bb717cc2211580ebef63f9aa4c13d866f` |
| CPL3 unmasked `SQRTSS` fixture | `1f057068b020baefe06f7f3b36bc0beeafcb879bd93d60c5ea66a3cce616c8d3` |
| OVMF code | `7e34e3b83224968bb0714c0ebc88f75c5c37b4cc892c902e511ccc7a7c107f94` |
| OVMF vars template | `5d2ac383371b408398accee7ec27c8c09ea5b74a0de0ceea6513388b15be5d1e` |

The earlier external kit pinned ESP
`33dd1576ab597a7d0b3e9a9100bc618e9b251a7a645e77e79d8852a404c40a6a`. That
identity is historical and rejected by this kit. The runner verifies both new
image hashes and compares the ESP's embedded `EFI/BOOT/BOOTX64.EFI` byte for
byte with the pinned EFI before starting QEMU. It does not fall back to a
diagnostic image.

## Run

Extract the archive to a writable directory. Install Python 3.10+ and the
version-pinned packages in `tools/phase15_external/requirements.txt` from
the official PyPI index in a virtual environment. Install QEMU from the host's
official OS package source or QEMU's official download source.

Linux with a working `/dev/kvm`:

```sh
tools/phase15_external/run_linux.sh
```

Windows x86-64 with WHPX, from PowerShell 7:

```powershell
pwsh -NoProfile -File .\tools\phase15_external\run_windows.ps1
```

Both entry points record preflight failures and refuse to start the guest if
the requested backend cannot initialize. They select KVM or WHPX explicitly;
they do not silently fall back to TCG. Each run records both CPU profiles,
firmware and emulator identity, actual QEMU exits, semantic guest exits,
serial and stderr logs, accelerator probe output, and the strict `#XF` and
containment assertions. Run `tools/phase15_external/verify_receipt.py` on the
generated receipt and transfer the whole result directory with it.

The receipt distinguishes firmware boot, ArenaOS boot, vector-19 delivery,
fault status `0x113`, faulting-thread termination, same-process sibling
survival, independent-process survival, slot reclamation, resource restoration,
continued kernel operation, and shutdown. A QEMU exit of zero does not pass a
guest test whose semantic exit or strict assertion failed.

## Verification performed here

- Re-extracted the archive into a fresh directory outside the repository
  checkout. All 28 manifest entries verified; the EFI, ESP, and firmware
  hashes matched the pins, including the embedded EFI check.
- Ran five host-only negative controls: missing guest proof, superseded ESP
  identity, QEMU process exit 0 with semantic guest exit 97, failed QEMU
  accelerator initialization, and firmware start without ArenaOS transfer.
- Ran the real Linux runner from that fresh extraction. It returned
  **BLOCKED**, recording `/dev/kvm` absent and QEMU unresolved; it started no
  guest.
- Applied the new stage classifier to the archived QEMU 11.1.2 TCG strict
  failure. QEMU exited 0, the semantic guest result was 97, vector 19 was not
  delivered, while sibling/peer survival, slot release, resource restoration,
  and clean shutdown were recorded. It remains a failed strict trial.
- PowerShell and Windows/WHPX guest execution were not run here. Host or mock
  tests do not establish Windows guest execution.

The local environment has no QEMU binary, `/dev/kvm`, PowerShell, or Bochs.
Therefore genuine vector 19 remains **NOT DELIVERED** in this environment.
The archive does not complete the official strict T2/T3/T4 gates, final-source
build and ISA checks, protected Phase 14 identity checks, or final qualification
archive. See [qualification status](QUALIFICATION.md) for those remaining
gates and the preserved prior evidence.
