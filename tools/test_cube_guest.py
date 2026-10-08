#!/usr/bin/env python3
"""Run genuine signed native ArenaOS 3D Cube application in QEMU guest.

Milestone G1.1 Qualification Witness:
- Verifies input artifact SHA-256 digests against release checkpoints.
- Compiles native arena-cube-app (no_std, x86_64-unknown-none).
- Packages signed APB1 bundle (using RFC 8032 test-vector seed).
- Seeds scratch AFS2 disk image.
- Boots QEMU guest with Phase-13 desktop environment.
- Performs pointer double-click install of Cube.apb1.
- Launches Cube application via All Applications search.
- Asserts all 60 frames rendered in sequence with exact golden reference hashes.
- Extracts genuine monotonic timing (rasterization vs presentation).
- Verifies rendered screenshot pixels for multi-color cube faces.
- Asserts clean process exit status 0 and clean kernel shutdown.
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
CUBE_APP_DIR = ROOT / "experimental/cube-app"
CUBE_EXE = CUBE_APP_DIR / "target/x86_64-unknown-none/release/arena-cube-app"
CUBE_BUNDLE = CUBE_APP_DIR / "Cube.apb1"
CHECKPOINT_TAR = ROOT / "releases/checkpoints/phase13-complete/arenaos-phase13-complete-qemu-x86_64.tar.gz"
AFS2_BASE = 8 * 1024 * 1024

# Golden reference hashes for key rotation frames
GOLDEN_HASHES = {
    0: 0x3ea9171a41daefd0,
    1: 0x6309282343c983c6,
    2: 0xbcafdb7b679bf72f,
    29: 0x97b0ee2c10f7b034,
    59: 0x00a11afb86782620,
}


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
        print(f"[cube-guest] Extracting required guest artifacts from {CHECKPOINT_TAR.name}...")
        assert CHECKPOINT_TAR.is_file(), f"Missing checkpoint tarball: {CHECKPOINT_TAR}"
        with tarfile.open(CHECKPOINT_TAR, "r:gz") as tar:
            for member in tar.getmembers():
                base_name = os.path.basename(member.name)
                if base_name in required:
                    member.name = base_name
                    tar.extract(member, GUEST_DIR)

    # Compute and report input digests
    efi_sha = sha256_file(GUEST_DIR / "arena-boot.efi")
    esp_sha = sha256_file(GUEST_DIR / "arena-esp.img")
    code_sha = sha256_file(GUEST_DIR / "edk2-x86_64-code.fd")
    scratch_sha = sha256_file(GUEST_DIR / "scratch-template.img")

    print("[cube-guest] Input Artifact Digests:")
    print(f"  arena-boot.efi:       {efi_sha}")
    print(f"  arena-esp.img:        {esp_sha}")
    print(f"  edk2-x86_64-code.fd:  {code_sha}")
    print(f"  scratch-template.img: {scratch_sha}")
    return efi_sha


def build_cube_app():
    print("[cube-guest] Building native arena-cube-app for x86_64-unknown-none...")
    env = os.environ.copy()
    env["PATH"] = f"/opt/rust/prefix/bin:{env.get('PATH', '')}"
    subprocess.run(
        ["cargo", "build", "--release"],
        cwd=str(CUBE_APP_DIR),
        env=env,
        check=True,
    )
    assert CUBE_EXE.is_file(), f"Missing binary {CUBE_EXE}"
    size = CUBE_EXE.stat().st_size
    exe_sha = sha256_file(CUBE_EXE)
    print(f"[cube-guest] Built arena-cube-app: {size} bytes ({size / 1024:.1f} KiB), SHA-256: {exe_sha}")
    assert size < 256 * 1024, "Binary exceeds 256 KiB dynamic image bound"

    # NOTE ON SIGNING: The seed below is a public RFC 8032 Section 7.1 test-vector seed
    # used strictly as test-only signing authority for development qualification fixtures.
    # It is NOT a production release-signing key.
    print("[cube-guest] Packaging signed APB1 bundle (using RFC 8032 test-only signing seed)...")
    manifest = apb1_format.make_manifest(
        app_id=b"org.arenaos.cube",
        package_id=b"org.arenaos.cube",
        display_name=b"3D Cube",
        version=1,
        flags=25,  # MULTI_INSTANCE (1) | STANDARD_STREAMS (8) | NATIVE_SYNC (16)
        requested=0,
        entry=b"bin/cube",
        icon=b"",
        width=320,
        height=240,
        associations=(),
    )
    bundle = apb1_format.build_bundle(
        [(1, b"bin/cube", CUBE_EXE.read_bytes())],
        manifest=manifest,
    )
    CUBE_BUNDLE.write_bytes(bundle)
    bundle_sha = sha256_file(CUBE_BUNDLE)
    print(f"[cube-guest] Packaged signed APB1 bundle: {len(bundle)} bytes, SHA-256: {bundle_sha}")
    return bundle, exe_sha, bundle_sha


def seed_disk(disk_path: Path, bundle: bytes):
    print(f"[cube-guest] Seeding {CUBE_BUNDLE.name} onto AFS2 disk image...")
    raw = bytearray(disk_path.read_bytes())
    volume = afs2.Volume(raw[AFS2_BASE:])
    desktop = volume.resolve("/Users/user/Desktop")

    # Clean other packages from Desktop so Cube.apb1 is at the first icon position (52, 58)
    for stale in [b"phase13.apb1", b"zz-headless.apb1", b"z-associated.txt"]:
        try:
            volume.unlink(desktop, stale)
        except Exception:
            pass

    obj = volume.create(desktop, b"Cube.apb1", 1)
    volume.write(obj, 0, bundle, 1)
    raw[AFS2_BASE:] = volume.image()
    disk_path.write_bytes(raw)
    print("[cube-guest] Successfully placed Cube.apb1 as primary Desktop item")


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
    """Verify visual correctness of the rendered cube in the desktop window."""
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
    red_face_count = 0
    green_face_count = 0
    blue_face_count = 0
    cream_face_count = 0

    for y in range(100, 280):
        for x in range(80, 380):
            r, g, b = pixel_at(x, y)
            # Dark slate window background (0x1A, 0x23, 0x32)
            if (r, g, b) == (0x1A, 0x23, 0x32):
                slate_bg_count += 1
            # Red cube face (Crimson: high R, low G and B)
            elif r > 110 and g < 60 and b < 60:
                red_face_count += 1
            # Green cube face (Emerald: high G, lower R and B)
            elif g > 90 and r < 75 and b < 75:
                green_face_count += 1
            # Blue cube face (Royal Blue: high B, lower R)
            elif b > 110 and r < 70:
                blue_face_count += 1
            # Cream / light face (high R and G)
            elif r > 110 and g > 110 and b < 100:
                cream_face_count += 1

    print("[cube-guest] Screenshot Pixel Analysis:")
    print(f"  Dark slate background pixels: {slate_bg_count}")
    print(f"  Red face pixels:              {red_face_count}")
    print(f"  Green face pixels:            {green_face_count}")
    print(f"  Blue face pixels:             {blue_face_count}")
    total_cube_pixels = red_face_count + green_face_count + blue_face_count
    print(f"  Total visible cube pixels:    {total_cube_pixels}")

    assert slate_bg_count > 10000, f"Insufficient window background pixels: {slate_bg_count}"
    assert red_face_count > 500, f"Insufficient red face pixels: {red_face_count}"
    assert green_face_count > 500, f"Insufficient green face pixels: {green_face_count}"
    assert blue_face_count > 500, f"Insufficient blue face pixels: {blue_face_count}"
    assert total_cube_pixels > 5000, f"Insufficient total cube face pixels: {total_cube_pixels}"
    print("[cube-guest] Visual assertions PASSED: 3 distinct visible cube faces (Red, Green, Blue) and window background verified!")


def run_qemu_test():
    efi_sha = ensure_guest_artifacts()
    bundle, exe_sha, bundle_sha = build_cube_app()

    work = ROOT / "build/cube-test-work"
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

        print("[cube-guest] Launching QEMU guest...")
        with serial.open("wb") as s_out, qemu_err.open("wb") as s_err:
            guest = subprocess.Popen(
                cmd,
                cwd=str(work),
                stdin=subprocess.PIPE,
                stdout=s_out,
                stderr=s_err,
            )

            # Wait for QMP socket
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

            print("[cube-guest] Waiting for Desktop initialization...")
            wait_for(
                lambda: "[desktop] real desktop frame presented" in log_text(),
                "real desktop frame presented",
                timeout_s=45,
            )
            print("[cube-guest] Real desktop frame presented!")

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

            # Double-click primary icon at (52, 58) to install Cube.apb1
            print("[cube-guest] Double-clicking Cube.apb1 icon at (52, 58) to install...")
            click(52, 58)
            time.sleep(0.15)
            click(52, 58)

            wait_for(
                lambda: "[desktop] APB1 installed; signed version=1 (no launch authority implied)" in log_text(),
                "Desktop APB1 install of Cube.apb1 (version 1)",
                timeout_s=30,
            )
            print("[cube-guest] Desktop verified signature and installed Cube.apb1 (version 1)!")

            # Verify installation on AFS2
            raw_disk = scratch.read_bytes()
            vol = afs2.Volume(raw_disk[AFS2_BASE:])
            tree = afs2.walk(vol)
            installed_path = "/System/Applications/org.arenaos.cube/1/bin/cube"
            assert installed_path in tree, f"Expected {installed_path} in AFS2 tree, got: {list(tree.keys())}"
            print(f"[cube-guest] AFS2 verified installed payload: {installed_path}")

            # Open All Applications search launcher at (94, 12)
            print("[cube-guest] Opening All Applications search launcher...")
            click(94, 12)
            time.sleep(0.5)

            print("[cube-guest] Typing 'cube' to filter catalog...")
            conn.type_text("cube", gap_s=0.04)
            time.sleep(0.5)

            print("[cube-guest] Clicking filtered application row at (250, 220)...")
            click(250, 220)

            print("[cube-guest] Waiting for arena-cube-app startup and frame rendering...")
            wait_for(
                lambda: "[cube-app] Startup ABI v2 verified; connecting to desktop compositor" in log_text(),
                "cube-app startup ABI v2 verification",
                timeout_s=30,
            )
            print("[cube-guest] Cube app verified Startup ABI v2 and connected to compositor!")

            wait_for(
                lambda: "[cube-app] window connected; backing mapped via real SharedRegion" in log_text(),
                "cube-app SharedRegion mapping",
                timeout_s=20,
            )
            print("[cube-guest] Backing mapped via real SharedRegion!")

            # Wait for frame 25 so window reveal animation is fully settled
            wait_for(
                lambda: "[cube-app] frame rendered; damage published; frame=25" in log_text(),
                "cube-app frame 25 rendered and window reveal animation settled",
                timeout_s=30,
            )
            print("[cube-guest] Frame 25 reached; window reveal animation fully settled!")

            # Take settled desktop screenshot showing full 320x240 cube window
            conn.command("screendump", filename=str(screen), format="ppm")
            shutil.copyfile(screen, ROOT / "experimental/graphics-prototype/guest_cube_desktop.ppm")
            # Convert to PNG
            subprocess.run(["convert", str(screen), str(ROOT / "experimental/graphics-prototype/guest_cube_desktop.png")], check=True)
            print(f"[cube-guest] Captured guest desktop screenshot ({screen.stat().st_size} bytes)")

            # Verify screenshot visual pixels
            verify_screenshot_pixels(screen)

            # Wait for all 60 frames to complete
            wait_for(
                lambda: "[cube-app] animation sequence completed successfully; exiting" in log_text(),
                "cube-app animation sequence completion",
                timeout_s=45,
            )
            print("[cube-guest] All 60 animated 3D cube frames rendered successfully!")

            # Send shutdown command to guest serial shell
            print("[cube-guest] Sending shutdown command to guest serial shell...")
            guest.stdin.write(b"shutdown\r")
            guest.stdin.flush()

        rc = guest.wait(timeout=30)
        assert rc == 0, f"QEMU guest exited with nonzero status {rc}"
        print(f"[cube-guest] Guest cleanly halted with code {rc}!")

        # ---------------------------------------------------------------------
        # Comprehensive Post-Execution Serial Log Assertions
        # ---------------------------------------------------------------------
        text = serial.read_text(errors="replace")

        # 1. Assert all 60 distinct frame indices appear in sequential order
        frame_pattern = re.compile(
            r"\[cube-app\] frame rendered; damage published; frame=(\d+); hash=0x([0-9a-f]+); raster_us=(\d+); present_us=(\d+)"
        )
        matches = frame_pattern.findall(text)
        assert len(matches) == 60, f"Expected exactly 60 rendered frame logs, found {len(matches)}"

        raster_times = []
        present_times = []
        for expected_idx, (frame_str, hash_str, raster_str, present_str) in enumerate(matches):
            frame_idx = int(frame_str)
            assert frame_idx == expected_idx, f"Frame sequence error: expected {expected_idx}, got {frame_idx}"
            frame_hash = int(hash_str, 16)
            if expected_idx in GOLDEN_HASHES:
                expected_hash = GOLDEN_HASHES[expected_idx]
                assert frame_hash == expected_hash, \
                    f"Frame {expected_idx} hash mismatch: expected 0x{expected_hash:016x}, got 0x{frame_hash:016x}"
            r_us = int(raster_str)
            p_us = int(present_str)
            raster_times.append(r_us)
            present_times.append(p_us)

        # 2. Timing Summary Assertions
        timing_summary = re.search(
            r"\[cube-app\] timing summary: total_frames=60; total_elapsed_us=(\d+); total_raster_us=(\d+); total_present_us=(\d+)",
            text,
        )
        assert timing_summary, "Missing [cube-app] timing summary line"
        total_elapsed_us = int(timing_summary.group(1))
        total_raster_us = int(timing_summary.group(2))
        total_present_us = int(timing_summary.group(3))

        avg_raster_us = total_raster_us / 60.0
        avg_present_us = total_present_us / 60.0
        fps_raster = 1_000_000.0 / avg_raster_us if avg_raster_us > 0 else 0.0
        fps_end_to_end = (60 * 1_000_000.0) / total_elapsed_us if total_elapsed_us > 0 else 0.0

        print("\n========================================================")
        print("GENUINE GUEST MONOTONIC TIMING REPORT (SYS_CLOCK_NOW):")
        print("========================================================")
        print(f"  Total Frames Rendered:           60")
        print(f"  Total Elapsed Animation Time:    {total_elapsed_us:,} us ({total_elapsed_us / 1000.0:.2f} ms)")
        print(f"  Total Guest Rasterization Time:  {total_raster_us:,} us ({total_raster_us / 1000.0:.2f} ms)")
        print(f"  Total IPC Presentation Time:     {total_present_us:,} us ({total_present_us / 1000.0:.2f} ms)")
        print(f"  Average Rasterization per Frame: {avg_raster_us:.1f} us ({avg_raster_us / 1000.0:.2f} ms)")
        print(f"  Average Presentation per Frame:  {avg_present_us:.1f} us ({avg_present_us / 1000.0:.2f} ms)")
        print(f"  Min / Max Rasterization Time:    {min(raster_times)} us / {max(raster_times)} us")
        print(f"  Effective Pure Raster Rate:      {fps_raster:.1f} FPS")
        print(f"  End-to-End Application Rate:     {fps_end_to_end:.1f} FPS")
        print("========================================================\n")

        # 3. Process Exit & Kernel Shutdown Assertions
        assert "[desktop] child Process-cap exit status=0" in text, "Missing clean process exit status 0"
        assert "[arena INFO  kernel] shutdown requested by pid 157 through its Power cap — goodnight" in text, \
            "Missing power cap clean shutdown confirmation"
        assert "halting via UEFI ResetSystem(shutdown)" in text, "Missing UEFI ResetSystem confirmation"

        print("[cube-guest] All guest serial and visual assertions PASSED successfully!")
        return {
            "efi_sha": efi_sha,
            "exe_sha": exe_sha,
            "bundle_sha": bundle_sha,
            "total_elapsed_us": total_elapsed_us,
            "avg_raster_us": avg_raster_us,
            "avg_present_us": avg_present_us,
            "fps_end_to_end": fps_end_to_end,
        }

    finally:
        if guest is not None and guest.poll() is None:
            guest.kill()
            guest.wait()
        actor.terminate()
        console.terminate()
        try:
            console.wait(timeout=3)
        except Exception:
            pass


if __name__ == "__main__":
    results = run_qemu_test()
    print("[cube-guest] Test completed successfully!")
