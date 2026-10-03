# ArenaOS v0.6.0 — QEMU run bundle

The complete, tested build for running ArenaOS in your own QEMU.
GitHub asset uploads were unreachable from the build environment, so
the release (https://github.com/zeromike12/ArenaOS/releases/tag/v0.6.0)
links this in-repo bundle instead.

## Download

```sh
curl -LO https://github.com/zeromike12/ArenaOS/raw/refs/heads/arena/01a0d6fd-arenaos/releases/v0.6.0/arenaos-v0.6.0-qemu-x86_64.tar.gz
sha256sum -c arenaos-v0.6.0-qemu-x86_64.tar.gz.sha256

# or via the GitHub API (contents endpoint caps at 1 MiB — use the blob):
gh api repos/zeromike12/ArenaOS/git/blobs/ba769c6619abc972e3d924ea9ae355f79684238c \
    -H "Accept: application/vnd.github.raw" > arenaos-v0.6.0-qemu-x86_64.tar.gz
```

## Run

```sh
tar xzf arenaos-v0.6.0-qemu-x86_64.tar.gz
cp ovmf-vars-template.img ovmf-vars.img     # fresh NVRAM per boot
cp scratch-template.img scratch.img         # FIRST boot only: the formatted
                                            # AFS1 volume — then REUSE your
                                            # scratch.img: files persist
qemu-system-x86_64 \
    -M q35 -m 512M -cpu qemu64,+nx,+smep,+smap \
    -drive if=pflash,format=raw,readonly=on,file=edk2-x86_64-code.fd \
    -drive if=pflash,format=raw,file=ovmf-vars.img \
    -drive format=raw,file=arena-esp.img \
    -drive file=scratch.img,format=raw,if=none,id=scr0 \
    -device virtio-blk-pci,drive=scr0 \
    -netdev user,id=net0 \
    -device virtio-net-pci,netdev=net0 \
    -display none -serial mon:stdio -no-reboot
```

Serial is the console in BOTH directions: the VM runs the milestone
suite (including the real filesystem tests on scratch.img and the ARP
link probe over QEMU's built-in network), then the kernel spawns the
storaged + fsd + netd services and the shell, and waits at
the `arena> ` prompt — type `help`, `ls`, `write note.txt hello`,
`cat note.txt`, `rm note.txt`, `ps`, `spawn`, and `shutdown` to
stop the machine. What you `write` is committed to scratch.img and
comes back next boot. Full details: RUNNING.md inside the tarball
(same as docs/RUNNING.md). Bundle contents: arena-esp.img (boot
disk), scratch-template.img (formatted AFS1 data disk),
edk2-x86_64-code.fd + ovmf-vars-template.img (tested EDK2 firmware
pair), RUNNING.md, sha256sums.txt.
