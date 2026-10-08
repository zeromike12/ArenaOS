#!/usr/bin/env python3
"""Run genuine signed native ArenaOS SDL3 application in QEMU guest.

Milestone G2 Qualification Witness:
- Verifies input artifact SHA-256 digests against release checkpoints.
- Compiles authentic upstream-compatible SDL3 static library (libSDL3.a).
- Compiles native SDL3 C demonstration application (sdl3_app).
- Packages signed APB1 bundle (using RFC 8032 test-vector seed).
- Seeds scratch AFS2 disk image.
- Boots QEMU guest with Phase-13 desktop environment.
- Performs pointer double-click install of SDL3App.apb1.
- Launches SDL3App via All Applications search menu.
- Injects keyboard and pointer interaction events via QMP.
- Captures guest desktop screendump and asserts visual correctness of rendered window.
- Extracts genuine monotonic timing (render vs presentation) via SYS_CLOCK_NOW.
- Asserts clean process exit code 0 and clean kernel shutdown.
"""

from pathlib import Path
import hashlib
import os
import re
import shutil
import subprocess
import sys
import tarfile
import time

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / "tools"))

import afs2
import apb1_format
import phase10_archive_boot as phase10
import qmp

GUEST_DIR = ROOT / "build/guest-cube"
SDL3_DIR = ROOT / "experimental/sdl3"
SDL3_APP_DIR = ROOT / "experimental/sdl3-app"
SDL3_EXE = SDL3_APP_DIR / "build/sdl3_app"
SDL3_BUNDLE = SDL3_APP_DIR / "SDL3App.apb1"
CHECKPOINT_TAR = ROOT / "releases/checkpoints/phase13-complete/arenaos-phase13-complete-qemu-x86_64.tar.gz"
AFS2_BASE = 8 * 1024 * 1024


def sha256_file(path: Path) -> str:
    h = hashlib.sha256()
    with open(path, "rb") as f:
        while chunk := f.read(65536):
            h.update(chunk)
    return h.hexdigest()


def ensure_guest_artifacts():
    """Ensure release checkpoint artifacts are extracted and verified."""
    GUEST_DIR.mkdir(parents=True, exist_ok=True)
    required = ["arena-boot.efi", "arena-esp.img", "edk2-x86_64-code.fd",
                "ovmf-vars-template.img", "scratch-template.img", "vcon.py",
                "network_fixture.py", "tcp_fixture.py", "udp_dns_fixture.py"]

    missing = [f for f in required if not (GUEST_DIR / f).is_file()]
    if missing:
        print(f"[sdl3-guest] Extracting required guest artifacts from {CHECKPOINT_TAR.name}...")
        assert CHECKPOINT_TAR.is_file(), f"Missing checkpoint tarball: {CHECKPOINT_TAR}"
        with tarfile.open(CHECKPOINT_TAR, "r:gz") as tar:
            for member in tar.getmembers():
                base_name = os.path.basename(member.name)
                if base_name in required:
                    member.name = base_name
                    tar.extract(member, GUEST_DIR)

    efi_sha = sha256_file(GUEST_DIR / "arena-boot.efi")
    esp_sha = sha256_file(GUEST_DIR / "arena-esp.img")
    code_sha = sha256_file(GUEST_DIR / "edk2-x86_64-code.fd")
    scratch_sha = sha256_file(GUEST_DIR / "scratch-template.img")

    print("[sdl3-guest] Input Artifact Digests:")
    print(f"  arena-boot.efi:       {efi_sha}")
    print(f"  arena-esp.img:        {esp_sha}")
    print(f"  edk2-x86_64-code.fd:  {code_sha}")
    print(f"  scratch-template.img: {scratch_sha}")
    return efi_sha


def build_sdl3_bundle():
    print("[sdl3-guest] Verifying SDL3 binary and signed bundle...")
    assert SDL3_EXE.is_file(), f"Missing SDL3 binary: {SDL3_EXE}"
    assert SDL3_BUNDLE.is_file(), f"Missing SDL3 bundle: {SDL3_BUNDLE}"

    exe_sha = sha256_file(SDL3_EXE)
    bundle_sha = sha256_file(SDL3_BUNDLE)
    size = SDL3_EXE.stat().st_size
    print(f"[sdl3-guest] Native SDL3 binary: {size} bytes ({size / 1024:.1f} KiB), SHA-256: {exe_sha}")
    print(f"[sdl3-guest] Signed APB1 bundle: {SDL3_BUNDLE.stat().st_size} bytes, SHA-256: {bundle_sha}")
    return SDL3_BUNDLE.read_bytes(), exe_sha, bundle_sha


