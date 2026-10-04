# Prebuilt desktop-maturity image (development build)

Boot it with `bash tools/run-desktop.sh` from the repository root (add
`--fresh` for a new empty disk). That needs only QEMU
(`qemu-system-x86_64`) and Python 3; no Rust toolchain. The launcher
verifies every file below against `SHA256SUMS` before booting.

| File | Contents |
|---|---|
| `arena-esp.img.gz` | 8 MiB FAT16 EFI system partition: `EFI/BOOT/BOOTX64.EFI` (the ArenaOS kernel with all userspace services and the six desktop applications embedded) |
| `edk2-x86_64-code.fd.gz` | EDK2/OVMF UEFI firmware code, from the QEMU 11.0.2 distribution pinned in `docs/DEV-ENV.md` (BSD-2-Clause-Patent); unchanged from the design checkpoint |
| `edk2-i386-vars.fd.gz` | matching empty UEFI variable store template |
| `boot-check.png` | this image booted through `run-desktop.sh --fresh`: Terminal launched from the dock with a tablet click, `help` typed |

Provenance:

* Source: branch `arena/phase10-desktop-maturity`, built at commit
  `c0ba2db`, whose code is identical to `1231e5a` (later commits change
  only docs and this directory).
* Build: `tools/build.sh --image` with the production desktop profile
  (`ARENA_GRAPHICS_FIXTURE` and `ARENA_PERF` unset;
  `build/graphics-profile.txt` = `desktop`), pinned Rust 1.97.0. No
  profiling probes are compiled in.
* `BOOTX64.EFI` SHA-256:
  `b2d429ab53d8156e96312c1a3fa361804a22de00a31e310d57ca30122137192a`;
  uncompressed ESP SHA-256:
  `3ae99a487041081c1c4b2aa3abe5c76b4e8b6378ba25b817b5decf19e51b3108`.
  The linker embeds a PE timestamp, so a rebuild of the same source gives
  a different hash; `--build` boots your own build instead.
* Evidence for these exact bytes: `tools/stability_loop.sh 100` →
  100/100 boots fully green, zero failures. The complete historical suite
  passed 97/97 on clean source `1231e5a` (receipts in
  `docs/phase10/DESKTOP-MATURITY.md` §8).

This is a development image for viewing and manual testing, **not** a
qualified Phase-10 release. The previous Opus design image remains in
`releases/checkpoints/phase10-opus-design/prebuilt/` for comparison. The
user disk (`build/interactive/disk.img`) is created by the launcher as a
freshly formatted AFS1 volume and is never part of this directory.
