# Phase 15 locally modified QEMU TCG qualification

Phase 15 remains **PARTIAL / UNQUALIFIED** pending the official strict
T2/T3/T4 gates and M/C review. These results use the explicitly identified
locally modified QEMU TCG build. They are not upstream-QEMU or
hardware-backed results. ADR-0113 remains Proposed.

## Backend and acceptance image

- QEMU: official v11.1.2 source, tag commit
  `ceaea6cb3e2a7d966f9ca97c7ebb5b88cd764dd0`, source archive SHA-256
  `fc3d810fc6b27f60df77eaf6748004ca76de2edc2c4ff3a9f9a9ada33066c242`.
- Local patch SHA-256:
  `c2909ea974f4ccaa9526bd5b70aef5599971bc567dd87d6ba8d9b06b327c8dd2`.
- Patched binary SHA-256:
  `e0ea2b1f5ae2a5aa65a455eb232e46e1743b5c327c48ca678630c214c69f26ff`.
- Unpatched same-source/config control binary SHA-256:
  `443c81fc9482493297ae7a3f37300bef3a82a2873e428f72884cf52d16e27284`.
- Required accelerator: explicit `-accel tcg`. The binary advertises TCG only;
  a QMP machine initialization smoke completed successfully. KVM was disabled
  in the build and `/dev/kvm` is absent.
- Strict EFI SHA-256:
  `bbbc63df271c3d7d5109789369f72e093b02ef77c77ebe25273b6da4c00d238e`.
- Exact matching ESP SHA-256:
  `631cc6452ecbacf58b2418e54b9c509bb717cc2211580ebef63f9aa4c13d866f`.
- OVMF Code SHA-256:
  `7e34e3b83224968bb0714c0ebc88f75c5c37b4cc892c902e511ccc7a7c107f94`.
- OVMF Vars template SHA-256:
  `5d2ac383371b408398accee7ec27c8c09ea5b74a0de0ceea6513388b15be5d1e`.
- The strict EFI/ESP were built from ArenaOS commit
  `13986b3dcf007b1721d85ef3509945131f8c6584`. The intervening changes through
  the last reviewed commit `df4ad9d4622e2e7e7d2b1ce03a57048305dd902a` were
  documentation-only; the kernel and strict guest sources were unchanged.

The full pin set is in [`patched-tcg-qualification.json`](patched-tcg-qualification.json).
The emulator source patch and independent tests are described in the
[prototype evidence record](experiments/qemu-tcg-xm-prototype/README.md).

## Download

The runnable overlay archive will be published at:

<https://raw.githubusercontent.com/zeromike12/ArenaOS/arena/phase15-fpu-simd/docs/phase15/artifacts/phase15-patched-tcg-qualification.tar.gz>

Verify the SHA-256 recorded in
[`phase15-patched-tcg-qualification.tar.gz.sha256`](artifacts/phase15-patched-tcg-qualification.tar.gz.sha256)
before extracting. The archive contains the pinned emulator binaries and their
runtime libraries, ROMs, exact strict EFI/ESP, OVMF files, control fixtures,
runner and verifier, raw strict receipts, source patch, reproducible build
procedure, and final qualification evidence.

The archive is an overlay for the exact ArenaOS source commit named inside the
archive. Clone that commit, extract the archive at the repository root, and
follow its `README.md`. It does not replace the hardware-only external kit in
[`EXTERNAL-KIT.md`](EXTERNAL-KIT.md).

## Run from the overlaid source checkout

Use Python 3.10+ and install the pinned Python dependencies from official
PyPI:

```sh
python3 -m venv .venv-phase15
. .venv-phase15/bin/activate
python -m pip install -r tools/phase15_external/requirements.txt
```

On Debian 13 the package supplies the pinned GLib, Pixman, and libslirp runtime
libraries. The host needs x86-64 glibc 2.38 or newer and the system
`libatomic`, `libpcre2-8`, zlib, and x86-64 dynamic loader. The runner verifies
the bundled library hashes, strict EFI/ESP/firmware hashes, and required
Python dependency version. Do not substitute a QEMU executable with a
different SHA-256.

Set paths to the extracted overlay's `build/phase15-patched-tcg` directory:

```sh
export PHASE15_QEMU_ROOT="$PWD/build/phase15-patched-tcg"
export ARENA_QEMU="env LD_LIBRARY_PATH=$PHASE15_QEMU_ROOT/runtime/lib $PHASE15_QEMU_ROOT/qemu-system-x86_64-patched -accel tcg -L $PHASE15_QEMU_ROOT/pc-bios"
export ARENA_OVMF_CODE="$PHASE15_QEMU_ROOT/firmware/OVMF_CODE_4M.fd"
export ARENA_OVMF_VARS="$PHASE15_QEMU_ROOT/firmware/OVMF_VARS_4M.fd"
export ARENA_PHASE15_EFI="$PHASE15_QEMU_ROOT/artifacts/arena-boot.efi"
export ARENA_PHASE15_ESP="$PHASE15_QEMU_ROOT/artifacts/arena-esp.img"
```

