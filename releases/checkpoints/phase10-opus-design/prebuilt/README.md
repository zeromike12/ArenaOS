# Prebuilt Opus design image (development build)

Boot it with `bash tools/run-desktop.sh` from the repository root. That
needs only QEMU (`qemu-system-x86_64`) and Python 3; no Rust toolchain. The
launcher verifies every file below against `SHA256SUMS` before booting.

| File | Contents |
|---|---|
| `arena-esp.img.gz` | 8 MiB FAT16 EFI system partition: `EFI/BOOT/BOOTX64.EFI` (the ArenaOS kernel with all userspace services and the six desktop applications embedded) |
| `edk2-x86_64-code.fd.gz` | EDK2/OVMF UEFI firmware code, from the QEMU 11.0.2 distribution pinned in `docs/DEV-ENV.md` (BSD-2-Clause-Patent) |
| `edk2-i386-vars.fd.gz` | matching empty UEFI variable store template |

Provenance:

* Source: branch `arena/phase10-opus-design`, built at commit
  `ecb538431ece526f6e74ae51c6d7293edec63cef`, whose code is identical to
  design source `cf311719e08d07cf765d7799524bce7067bd8419` (later commits
  change only docs and evidence).
* Build: `tools/build.sh --image` with the production desktop profile
  (`ARENA_GRAPHICS_FIXTURE` unset; `build/graphics-profile.txt` = `desktop`),
  pinned Rust 1.97.0.
* `BOOTX64.EFI` SHA-256: `ff4b33b70b42979315b02697c372dea25622e472c375f543e43f0fbd7e0ac94b`.
  The linker embeds a PE timestamp, so a rebuild of the same source gives a
  different hash; `--build` boots your own build instead.
* Verified by booting it with `tools/run-desktop.sh` (fresh and reused disk)
  to the desktop and launching an application with a real tablet click.

This is a development image of the design branch for viewing and manual
testing. It is **not** a qualified Phase-10 release: Sol's post-design
full suite, exact-EFI 100/100 and release archive are still outstanding.
The user disk (`build/interactive/disk.img`) is created by the launcher as
a freshly formatted AFS1 volume and is never part of this directory.
