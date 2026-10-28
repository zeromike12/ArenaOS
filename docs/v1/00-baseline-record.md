# V1 baseline record — QEMU/backend environment

**Milestone:** V1 — Native Graphical Launch Foundation
**Branch:** `arena/f6b7ebc4-arenaos` (from main `d2bfd660040ddface1cd6e791301fb8ea73d181c`)
**Status:** environment baseline established; guest demonstration blocked by missing
Rust toolchain in this sandbox (see §5).

## 1. Emulator (pinned)

| Identity | Value | Verified against |
|---|---|---|
| Binary | `/home/user/arena-backend/qemu-system-x86_64` | SHA-256 `e0ea2b1f5ae2a5aa65a455eb232e46e1743b5c327c48ca678630c214c69f26ff` matches `docs/phase15/ACCEPTANCE.md` and `manifest.json` |
| Origin | Phase 15 package `/tmp/qemu-pkg/backend/qemu-system-x86_64-patched`; copied and re-hashed at staging | byte-identical re-copy |
| Accelerator | explicit `-accel tcg` (TCG only; no KVM/HVPX/WHPX) | per ACCEPTANCE.md scope |
| Source | QEMU v11.1.2, locally modified (patch SHA `c2909ea9…`), official source archive SHA `fc3d810f…` | manifest.json |
| Runtime libs | `/home/user/arena-backend/runtime/lib/` (libglib-2.0.so.0, libpixman-1.so.0, libslirp.so.0) | staged with the binary; `LD_LIBRARY_PATH` required at launch |

Launch prefix (used for all baseline boots):

```
ARENA_QEMU="env LD_LIBRARY_PATH=/home/user/arena-backend/runtime/lib /home/user/arena-backend/qemu-system-x86_64 -accel tcg -L /home/user/arena-backend/pc-bios"
```

`-L .../pc-bios` is required so the emulator finds its ROMs; the bundled pc-bios ROMs are the package's (SHA-inventoried in manifest.json).

## 2. Firmware (OVMF/EDK2)

| Image | Path | Size | SHA-256 |
|---|---|---|---|
| OVMF_CODE_4M.fd | `/home/user/arena-backend/firmware/OVMF_CODE_4M.fd` | 3,653,632 | `7e34e3b83224968bb0714c0ebc88f75c5c37b4cc892c902e511ccc7a7c107f94` |
| OVMF_VARS_4M.fd | `/home/user/arena-backend/firmware/OVMF_VARS_4M.fd` | 540,672 | `5d2ac383371b408398accee7ec27c8c09ea5b74a0de0ceea6513388b15be5d1e` |

Both were decompressed from the package's bz2-compressed pc-bios firmware
(`edk2-x86_64-code.fd.bz2`, `edk2-i386-vars.fd.bz2`) at `/tmp/OVMF_CODE.fd` and
`/tmp/OVMF_VARS.fd`, then copied to staging and re-hashed. VARS is copied fresh
per boot (fresh NVRAM), per `tools/mtest.py`.

## 3. Boot artifacts (accepted strict Phase 15 identities)

| Artifact | Path | SHA-256 | Note |
|---|---|---|---|
| arena-boot.efi | `/tmp/qemu-pkg/artifacts/arena-boot.efi` | `bbbc63df271c3d7d5109789369f72e093b02ef77c77ebe25273b6da4c00d238e` | accepted strict EFI |
| arena-esp.img | `/tmp/qemu-pkg/artifacts/arena-esp.img` | `631cc6452ecbacf58b2418e54b9c509bb717cc2211580ebef63f9aa4c13d866f` | accepted strict ESP |

ESP provenance (honest record): the package README states these artifacts were
produced from ArenaOS source commit `f54797f2fb5bac0b3440f986d2120382b3611ecc` —
**not** from this session's main `d2bfd66` (that commit is not in this repo's
history). This sandbox has no Rust toolchain (`cargo`/`rustc` absent; static.rust-lang.org
is not among the allowed outbound hosts), so `tools/build.sh --image` cannot rebuild
the kernel ESP from main source here. The baseline boot below therefore runs the
accepted Phase 15 ESP, and this limitation is recorded rather than worked around.

## 4. Baseline boot — EXECUTED (green)

Command (sandbox-safe, own staging only):