Run and verify the strict two-profile trial:

```sh
python3 tools/phase15_patched_tcg_qualification.py preflight \
  --qemu-binary "$PHASE15_QEMU_ROOT/qemu-system-x86_64-patched" \
  --lib-dir "$PHASE15_QEMU_ROOT/runtime/lib" --rom-dir "$PHASE15_QEMU_ROOT/pc-bios" \
  --efi "$ARENA_PHASE15_EFI" --esp "$ARENA_PHASE15_ESP" \
  --ovmf-code "$ARENA_OVMF_CODE" --ovmf-vars "$ARENA_OVMF_VARS"
python3 tools/phase15_patched_tcg_qualification.py strict \
  --qemu-binary "$PHASE15_QEMU_ROOT/qemu-system-x86_64-patched" \
  --lib-dir "$PHASE15_QEMU_ROOT/runtime/lib" --rom-dir "$PHASE15_QEMU_ROOT/pc-bios" \
  --efi "$ARENA_PHASE15_EFI" --esp "$ARENA_PHASE15_ESP" \
  --ovmf-code "$ARENA_OVMF_CODE" --ovmf-vars "$ARENA_OVMF_VARS" \
  --receipts "$PHASE15_QEMU_ROOT/runs/strict"
python3 tools/phase15_patched_tcg_qualification.py verify \
  "$PHASE15_QEMU_ROOT/runs/strict"
```

The preflight checks all image, firmware, emulator, and packaged runtime
library hashes, the embedded EFI, the exact advertised accelerator set, and
an actual QMP initialization with explicit TCG. A semantic guest failure
remains a failure even if QEMU exits 0.

## Official strict stability

The repository's established stability requirements use
`qemu64,+nx,+smep,+smap` for fresh T2/T3/T4 boots. The two-profile direct
strict test above separately covers both `max,+nx,+smep,+smap` and
`qemu64,+nx,+smep,+smap`. The strict stability mode of
`tools/stability_loop.sh` additionally requires Phase 15 PASS 6/6,
the `xf_delivery_probe` PASS marker, vector `0x13`, fault status `0x113`,
both survivor receipts, slot/resource recovery, all existing guest verdicts,
and clean shutdown on every boot. Boot 1's general suite expectations cannot
substitute for these Phase 15 requirements. It records every attempt and does
not retry a failed boot.

Run all official strict tiers in order:

```sh
python3 tools/phase15_patched_tcg_qualification.py stability all \
  --qemu-binary "$PHASE15_QEMU_ROOT/qemu-system-x86_64-patched" \
  --lib-dir "$PHASE15_QEMU_ROOT/runtime/lib" --rom-dir "$PHASE15_QEMU_ROOT/pc-bios" \
  --efi "$ARENA_PHASE15_EFI" --esp "$ARENA_PHASE15_ESP" \
  --ovmf-code "$ARENA_OVMF_CODE" --ovmf-vars "$ARENA_OVMF_VARS" \
  --receipts "$PHASE15_QEMU_ROOT/runs/strict-stability"
```

Required boot counts: T2 **10**, T3 **25**, and T4 **100**, all fresh and
successful with the same pinned artifacts. Verify each tier using the
`verify` action. Preserve all logs and receipts, including any failed or
timed-out attempt. Never describe diagnostic stability as strict stability.

## Build the backend again

`tools/build_phase15_patched_qemu.sh patched /fresh/output` downloads
QEMU only from the official QEMU GitLab source archive, verifies its pinned
SHA-256, and applies the committed patch outside ArenaOS. It configures
`x86_64-softmmu` with TCG enabled and KVM disabled. A clean build completed
with TCG only and binary SHA-256
`2ff8e8896de96cbba3fc368a39e45e4394ff174495d45cbae2452ca192941450`; it was
not used for qualification because its hash differs from the selected pinned
binary. The qualification binary is included in the package. A different
binary hash requires new validation and qualification before substitution.

## Results and review state

The direct strict test and expanded independent controls passed on the pinned
modified-TCG backend. The same-source unpatched binary failed the unchanged
strict test 5/6 on both CPU profiles, with QEMU exit 0 and semantic exit 97.
The updated registered regression suite, strict T2/T3/T4 results, final
independent archive verification, and review state are recorded in the
package manifest after those gates finish.

M is the architecture and acceptance decision maker; C reviews the evidence.
This record does not accept ADR-0113, merge a branch, or release the milestone.