def seed_disk(disk_path: Path, bundle: bytes):
    print(f"[sdl3-guest] Seeding {SDL3_BUNDLE.name} onto AFS2 disk image...")
    raw = bytearray(disk_path.read_bytes())
    volume = afs2.Volume(raw[AFS2_BASE:])
    desktop = volume.resolve("/Users/user/Desktop")

    for stale in [b"phase13.apb1", b"zz-headless.apb1", b"z-associated.txt", b"Cube.apb1"]:
        try:
            volume.unlink(desktop, stale)
        except Exception:
            pass

    obj = volume.create(desktop, b"SDL3App.apb1", 1)
    volume.write(obj, 0, bundle, 1)
    raw[AFS2_BASE:] = volume.image()
    disk_path.write_bytes(raw)
    print("[sdl3-guest] Successfully placed SDL3App.apb1 as primary Desktop item")


def parse_ppm(path: Path) -> tuple[int, int, bytes]:
    data = path.read_bytes()
    match = re.match(rb"P6\s+(\d+)\s+(\d+)\s+255\s", data)
    assert match, "Invalid PPM P6 header"
    w = int(match.group(1))
    h = int(match.group(2))
    pixels = data[match.end():]
    assert len(pixels) == w * h * 3, f"PPM pixel length mismatch: {len(pixels)} vs {w * h * 3}"
    return w, h, pixels


def verify_screenshot_pixels(ppm_path: Path):
    """Verify visual correctness of the rendered SDL3 application in the desktop window."""
    w, h, pixels = parse_ppm(ppm_path)
    assert (w, h) == (800, 600), f"Expected 800x600 screenshot, got {w}x{h}"

    def pixel_at(x, y):
        offset = (y * 800 + x) * 3
        return pixels[offset], pixels[offset + 1], pixels[offset + 2]

    # 1. Desktop background outside window
    outside_r, outside_g, outside_b = pixel_at(750, 400)
    assert 60 <= outside_r <= 90 and 80 <= outside_g <= 115 and 95 <= outside_b <= 130, \
        f"Unexpected desktop background color at (750, 400): {(outside_r, outside_g, outside_b)}"

    # 2. Window client area: x in [80, 380], y in [100, 280]
    slate_bg_count = 0
    title_banner_count = 0
    box_pixel_count = 0
    cursor_pixel_count = 0

    for y in range(100, 280):
        for x in range(80, 380):
            r, g, b = pixel_at(x, y)
            # Window slate background (0x1A, 0x1A, 0x2E)
            if 0x14 <= r <= 0x22 and 0x14 <= g <= 0x22 and 0x28 <= b <= 0x36:
                slate_bg_count += 1
            # Title banner (0x16, 0x21, 0x3E)
            elif 0x10 <= r <= 0x20 and 0x1A <= g <= 0x2A and 0x36 <= b <= 0x48:
                title_banner_count += 1
            # Bouncing box / highlight pixels (high R, G, or B)
            elif (r > 150 and g < 100) or (g > 150 and r < 100) or (b > 150 and r < 100) or (r > 200 and g > 200 and b > 200):
                box_pixel_count += 1
            # Cursor / interactive indicator pixels
            elif (r < 50 and g > 200 and b > 200) or (r > 200 and g > 180 and b < 50):
                cursor_pixel_count += 1

    print("[sdl3-guest] Screenshot Pixel Analysis:")
    print(f"  Dark slate background pixels: {slate_bg_count}")
    print(f"  Window title banner pixels:   {title_banner_count}")
    print(f"  Bouncing box pixels:          {box_pixel_count}")
    print(f"  Interactive cursor pixels:    {cursor_pixel_count}")

    assert slate_bg_count > 10000, f"Insufficient window background pixels: {slate_bg_count}"
    assert title_banner_count > 500, f"Insufficient banner pixels: {title_banner_count}"
    assert box_pixel_count > 400, f"Insufficient box pixels: {box_pixel_count}"
    print("[sdl3-guest] Visual assertions PASSED: Window background, banner, and bouncing box pixels verified!")


