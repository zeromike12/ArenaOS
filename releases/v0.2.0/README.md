# ArenaOS v0.2.0 — QEMU run bundle (Milestone 2: Kernel Foundations)

`arenaos-v0.2.0-qemu-x86_64.tar.gz` is the complete, tested build for
running ArenaOS in your own QEMU (verified end-to-end from this exact
bundle: extract → boot → `m1: RESULT PASS (8/8)`, `m2: RESULT PASS
(21/21)`, clean self-shutdown).

> **Why a repo bundle instead of release assets?** GitHub's asset upload
> endpoint (`uploads.github.com`) is unreachable from the build
> environment, so the release
> [v0.2.0](https://github.com/zeromike12/ArenaOS/releases/tag/v0.2.0)
> links here instead. Same artifact, delivered through the repo.

## Download

```sh
# raw file (works in a browser too):
curl -LO https://github.com/zeromike12/ArenaOS/raw/refs/heads/arena/01a0d6fd-arenaos/releases/v0.2.0/arenaos-v0.2.0-qemu-x86_64.tar.gz

# or via the GitHub API (the contents endpoint caps base64 responses at
# 1 MiB — fetch the git blob raw instead; verified byte-identical):
gh api repos/zeromike12/ArenaOS/git/blobs/6e41012fa4bc0ab15c017623a72d9999342f2f67 \
    -H "Accept: application/vnd.github.raw" > arenaos-v0.2.0-qemu-x86_64.tar.gz
```

Verify (optional but recommended):

```sh
sha256sum -c arenaos-v0.2.0-qemu-x86_64.tar.gz.sha256
```

## Run

```sh
tar xzf arenaos-v0.2.0-qemu-x86_64.tar.gz
cp ovmf-vars-template.img ovmf-vars.img     # fresh NVRAM per boot
qemu-system-x86_64 \
    -M q35 -m 512M -cpu qemu64,+nx \
    -drive if=pflash,format=raw,readonly=on,file=edk2-x86_64-code.fd \
    -drive if=pflash,format=raw,file=ovmf-vars.img \
    -drive format=raw,file=arena-esp.img \
    -display none -serial mon:stdio -no-reboot
```

Serial is the console. The VM boots, runs the whole Milestone 1+2 suite
(29 machine-checked tests), prints `m2: RESULT PASS (21/21)`, and shuts
itself down. Full details, expected output, and troubleshooting:
`RUNNING.md` inside the tarball (same as
[docs/RUNNING.md](../../docs/RUNNING.md)).

## Bundle contents

| File | Size (unpacked) | What it is |
|---|---|---|
| `arena-esp.img` | 8 MiB | Bootable EFI System Partition: ArenaOS boot stage + kernel |
| `edk2-x86_64-code.fd` | 3.5 MiB | EDK2/OVMF firmware code flash (the exact tested build, BSD-2-Clause-Patent) |
| `ovmf-vars-template.img` | 528 KiB | Blank firmware NVRAM template |
| `RUNNING.md` | — | How to run, expected output, variations |
| `sha256sums.txt` | — | Checksums of the four files above |

Build: commit `a95e819` + stability fix `9419067`, QEMU 11.0.2 / EDK2
(qemu-portable), Rust 1.97.0. Test evidence: `tools/test_m1.py` 8/8,
`tools/test_m2.py` 21/21, `tools/stability_loop.sh` 100/100 clean boots.
