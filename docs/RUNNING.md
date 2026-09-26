# Running ArenaOS in your own QEMU

Every completed milestone ships as a **GitHub release** containing a
prebuilt, tested boot image plus the exact UEFI firmware pair it was
verified against. This page explains how to boot it on your own machine.

## What you need

* Any x86-64 host (Linux, macOS, Windows/WSL) with
  `qemu-system-x86_64` **8.0 or newer** installed (developed and tested
  against QEMU 11.0.2, TCG emulation — no KVM required).
  * Debian/Ubuntu: `sudo apt install qemu-system-x86`
  * Fedora: `sudo dnf install qemu-system-x86-core`
  * Arch: `sudo pacman -S qemu-full`
  * macOS: `brew install qemu`
* The release artifacts. Normally they are attached to the GitHub
  release itself:

```sh
gh release download v0.2.0 --repo zeromike12/ArenaOS
```

  The build environment cannot reach GitHub's asset-upload endpoint
  (`uploads.github.com`), so releases additionally ship the **identical
  bundle through the repository** under `releases/<tag>/` — download the
  tarball, check its sha256, extract, and you have the same five files:

```sh
curl -LO https://github.com/zeromike12/ArenaOS/raw/refs/heads/arena/01a0d6fd-arenaos/releases/v0.2.0/arenaos-v0.2.0-qemu-x86_64.tar.gz
sha256sum -c arenaos-v0.2.0-qemu-x86_64.tar.gz.sha256
tar xzf arenaos-v0.2.0-qemu-x86_64.tar.gz
```

  (Each release's notes link its own bundle; after a branch merge the
  same path works under `raw/refs/heads/main/...`.)

| Asset | What it is |
|---|---|
| `arena-esp.img` | The bootable disk: an EFI System Partition holding the ArenaOS boot stage + kernel (a UEFI application, ADR-0003) |
| `edk2-x86_64-code.fd` | EDK2/OVMF firmware **code** flash (read-only), the exact build the release was tested with |
| `ovmf-vars-template.img` | Blank firmware **NVRAM** template (writable copy required per boot) |
| `RUNNING.md` | This file |
| `sha256sums.txt` | Checksums for all of the above |

> EDK2 firmware is redistributed under its BSD-2-Clause-Patent license;
> QEMU itself is **not** redistributed — install it from your platform's
> package manager.

## Boot it

The vars flash is written by the firmware, so always boot from a **fresh
copy** of the template:

```sh
cp ovmf-vars-template.img ovmf-vars.img

qemu-system-x86_64 \
    -M q35 -m 512M -cpu qemu64,+nx,+smep,+smap \
    -drive if=pflash,format=raw,readonly=on,file=edk2-x86_64-code.fd \
    -drive if=pflash,format=raw,file=ovmf-vars.img \
    -drive format=raw,file=arena-esp.img \
    -display none -serial mon:stdio -no-reboot
```

Serial is the console: everything ArenaOS logs goes there. The VM
**shuts itself down** when the milestone test suite finishes (typically
a few seconds) — that is the expected, clean ending, not a crash.

### Using your distro's OVMF instead

The bundled firmware is optional — any recent OVMF/EDK2 x86-64 split
(code + vars) works. Point the two `pflash` drives at your distro's
files instead, e.g. on Debian/Ubuntu:

```sh
    -drive if=pflash,format=raw,readonly=on,file=/usr/share/OVMF/OVMF_CODE.fd \
    -drive if=pflash,format=raw,file=/usr/share/OVMF/OVMF_VARS.fd \
```

(copy `OVMF_VARS.fd` to a writable location first, as above).

## What a healthy boot looks like (current: Milestone 4 in progress)

The serial output is a boot stage log followed by kernel log lines. The
machine-checkable landmarks, in order:

1. Boot stage banner, memory-map capture, page-table install (ADR-0008)
2. `m1: RESULT PASS (8/8)` — Milestone 1 self-tests re-run every boot
3. `ebs_exited` — ExitBootServices survived (ADR-0011)
4. `timer chain reclaimed: ... ioapic pin 2 ...` — the kernel took the
   PIT→IOAPIC→LAPIC chain back from firmware
5. 21 `m2:test:<name>: PASS` lines, ending with `m2: RESULT PASS (21/21)`
6. 13 `m3:test:<name>: PASS` lines — kernel threads, preemption, ring 3
   + syscalls, processes as address spaces, capability spaces
   (ADR-0012…0015) — ending with `m3: RESULT PASS (13/13)`
7. 8 `m4:test:<name>: PASS` lines — the ELF validator, its rejection
   corpus, the image loader (ADR-0016), the syscall ABI v1 proven from
   ring 3 (ADR-0017), the first user process (its
   `ARENAOS-M43-FIRST-USER-PROCESS…` message on the console is the
   payload's own debug_write), the IPC v1 echo-server demo — two
   processes rendezvousing over an endpoint (ADR-0018) — and the spawn
   protocol's supervisor restart demo: the same image spawned twice
   through SYS_SPAWN, its message on the console once per life
   (ADR-0019) — ending with `m4: RESULT PASS (8/8)`
8. `halting via UEFI ResetSystem(shutdown)` — the clean-halt declaration
9. QEMU exits on its own with status 0

If you see `PANIC`, a `FAIL` marker, or QEMU hangs instead, please open
an issue with the full serial output attached — the log is designed to
be a diagnostic artifact, not decoration.

## Useful variations

```sh
# Keep the machine alive after the suite (drop -no-reboot effect on
# shutdown): there is nothing to interact with yet — milestones are
# headless test boots by design (see docs/ROADMAP.md).

# Log serial to a file instead of the terminal:
    -serial file:serial.log

# Interrupt/CPU-reset trace for debugging (QEMU-side):
    -d int,cpu_reset -D qemu-int.log

# Faster on Linux hosts (untested configuration — the project's
# reference environment is TCG):
    -enable-kvm
```

## Building from source instead

Releases are convenience artifacts; the repo builds everything itself:

```sh
./tools/dev-env/bootstrap.sh   # one-time: Rust, QEMU, EDK2 (see docs/DEV-ENV.md)
./tools/run_tests.sh           # build + M1/M2 harnesses
./tools/run.sh                 # interactive boot, serial on stdio
./tools/stability_loop.sh 100  # 100 clean boots (ADR-0011 gate)
```
