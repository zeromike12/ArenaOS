# ArenaOS v0.4.0 — QEMU run bundle

The complete, tested build for running ArenaOS in your own QEMU.
GitHub asset uploads were unreachable from the build environment, so
the release (https://github.com/zeromike12/ArenaOS/releases/tag/v0.4.0)
links this in-repo bundle instead.

## Download

```sh
curl -LO https://github.com/zeromike12/ArenaOS/raw/refs/heads/arena/01a0d6fd-arenaos/releases/v0.4.0/arenaos-v0.4.0-qemu-x86_64.tar.gz
sha256sum -c arenaos-v0.4.0-qemu-x86_64.tar.gz.sha256

# or via the GitHub API (contents endpoint caps at 1 MiB — use the blob):
gh api repos/zeromike12/ArenaOS/git/blobs/b26749a00adc63dc61dd30b1f127f710754835a3 \
    -H "Accept: application/vnd.github.raw" > arenaos-v0.4.0-qemu-x86_64.tar.gz
```

## Run

```sh
tar xzf arenaos-v0.4.0-qemu-x86_64.tar.gz
cp ovmf-vars-template.img ovmf-vars.img     # fresh NVRAM per boot
qemu-system-x86_64 \
    -M q35 -m 512M -cpu qemu64,+nx,+smep,+smap \
    -drive if=pflash,format=raw,readonly=on,file=edk2-x86_64-code.fd \
    -drive if=pflash,format=raw,file=ovmf-vars.img \
    -drive format=raw,file=arena-esp.img \
    -display none -serial mon:stdio -no-reboot
```

Serial is the console in BOTH directions: the VM runs the milestone
suite, then the kernel spawns the shell and waits at the `arena> `
prompt — type `help`, `ps`, `echo hi`, `spawn` (runs the test
payload as a child process), and `shutdown` to stop the machine.
Full details: RUNNING.md inside the tarball (same as docs/RUNNING.md).
Bundle contents: arena-esp.img (boot disk), edk2-x86_64-code.fd +
ovmf-vars-template.img (tested EDK2 firmware pair), RUNNING.md,
sha256sums.txt.
