# V1 baseline record — QEMU/backend environment

**Milestone:** V1 — Native Graphical Launch Foundation
**Branch:** `arena/f6b7ebc4-arenaos` (from main `d2bfd660040ddface1cd6e791301fb8ea73d181c`)
**Status:** QEMU backend recovered; current-main baseline boot established through genuine QEMU TCG execution. Guest demonstration (P-1..P-4, N-1..N-14) blocked pending ADR-0114 acceptance and Desktop Identify implementation.

## 1. Emulator (pinned, recovered from Phase 15 qualification archive)

| Identity | Value | Verified against |
|---|---|---|
| Binary | `/home/user/phase15-pkg/backend/qemu-system-x86_64-patched` | SHA-256 `e0ea2b1f5ae2a5aa65a455eb232e46e1743b5c327c48ca678630c214c69f26ff` matches `docs/phase15/ACCEPTANCE.md` and `manifest.json` |
| Origin | Phase 15 package `docs/phase15/artifacts/phase15-patched-tcg-qualification.tar.gz` (SHA verified at extraction) | byte-identical re-extraction |
| Accelerator | explicit `-accel tcg` (TCG only; no KVM/HVPX/WHPX) | per ACCEPTANCE.md scope |
| Source | QEMU v11.1.2, locally modified (patch SHA `c2909ea9…`), official source archive SHA `fc3d810f…` | manifest.json |

QEMU wrappers at `/home/user/phase16-backend/qemu-patched` and
`/home/user/phase16-backend/qemu-unpatched` invoke the pinned binary through
the isolated glibc 2.40 loader with explicit `-accel tcg -L pc-bios`.

## 2. Isolated glibc 2.40

Built via `tools/dev-env/build-isolated-glibc.sh` (m4, bison, gawk, glibc 2.40
from source mirrors; no outbound access to static.rust-lang.org needed for glibc).

| Library | Path | SHA-256 |
|---|---|---|
| libc.so.6 | `/home/user/glibc-2.40/lib/libc.so.6` | `eef95f26903e6f9ac6b8cb92df9ba5306ff01b4058ddf7f9ad6aaa986bb7f35f` |
| ld-linux-x86-64.so.2 | `/home/user/glibc-2.40/lib/ld-linux-x86-64.so.2` | `63704534fba7799d309e9e16992ea7d11e709906bd44b72060c007461b970458` |

## 3. Firmware (OVMF/EDK2)

| Image | Path | Size | SHA-256 |
|---|---|---|---|
| OVMF_CODE_4M.fd | `/home/user/phase15-pkg/firmware/OVMF_CODE_4M.fd` | 3,653,632 | `7e34e3b83224968bb0714c0ebc88f75c5c37b4cc892c902e511ccc7a7c107f94` |
| OVMF_VARS_4M.fd (template) | `/home/user/phase15-pkg/firmware/OVMF_VARS_4M.fd` | 540,672 | `5d2ac383371b408398accee7ec27c8c09ea5b74a0de0ceea6513388b15be5d1e` |

## 4. Rust / toolchain availability (verified)

```
$ ls /opt/rust/prefix/bin/
cargo  cargo-clippy  cargo-fmt  clippy-driver  rustc  rustfmt
$ /opt/rust/prefix/bin/rustc --version
rustc 1.97.0
$ /opt/rust/prefix/bin/cargo --version
cargo 1.97.0
```

- `rustc` and `cargo` exist at `/opt/rust/prefix/bin/` but are **not on PATH** in this sandbox.
- They are **not needed** for the current-main baseline boot because the ESP is a surviving build artifact from a prior `tools/build.sh --image` run; the ESP was **not** rebuilt in this session.
- A fresh rebuild of main `d2bfd66` from source would require the Rust toolchain on PATH and is **not** the same as booting surviving build artifacts.
- The current-main baseline below boots surviving build artifacts at `/workspace/ArenaOS/build/` (bind-mounted from `/home/user/ArenaOS` during an earlier build), not a fresh rebuild.

## 5. Boot artifacts

| Artifact | Path | SHA-256 | Note |
|---|---|---|---|
| arena-esp.img | `/workspace/ArenaOS/build/arena-esp.img` | `d5aa4cc4dc6f468687daa2212db3ad78c4cc0b1941a8b3256f640e112ee1160a` | current-main d2bfd66 ESP; surviving build artifact, not rebuilt in this session |
| EFI (extracted from ESP) | `EFI/BOOT/BOOTX64.EFI` inside the ESP | `7dcee4426edc4b2a204592a5680c27c5ee91d51e3231705641df2ca0d7e2bfe6` | 11,038,720 bytes; extracted and packaged under `docs/v1/evidence-main-current-package/` |

ESP provenance: produced by an earlier `tools/build.sh --image` run in a
bind-mounted workspace (`/home/user/ArenaOS → /workspace/ArenaOS`). The
package README's claim that the Phase 15 artifacts were produced from commit
`f54797f2` refers to a different, historical artifact set; this session's
ESP is from main `d2bfd66`.

## 6. Current-main baseline boot — EXECUTED (green)

Current-main `d2bfd66` kernel/userspace (surviving build artifacts) booted
through the recovered Phase 15 QEMU 11.1.2 TCG backend, with genuine QEMU
screendumps captured.

