# ArenaOS — Development & Debugging Environment

This document describes the exact toolchain, how to bootstrap it from nothing,
and how to build/boot/debug the system. The dev environment is deliberately
reproducible offline (the primary sandbox has no access to crates.io,
static.rust-lang.org, or Debian mirrors — everything below is pinned to
reachable sources).

## Toolchain inventory (pinned)

| Component | Version | Source | Install location |
|---|---|---|---|
| Rust (rustc, cargo, clippy, rustfmt, llvm-tools incl. `rust-lld`, `llvm-objcopy`) | 1.97.0 stable | PyPI `arena-rust-toolchain[all]==1.97.0` (bundled offline toolchain) | `/opt/rust/prefix` |
| Rust std for `x86_64-unknown-uefi` / `x86_64-unknown-none` (`core`, `alloc`, `compiler_builtins`, `panic_abort`, shims) | built from source, tag `1.97.0` | GitHub `rust-lang/rust` (partial clone, `library/`) via `tools/build-sysroot.sh` | `/opt/rust/prefix/lib/rustlib/<target>/lib` |
| QEMU (qemu-system-x86_64, qemu-img) | 11.0.2 (static musl build) | npm `qemu-portable-linux-x64-musl@0.2.1` | `/opt/qemu` |
| musl libc (loader for the QEMU build; host glibc 2.36 < required 2.38) | v1.2.5 | GitHub `ifduyue/musl` (mirror) | `/opt/musl` |
| UEFI firmware (EDK2/OVMF `edk2-x86_64-code.fd`, `edk2-i386-vars.fd`) | bundled with QEMU 11.0.2 | npm package above | `/opt/qemu/share/qemu` |
| FAT image tooling (ESP creation) | `pyfatfs==1.1.0` | PyPI | `~/.local` |
| zstd extraction helper | `zstandard` (pip) | PyPI | `~/.local` |
| Host compiler (for musl + misc) | gcc 12 (Debian bookworm) | preinstalled | `/usr/bin` |

Nothing else is required. There are no apt packages, no crates.io fetches, no
nightly Rust.

## Bootstrap from a clean sandbox

```bash
tools/dev-env/bootstrap.sh        # idempotent; uses sudo for /opt installs
source tools/dev-env/env.sh       # PATH, QEMU/OVMF resolution for this shell
tools/run_tests.sh                # builds + boots + asserts M1
```

`bootstrap.sh` performs (skipping anything already present):

1. `pip install --user zstandard pyfatfs`
2. Downloads `arena-rust-toolchain` wheels from PyPI, reassembles the split
   `tar.zst`, extracts to `/opt/rust/prefix`.
3. Partial-clones `rust-lang/rust` (tag `1.97.0`, `library/` only) and runs
   `tools/build-sysroot.sh` for `x86_64-unknown-uefi` and
   `x86_64-unknown-none` — this compiles `core`/`compiler_builtins`/`alloc`/
   `panic_abort`/workspace shims and installs them into the toolchain sysroot
   (an offline replacement for `rustup target add`).
4. Downloads QEMU (`qemu-portable-linux-x64-musl` npm tarball) to `/opt/qemu`.
5. Builds musl v1.2.5 to `/opt/musl` (its `libc.so` doubles as the dynamic
   loader for the musl-linked QEMU binaries, since the host lacks
   `/lib/ld-musl-x86_64.so.1`).

### How the sysroot bootstrap works (and why)

The offline sandbox cannot `rustup target add x86_64-unknown-uefi`. But
`rustc` knows both targets natively; only the precompiled libraries are
missing. `tools/build-sysroot.sh` rebuilds exactly those from the official
sources at the toolchain's matching tag:

- crates: `core`, `compiler_builtins` (with `feature="mem"`,
  `feature="unmangled-names"`, and the exact cfgs emitted by its own
  `build.rs`, which the script compiles and runs — including
  `f16_enabled`/`f128_enabled` derived from `rustc --print cfg`'s
  `target_has_reliable_f16/f128`), `alloc`, `panic_abort`, and the two
  `rustc_std_workspace_*` shims;