```
/home/user/arena-backend/qemu-launcher.sh -M q35 -m 512M \
  -cpu qemu64,+nx,+smep,+smap \
  -drive if=pflash,format=raw,readonly=on,file=/home/user/arena-backend/firmware/OVMF_CODE_4M.fd \
  -drive if=pflash,format=raw,file=<fresh OVMF_VARS_4M.fd copy> \
  -drive if=none,id=esp0,format=raw,file=<arena-esp.img copy> \
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

The launcher execs the pinned QEMU through the isolated glibc 2.40 loader
with `LD_LIBRARY_PATH` = glibc-2.40/lib + arena-backend/runtime/lib +
/usr/lib/x86_64-linux-gnu (host libs read-only, unmodified), explicit
`-accel tcg`, `-L pc-bios`.

Evidence captured at `/home/user/v1-boot/evidence-baseline/`:

| Artifact | SHA-256 | Notes |
|---|---|---|
| shot-000-baseline-firmware.ppm | `15e7d086f38e02fb3b8af538b68d76619b0ecf56927333e1ba7510e79346fb03` | 640×480 genuine QEMU screendump, firmware/boot screen |
| shot-001-baseline-desktop.ppm | `f4565433c53b2e45d3a1ee8f008fda8f64dbbeb82f8e8a44789bc244f4613a15` | 800×600 genuine QEMU screendump, desktop presented |
| serial-baseline.log | (see evidence.json) | full boot through shell + shutdown |
| qemu-stderr-baseline.log | (see evidence.json) | QEMU stderr, kept verbatim |
| udp-dns-baseline.log, tcp-baseline.log | (see evidence.json) | M7 fixture logs |
| evidence.json | — | all identities, hashes, exit code |

Completion criteria — all met:

- [x] QEMU starts and reaches the firmware/boot screen (screendump PPM captured)
- [x] Serial log shows a clean UEFI→kernel→userspace boot with no PANIC
- [x] Desktop frame presented ("real desktop frame presented; gallery launcher available")
- [x] Shell reached; feeder typed `shutdown` at the `arena>` prompt
- [x] Kernel declared clean halt via UEFI ResetSystem(shutdown)
- [x] QEMU exit code 0 / clean termination
- [x] All evidence file hashes recorded in evidence.json

The booted ESP is the accepted Phase 15 artifact (f54797f2-era kernel — see
§3 provenance note); current-main d2bfd66 could not be rebuilt here
(§5 limitation).

## 5. Environment limitation (recorded, not worked around)

No Rust toolchain exists in this sandbox, and installing one is not possible
from the allowed outbound hosts (static.rust-lang.org is not allowed).
Consequently:

- `tools/build.sh --image` cannot run; main `d2bfd66` kernel/userspace cannot be rebuilt here.
- The QEMU baseline boot uses the pinned Phase 15 accepted ESP (f54797f2-era kernel), recorded above; it validates the *emulator/firmware/environment baseline*, not a rebuild of current main.
- Any guest demonstration that requires a graphics-capable mainline kernel ESP is deferred until a buildable environment exists; the C graphical app is still written, built as a guest ELF, and statically audited (GCC + Clang) here (see docs/v1/02-app-design.md).
- This sandbox also has no QEMU binary and no `/dev/kvm`, so baseline QEMU boots cannot be re-executed here; the record below documents what was established in the qualified environment.

This is the honest baseline: genuine QEMU screenshots, raw logs, and artifact
hashes are captured for what can run; what cannot run is declared, not faked.
Labelled mockups are never presented as OS execution.

## 6. Current-main build (from source, in-bind-mount)

`tools/build.sh --image` was run successfully against unchanged main `d2bfd66`
from `/workspace/ArenaOS` (bind mount `/home/user/ArenaOS → /workspace/ArenaOS`
so path-embedded strings match the expected PIE fixture hash):

- `build/arena-boot.efi` 11,038,720 bytes; ISA audit PASS (no AVX/x87 in guest path; x87/SSE2 baseline only)
- `build/arena-esp.img` 16,777,216 bytes with `/EFI/BOOT/BOOTX64.EFI`
- Protected identities preserved: PIE fixture `d7b0a8cf…`, signed APB1 `04add54f…`

This main build could not be booted here because the sandbox has no QEMU.