Command:

```
/home/user/phase16-backend/qemu-patched -M q35 -m 512M \
  -cpu qemu64,+nx,+smep,+smap \
  -drive if=pflash,format=raw,readonly=on,file=/home/user/phase15-pkg/firmware/OVMF_CODE_4M.fd \
  -drive if=pflash,format=raw,file=<fresh OVMF_VARS_4M.fd copy> \
  -drive if=none,id=esp0,format=raw,file=<current-main arena-esp.img copy> \
  -device ide-hd,drive=esp0,bus=ide.0,bootindex=0 \
  -drive file=<scratch.img>,format=raw,if=none,id=scr0 \
  -device virtio-blk-pci,drive=scr0 \
  -netdev user,id=net0 -device virtio-net-pci,netdev=net0 \
  -device virtio-rng-pci -device virtio-keyboard-pci \
  -chardev socket,path=<vcon.sock>,server=on,wait=on \
  -device virtio-serial-pci,max_ports=1 -device virtconsole,chardev=vcon0 \
  -qmp unix:<qmp.sock>,server=on,wait=off \
  -vga std -display none \
  -chardev stdio,id=con0,signal=off -serial chardev:con0 -no-reboot
```

The wrapper invokes the pinned QEMU binary through the isolated glibc 2.40
loader (`ld-linux-x86-64.so.2 --library-path ...`), with explicit
`-accel tcg`, `-L pc-bios`.

Evidence captured at `/home/user/v1-boot/evidence-main-current/`
and packaged under `docs/v1/evidence-main-current-package/`:

| Artifact | SHA-256 | Notes |
|---|---|---|
| shot-000-main-firmware.ppm | `15e7d086f38e02fb3b8af538b68d76619b0ecf56927333e1ba7510e79346fb03` | 640×480 genuine QEMU screendump, firmware/boot screen |
| shot-001-main-desktop.ppm | `f4565433c53b2e45d3a1ee8f008fda8f64dbbeb82f8e8a44789bc244f4613a15` | 800×600 genuine QEMU screendump, desktop presented |
| serial-main.log | `a3d9b288c16b4dca4e6f9e4a167e787b4b55fd101288a39c05f7fdff1f052e2e` | full boot through shell + shutdown |
| qemu-stderr-main.log | `69b055e5634c380f40e7cd60511b1b0363ae950c6c02cf861bf32f5682e5fef3` | QEMU stderr, kept verbatim |
| udp-dns-main.log | `a0fc5068a30840b152219e69e965fc7c65874f2eafca6ec6dbf0f7e8a69802fb` | M7 fixture log |
| tcp-main.log | `511611c8e6cb584101014d4b5372a698914d69db907b07d9a17e4356b28f09a8` | M7 fixture log |
| BOOTX64.EFI (extracted from ESP) | `7dcee4426edc4b2a204592a5680c27c5ee91d51e3231705641df2ca0d7e2bfe6` | 11,038,720 bytes |
| evidence.json | `3dd7a6639cbd0933e45f4b12cda749855374e2eb4bb5236f0054bced33a8b2b7` | all identities, hashes, exit code |

Completion criteria — all met:

- [x] QEMU starts and reaches the firmware/boot screen (screendump PPM captured)
- [x] Serial log shows a clean UEFI→kernel→userspace boot with no PANIC
- [x] Desktop frame presented ("real desktop frame presented; gallery launcher available")
- [x] Shell reached; feeder typed `shutdown` at the `arena>` prompt
- [x] Kernel declared clean halt via UEFI ResetSystem(shutdown)
- [x] QEMU exit code 0 / clean termination
- [x] All evidence file hashes recorded in evidence.json

**Current-main baseline evidence is separate from historical Phase 15
environment evidence.** The Phase 15 accepted ESP (f54797f2-era kernel) was
used for the original baseline validation; the current-main ESP (d2bfd66) is
booted here.

## 7. Backend controls — PASS

| Control | QEMU exit | Result |
|---|---|---|
| Patched QEMU | 33 | PASS |
| Unpatched QEMU | 35 | Expected fail |
| Verifier | 0 | PASS |

Evidence in `build/backend-controls/` (exit-*.txt, serial-*.log,
qemu-stderr-*.log, xm-verify.json).

## 8. V1 contract status (what is and is not done)

- **QEMU recovery:** completed — pinned QEMU 11.1.2 TCG backend running through isolated glibc 2.40; current-main baseline boot established.
- **c_gfx_app.c:** corrected, built, audited (GCC + Clang); static audit pass, ABI 94/94, host 1721 checks PASS.
- **ADR-0114:** Proposed, **not accepted**. Identify round-trip specified but not implemented. No Desktop server opcode-13 handling, no kernel changes.
- **V1 graphical guest acceptance (P-1..P-4, N-1..N-14):** **not completed.** Blocked pending ADR-0114 acceptance and Desktop Identify implementation.
- **Discovery-only identification:** diagnostic scaffolding only. It locates candidates by kind+rights over the verified ARST record but does **not** resolve B-GFX-1, does **not** implement an accepted identification contract, and does **not** satisfy V1's positive acceptance case.
- **Evidence package:** committed under `docs/v1/evidence-main-current-package/` with manifest, SHA-256 sidecar, and verification script; verified after fresh extraction.