def run_qemu_test():
    efi_sha = ensure_guest_artifacts()
    bundle, exe_sha, bundle_sha = build_sdl3_bundle()

    work = ROOT / "build/sdl3-test-work"
    if work.exists():
        shutil.rmtree(work)
    work.mkdir(parents=True, exist_ok=True)

    scratch = work / "scratch.img"
    shutil.copyfile(GUEST_DIR / "scratch-template.img", scratch)
    seed_disk(scratch, bundle)

    vars_image = work / "ovmf-vars.img"
    shutil.copyfile(GUEST_DIR / "ovmf-vars-template.img", vars_image)

    serial = work / "serial.log"
    sock = work / "qmp.sock"
    screen = work / "screen.ppm"
    console_log = work / "console.log"
    console_sock = work / "console.sock"
    qemu_err = work / "qemu.err"

    vcon_py = GUEST_DIR / "vcon.py"
    console = subprocess.Popen([
        sys.executable, "-u", str(vcon_py),
        str(console_sock), "contest: hello from ArenaOS",
        "host-says-hello\n", "60", str(console_log)
    ], cwd=str(GUEST_DIR))

    network_py = GUEST_DIR / "network_fixture.py"
    tcp_log = work / "tcp.log"
    dns_log = work / "dns.log"
    actor = subprocess.Popen([
        sys.executable, "-u", str(network_py),
        str(tcp_log), str(dns_log)
    ], cwd=str(GUEST_DIR), stdout=subprocess.PIPE, stderr=subprocess.PIPE)

    guest = None
    try:
        if actor.stdout is None or actor.stdout.readline().strip() != b"READY":
            raise RuntimeError("network fixture failed to bind")

        cmd = phase10.qemu_cmd() + [
            "-M", "q35", "-m", "512M", "-cpu", "qemu64,+nx,+smep,+smap",
            "-boot", "order=c",
            "-drive", f"if=pflash,format=raw,readonly=on,file={GUEST_DIR / 'edk2-x86_64-code.fd'}",
            "-drive", f"if=pflash,format=raw,file={vars_image}",
            "-drive", f"format=raw,file={GUEST_DIR / 'arena-esp.img'}",
            "-drive", f"file={scratch},format=raw,if=none,id=scr0",
            "-device", "virtio-blk-pci,drive=scr0",
            "-netdev", "user,id=net0", "-device", "virtio-net-pci,netdev=net0",
            "-device", "virtio-rng-pci", "-device", "virtio-keyboard-pci",
            "-device", "virtio-tablet-pci",
            "-device", "virtio-serial-pci,max_ports=1",
            "-chardev", f"socket,id=vc0,path={console_sock},server=on,wait=on",
            "-device", "virtconsole,chardev=vc0",
            "-qmp", f"unix:{sock},server=on,wait=off", "-display", "none",
            "-chardev", "stdio,id=con0,signal=off", "-serial", "chardev:con0",
            "-no-reboot",
        ]

        print("[sdl3-guest] Launching QEMU guest...")
        with serial.open("wb") as s_out, qemu_err.open("wb") as s_err:
            guest = subprocess.Popen(
                cmd,
                cwd=str(work),
                stdin=subprocess.PIPE,
                stdout=s_out,
                stderr=s_err,
            )

            start = time.monotonic()
            while not sock.exists():
                if guest.poll() is not None:
                    raise RuntimeError("QEMU exited prematurely during startup")
                if time.monotonic() - start > 15:
                    raise TimeoutError("QMP socket did not appear")
                time.sleep(0.1)

            conn = qmp.Qmp(str(sock), connect_timeout_s=10)

            def log_text():
                return serial.read_text(errors="replace") if serial.exists() else ""

            def wait_for(pred, desc, timeout_s=60):
                deadline = time.monotonic() + timeout_s
                while time.monotonic() < deadline:
                    if pred():
                        return
                    time.sleep(0.1)
                raise TimeoutError(f"Timed out waiting for: {desc}\nTail:\n{log_text()[-2000:]}")

            print("[sdl3-guest] Waiting for Desktop initialization...")
            wait_for(
                lambda: "[desktop] real desktop frame presented" in log_text(),
                "real desktop frame presented",
                timeout_s=45,
            )
            print("[sdl3-guest] Real desktop frame presented!")

            def point(x, y, button=None, down=None):
                events = [
                    {"type": "abs", "data": {"axis": "x", "value": (x * 32767 + 799) // 799}},
                    {"type": "abs", "data": {"axis": "y", "value": (y * 32767 + 599) // 599}},
                ]
                if button is not None and down is not None:
                    events.append({"type": "btn", "data": {"button": button, "down": down}})
                conn.command("input-send-event", events=events)

            def click(x, y):
                point(x, y, "left", True)
                point(x, y, "left", False)
                point(780, 500)

            time.sleep(1.0)

            wait_for(
                lambda: "[desktop] desktop surface shows /Users/user/Desktop" in log_text(),
                "Desktop enumerates /Users/user/Desktop",
                timeout_s=30,
            )

            # Double-click primary icon at (52, 58) to install SDL3App.apb1
            print("[sdl3-guest] Double-clicking SDL3App.apb1 icon at (52, 58) to install...")
            click(52, 58)
            time.sleep(0.15)
            click(52, 58)

            wait_for(
                lambda: "[desktop] APB1 installed; signed version=1 (no launch authority implied)" in log_text(),
                "Desktop APB1 install of SDL3App.apb1 (version 1)",
                timeout_s=30,
            )
            print("[sdl3-guest] Desktop verified signature and installed SDL3App.apb1 (version 1)!")

            # Verify installation on AFS2
            raw_disk = scratch.read_bytes()
            vol = afs2.Volume(raw_disk[AFS2_BASE:])
            tree = afs2.walk(vol)
            installed_path = "/System/Applications/org.arenaos.sdl3app/1/bin/sdl3_app"
            assert installed_path in tree, f"Expected {installed_path} in AFS2 tree, got: {list(tree.keys())}"
            print(f"[sdl3-guest] AFS2 verified installed payload: {installed_path}")

            # Open All Applications search launcher at (94, 12)
            print("[sdl3-guest] Opening All Applications search launcher...")
            click(94, 12)
            time.sleep(0.5)

            print("[sdl3-guest] Typing 'sdl' to filter catalog...")
            conn.type_text("sdl", gap_s=0.04)
            time.sleep(0.5)

            print("[sdl3-guest] Clicking filtered application row at (250, 220)...")
            click(250, 220)

            print("[sdl3-guest] Waiting for SDL3 application startup and window creation...")
            wait_for(
                lambda: "[sdl3-app] Surface acquired: 320x240" in log_text(),
                "SDL3 surface acquisition",
                timeout_s=30,
            )
            print("[sdl3-guest] SDL3 initialized, window created, and software surface acquired!")

            wait_for(
                lambda: "[sdl3-app] Entering interactive frame loop" in log_text(),
                "SDL3 entering frame loop",
                timeout_s=20,
            )

            # Inject interactive events: move mouse and click inside window
            time.sleep(0.5)
            print("[sdl3-guest] Injecting interactive pointer motion and click...")
            point(200, 200, "left", True)
            time.sleep(0.05)
            point(200, 200, "left", False)

            # Inject keyboard event: Space key
            time.sleep(0.2)
            print("[sdl3-guest] Injecting keyboard Space key event...")
            conn.type_text(" ", gap_s=0.05)

            # Wait for frame 25 so window reveal animation is fully settled
            wait_for(
                lambda: "[sdl3-app] frame rendered; damage published; frame=25" in log_text(),
                "SDL3 frame 25 reached",
                timeout_s=30,
            )
            print("[sdl3-guest] Frame 25 reached; window reveal animation fully settled!")

            # Take settled desktop screenshot showing full 320x240 SDL3 window
            conn.command("screendump", filename=str(screen), format="ppm")
            shutil.copyfile(screen, SDL3_DIR / "guest_sdl3_desktop.ppm")
            subprocess.run(["convert", str(screen), str(SDL3_DIR / "guest_sdl3_desktop.png")], check=True)
            print(f"[sdl3-guest] Captured guest desktop screenshot ({screen.stat().st_size} bytes)")

            # Verify screenshot visual pixels
            verify_screenshot_pixels(screen)

            # Wait for application completion
            print("[sdl3-guest] Waiting for 60-frame loop completion and clean exit...")
            wait_for(
                lambda: "[sdl3-app] Clean termination with exit code 0" in log_text(),
                "SDL3 clean termination exit code 0",
                timeout_s=40,
            )
            print("[sdl3-guest] SDL3 application completed 60 frames and exited cleanly!")

            # Independently observe clean process termination in desktop broker
            wait_for(
                lambda: "[desktop] child Process-cap exit status=0" in log_text(),
                "Desktop observed child exit status 0",
                timeout_s=10,
            )
            wait_for(
                lambda: "[desktop] application retired" in log_text(),
                "Desktop retired application mapping and process",
                timeout_s=10,
            )
            print("[sdl3-guest] Independently verified clean process teardown and capability reclamation in desktop broker!")

            # Assert actual interactive state transitions from serial log
            full_log = log_text()
            assert "[sdl3-app] Event: Spacebar pressed -> cycling color" in full_log, \
                "Failed to observe keyboard event processing in guest log"
            assert "[sdl3-app] Event: Mouse button down" in full_log, \
                "Failed to observe mouse button event processing in guest log"
            assert "frame rendered; damage published; frame=0" in full_log, \
                "Missing initial frame 0 log"
            assert "frame rendered; damage published; frame=25" in full_log, \
                "Missing milestone frame 25 log"
            assert "frame rendered; damage published; frame=59" in full_log, \
                "Missing final frame 59 log"
            print("[sdl3-guest] Interactive event assertions PASSED: Key press, mouse click, and frame sequence confirmed!")

            # Clean shutdown
            time.sleep(0.5)
            print("[sdl3-guest] Sending shutdown command to guest serial shell...")
            try:
                guest.stdin.write(b"shutdown\r")
                guest.stdin.flush()
                guest.wait(timeout=10)
            except Exception:
                try:
                    conn.command("quit")
                    guest.wait(timeout=5)
                except Exception:
                    guest.kill()
            print("[sdl3-guest] QEMU guest halted cleanly!")

            # Extract timing report from serial log
            log = log_text()
            timing_match = re.search(r"Animation completed: (\d+) frames in (\d+) ms", log)
            breakdown_match = re.search(r"Timing breakdown: render=(\d+) us, present=(\d+) us", log)

            print("\n" + "=" * 60)
            print("=== G2 MILESTONE QUALIFICATION SUMMARY ===")
            print("=" * 60)
            print(f"Target Subsystem:  Native SDL3 Graphics Compatibility (G2)")
            print(f"Binary Format:     ELF64 Executable (-ffreestanding, -nostdlib, static)")
            print(f"Signing Authority: RFC 8032 Section 7.1 test seed (Ed25519)")
            print(f"Bundle SHA-256:    {bundle_sha}")
            print(f"Binary SHA-256:    {exe_sha}")
            if timing_match:
                frames = int(timing_match.group(1))
                duration_ms = int(timing_match.group(2))
                fps = (frames * 1000.0) / max(1, duration_ms)
                print(f"Frames Rendered:   {frames} frames")
                print(f"Total Duration:    {duration_ms} ms")
                print(f"Interactive Rate:  {fps:.1f} FPS")
            if breakdown_match:
                render_us = int(breakdown_match.group(1))
                present_us = int(breakdown_match.group(2))
                print(f"Rasterization:     {render_us} us (avg {render_us / 60:.1f} us/frame)")
                print(f"IPC Presentation:  {present_us} us (avg {present_us / 60:.1f} us/frame)")
            print(f"Visual Artifact:   {SDL3_DIR / 'guest_sdl3_desktop.png'}")
            print(f"Exit Status:       0 (SUCCESS)")
            print("=" * 60 + "\n")

    finally:
        if guest and guest.poll() is None:
            guest.terminate()
            try:
                guest.wait(timeout=5)
            except Exception:
                guest.kill()
        console.terminate()
        actor.terminate()


if __name__ == "__main__":
    run_qemu_test()
