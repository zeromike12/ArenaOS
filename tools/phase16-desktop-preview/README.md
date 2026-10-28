# DIAGNOSTIC DESKTOP PREVIEW — XF NOT RUN

This is a separate Phase 16 desktop demo for stock QEMU TCG, **not** a qualification, release, or milestone acceptance. The m15 historical diagnostic omits the #XF test: `m15: DIAGNOSTIC PASS (5/5; XF NOT RUN)` is **not** strict m15 PASS. The strict Phase 16 artifacts remain separate and unchanged.

## Windows launch

Extract all ZIP contents into `C:\ArenaOS-Phase16-Desktop-Preview` (use a writable path without commas). Install stock Windows QEMU with its SDL display backend; the default executable is `C:\Program Files\qemu\qemu-system-x86_64.exe`. For a different installation, set `QEMU_EXE` to the complete executable path before running the command. No compiler, Python, pip, or administrator privileges are needed.

From **Command Prompt**, run exactly:

```cmd
cd /d C:\ArenaOS-Phase16-Desktop-Preview && launch.cmd
```

Wait until the serial console says `inputtest: input-service client starting`, click the QEMU window, and type **arena** (no Enter). Boot will continue to Desktop. Click the dock to open apps. To stop cleanly, return to the serial console, type `shutdown`, and press Enter; do not close the window as a substitute for shutdown. `run\serial.log` is replaced each launch. `run\preview-data.img` and `run\preview-vars.fd` persist between launches; delete them while QEMU is stopped to start fresh. The immutable templates remain in the package. The ESP uses a temporary snapshot.

The launcher prebinds local loopback TCP 54321 and UDP 1053 for the existing boot-test peers; those ports must be free. It starts QEMU explicitly with `-accel tcg`, q35, 512 MiB, qemu64, the bundled EDK2 firmware, the prepared AFS1/AFS2 data disk, virtio input/network/rng, and SDL. It cleans up its helper jobs after QEMU exits. If QEMU refuses a device or SDL, verify the installed Windows QEMU supports those options.

## Evidence and limitations

`evidence/linux-tcg-serial.log` is the raw serial output from a Linux stock QEMU 11.0.2 TCG boot of the extracted preview image, firmware, and disk template. `evidence/linux-tcg-desktop.png` is a genuine QMP framebuffer screenshot after a virtual tablet click launched the UI Gallery. The raw TCP/DNS peer captures and QEMU stderr are alongside it. That run saw the diagnostic marker, package-service READY and installed-registry READY, the real Desktop frame, input-triggered application spawn and a framebuffer change, and a guest-authorized UEFI shutdown with QEMU exit 0. It used the repository's Python network peers and headless display/QMP instrumentation on Linux; the Windows PowerShell peer/launcher implementation and Windows QEMU were **not tested on Windows**. This is not a cross-platform qualification claim.

The image was built from the Phase 16 mainline checkout with the unchanged command:

```sh
ARENA_GRAPHICS_FIXTURE=desktop ARENA_PHASE15_HISTORICAL_DIAGNOSTIC=1 tools/build.sh --image
```

The normal protected PIE and signed APB1 checks passed with their original identities; the kernel ISA audit passed. No kernel source, FP/SIMD initialization or switching, capability protections, signing checks, or Phase 14 build guard were changed for this preview. `SHA256SUMS.txt` lists immutable package files; the ZIP's own SHA-256 is supplied separately. EFI is provided for identification/inspection; the launcher boots the ESP.