- flags: `RUSTC_BOOTSTRAP=1` (build-time only), `-Z
  force-unstable-if-unmarked`, `-C panic=abort`, rlibs installed under
  `lib/rustlib/<target>/lib` exactly like a rustup component.

If the toolchain version changes, both the wheels and the `rust-lang/rust`
tag must change together (the script asserts version match).

## Build & boot

```bash
tools/build.sh                     # cargo build (release) -> build/arena-boot.efi
tools/build.sh --image             # + build/arena-esp.img (FAT16 ESP, EFI/BOOT/BOOTX64.EFI)
tools/run.sh                       # interactive: serial on stdio (Ctrl-A X to quit)
tools/test_m1.py                   # automated milestone test (asserts markers)
tools/run_tests.sh                 # all milestone tests
```

QEMU invocation used by the harness (canonical M1 command):

```bash
LD_LIBRARY_PATH=/opt/qemu/lib /opt/musl/lib/libc.so /opt/qemu/bin/qemu-system-x86_64 \
  -L /opt/qemu/share/qemu -M q35 -m 512M -cpu qemu64,+nx \
  -drive if=pflash,format=raw,readonly=on,file=<OVMF_CODE> \
  -drive if=pflash,format=raw,file=<fresh OVMF_VARS copy> \
  -drive format=raw,file=build/arena-esp.img \
  -display none -serial file:build/serial.log -no-reboot
```

On a normal workstation with system packages, the same scripts work with
`qemu-system-x86_64`/`OVMF_CODE.fd` from PATH and distro locations — the
resolution order lives in `tools/qemu_env.py` (env overrides:
`ARENA_QEMU`, `ARENA_OVMF_CODE`, `ARENA_OVMF_VARS`).

## Debugging

- **Serial log first.** Every kernel message is structured
  (`[arena INFO boot] ...`); tests assert on `m1:test:*` markers. The log is
  captured to `build/serial.log` on every harness run.
- **PE inspection:** `llvm-objdump -p build/arena-boot.efi` (headers,
  sections), `llvm-readobj --file-headers`, `llvm-nm`. Available at
  `/opt/rust/prefix/lib/rustlib/x86_64-unknown-linux-gnu/bin/`.
- **QEMU monitor/debug flags:** add `-d int,cpu_reset -D qemu.log` to see
  interrupts/resets; `-monitor stdio` in interactive runs.
- **GDB stub (from M2, once paging/IDT exist):** `-s -S` then
  `gdb build/arena-boot.efi` with `target remote :1234` and
  `add-symbol-file` at the loaded base (OVMF logs the image base to serial at
  `INFO` level in M2+ for exactly this purpose).
- **Triple fault / silent reset:** almost always a GDT/far-jump/paging bug —
  `-d cpu_reset,int` plus the OVMF serial output narrows it; the M1 GDT
  read-back test exists to keep this class out of the baseline.
- **Firmware-side visibility:** EDK2's own BDS output shares the serial log
  (`BdsDxe: loading ...`), useful to distinguish "firmware never loaded us"
  from "we crashed after entry".

## Known environment quirks (sandbox-specific)

- `pyfatfs` cannot overwrite an existing file inside an image (cluster-chain
  bug): ESP images are **always rebuilt from scratch** — `tools/espimg.py`
  enforces this.
- Host glibc is 2.36; the portable QEMU glibc build needs 2.38 → we use the
  musl build launched through `/opt/musl/lib/libc.so` as its loader.
- OVMF falls back to its built-in EFI Shell if `EFI/BOOT/BOOTX64.EFI` is
  missing/corrupt — a shell prompt in the serial log means the ESP image is
  broken, not the kernel.
- No KVM in the sandbox: TCG only. Tests must not depend on wall-clock speed.
