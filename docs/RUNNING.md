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
truncate -s 8M scratch.img      # Milestone-5 fixture disk (ADR-0021);
                                # dd if=/dev/zero of=scratch.img bs=1m count=8 works too

qemu-system-x86_64 \
    -M q35 -m 512M -cpu qemu64,+nx,+smep,+smap \
    -drive if=pflash,format=raw,readonly=on,file=edk2-x86_64-code.fd \
    -drive if=pflash,format=raw,file=ovmf-vars.img \
    -drive format=raw,file=arena-esp.img \
    -drive file=scratch.img,format=raw,if=none,id=scr0 \
    -device virtio-blk-pci,drive=scr0 \
    -display none -serial mon:stdio -no-reboot
```

Since M5.1 the scratch disk is **required**: the boot-time m5 suite
asserts the kernel's PCI scan finds a virtio-blk device, and a boot
without one halts after the m4 suite by design (its contents are not
yet used — that arrives with the storage driver in 5.2).

Serial is the console — in **both directions**. Everything ArenaOS logs
goes there, and since Milestone 4.6 (ADR-0020) your keystrokes come
back in through the same port: after the boot-time test suites pass (a
few seconds), the kernel spawns the **shell** and the machine waits for
you at the `arena> ` prompt. Type `help`. The VM stops only when you
type `shutdown` (or kill QEMU with `Ctrl-A X`) — a boot that ends by
itself would mean the shell never came up.

### Using your distro's OVMF instead

The bundled firmware is optional — any recent OVMF/EDK2 x86-64 split
(code + vars) works. Point the two `pflash` drives at your distro's
files instead, e.g. on Debian/Ubuntu:

```sh
    -drive if=pflash,format=raw,readonly=on,file=/usr/share/OVMF/OVMF_CODE.fd \
    -drive if=pflash,format=raw,file=/usr/share/OVMF/OVMF_VARS.fd \
```

(copy `OVMF_VARS.fd` to a writable location first, as above).

## The shell

The initial service (a real userspace image, spawned through the M4.5
protocol with kernel-granted capabilities). What it understands:

| Command | What happens |
|---|---|
| `help` | lists the builtins |
| `ps` | live processes as `(pid, threads)` pairs — you will see the shell itself |
| `echo TEXT` | prints TEXT (the kernel line discipline echoes as you type; backspace works) |
| `spawn` | `SYS_SPAWN`s registry image 0 — the untouched M4.3 test payload — as a child process: its pinned message lands mid-session, then its exit badge comes back through the shell's notification |
| `shutdown` | the Power-gated halt: the kernel logs the requesting pid and hands the machine to firmware's `ResetSystem` |

Anything else answers `unknown command: '…' — try 'help'`.

## What a healthy boot looks like (current: Milestone 4 complete)

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
7. 9 `m4:test:<name>: PASS` lines — the ELF validator, its rejection
   corpus, the image loader (ADR-0016), the syscall ABI v1 proven from
   ring 3 (ADR-0017), the first user process (its
   `ARENAOS-M43-FIRST-USER-PROCESS…` message on the console is the
   payload's own debug_write), the IPC v1 echo-server demo — two
   processes rendezvousing over an endpoint (ADR-0018), the spawn
   protocol's supervisor restart demo — the same image spawned twice
   through SYS_SPAWN, its message on the console once per life
   (ADR-0019) — and the console input service: the line discipline
   driven through the RX ISR's own entry point, with a ring-3 reader
   parked and woken by a fed line (ADR-0020), ending with
   `m4: RESULT PASS (9/9)`
8. `console input armed: com1 rx -> ioapic pin 4 -> vector 33` — the
   input half of the console (right after the timer-chain line, step 4)
9. `milestone 4 complete … spawning the shell`, then
   `shell spawned: pid …` — the hand-off (ADR-0020)
10. `ArenaOS shell v0.4 …` and the `arena> ` prompt — the machine is
    now an interactive system; type into it (see "The shell" above)
11. After `shutdown`: `shutdown requested by pid … through its Power
    cap` and `halting via UEFI ResetSystem(shutdown)` — the clean-halt
    declaration (the automated harnesses type `shutdown` for you,
    marker-paced)
12. QEMU exits on its own with status 0

If you see `PANIC`, a `FAIL` marker, or QEMU hangs instead, please open
an issue with the full serial output attached — the log is designed to
be a diagnostic artifact, not decoration.

## Useful variations

```sh
# Log serial to a file INSTEAD of the terminal — output only: the shell
# gets no input this way and the machine stays up until you kill QEMU
# (the automated suites feed it from a pipe; see tools/mtest.py):
    -serial file:serial.log

# Log to a file AND keep typing (the monitor multiplex also lands in
# the log — fine for humans, the harnesses use a clean chardev instead):
    -serial mon:stdio | tee serial.log

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
