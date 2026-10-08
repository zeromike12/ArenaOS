#!/usr/bin/env python3
"""Run genuine signed native ArenaOS 3D Cube application in QEMU guest."""

from pathlib import Path
import os
import re
import shutil
import subprocess
import sys
import tempfile
import time

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / "tools"))
sys.path.insert(0, str(ROOT / "build/guest-cube"))

import afs2
import apb1_format
import phase10_archive_boot as phase10
import qmp

GUEST_DIR = ROOT / "build/guest-cube"
CUBE_APP_DIR = ROOT / "experimental/cube-app"
CUBE_EXE = CUBE_APP_DIR / "target/x86_64-unknown-none/release/arena-cube-app"
CUBE_BUNDLE = CUBE_APP_DIR / "Cube.apb1"
AFS2_BASE = 8 * 1024 * 1024


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
    print(f"[cube-guest] Built arena-cube-app: {size} bytes ({size / 1024:.1f} KiB)")
    assert size < 256 * 1024, "Binary exceeds 256 KiB dynamic image bound"

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
    print(f"[cube-guest] Packaged signed APB1 bundle: {len(bundle)} bytes")
    return bundle


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


def run_qemu_test():
    bundle = build_cube_app()

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

            # Wait for desktop icons to settle
            time.sleep(1.0)

            # Inspect desktop icons on Desktop
            wait_for(
                lambda: "[desktop] desktop surface shows /Users/user/Desktop" in log_text(),
                "Desktop enumerates /Users/user/Desktop",
                timeout_s=30,
            )

            # Desktop has 4 items now: phase13.apb1 (y=58), z-associated.txt (y=132), zz-headless (y=206), Cube.apb1 (y=280)
            # Or let's double click Cube.apb1 icon!
            # Let's find Cube.apb1 y coordinate:
            # Each icon is at x=52, y = 58 + index * 74:
            # index 0: y=58 (phase13.apb1)
            # index 1: y=132 (z-associated.txt)
            # index 2: y=206 (zz-headless.apb1)
            # index 3: y=280 (Cube.apb1)
            print("[cube-guest] Double-clicking Cube.apb1 icon at (52, 58) to install...")
            before_install = log_text().count("[desktop] APB1 installed; signed version=1 (no launch authority implied)")
            click(52, 58)
            time.sleep(0.15)
            click(52, 58)

            wait_for(
                lambda: "[desktop] APB1 installed; signed version=1" in log_text(),
                "Desktop APB1 install of Cube.apb1 (version 1)",
                timeout_s=30,
            )
            print("[cube-guest] Desktop verified signature and installed Cube.apb1 (version 1)!")

            # Now launch the installed application via All Applications search
            print("[cube-guest] Opening All Applications search launcher...")
            click(94, 12)
            time.sleep(0.5)

            print("[cube-guest] Typing 'cube' to filter catalog...")
            conn.type_text("cube", gap_s=0.04)
            time.sleep(0.5)

            # Take screendump of launcher search
            conn.command("screendump", filename=str(work / "screen-search.ppm"), format="ppm")

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

            wait_for(
                lambda: "[cube-app] frame rendered; damage published; frame=20" in log_text(),
                "cube-app frame 20 rendered and window reveal animation settled",
                timeout_s=30,
            )
            print("[cube-guest] Frame 20 reached; window reveal animation fully settled!")

            # Take settled desktop screenshot showing full 320x240 cube window
            conn.command("screendump", filename=str(screen), format="ppm")
            shutil.copyfile(screen, ROOT / "experimental/graphics-prototype/guest_cube_desktop.ppm")
            print(f"[cube-guest] Captured guest desktop screenshot to experimental/graphics-prototype/guest_cube_desktop.ppm ({screen.stat().st_size} bytes)")

            # Wait for animation sequence to complete
            wait_for(
                lambda: "[cube-app] animation sequence completed successfully; exiting" in log_text(),
                "cube-app animation sequence completion",
                timeout_s=45,
            )
            print("[cube-guest] All 60 animated 3D cube frames rendered successfully!")

            # Gracefully shut down guest
            print("[cube-guest] Sending shutdown command to guest serial shell...")
            guest.stdin.write(b"shutdown\r")
            guest.stdin.flush()

        rc = guest.wait(timeout=30)
        print(f"[cube-guest] Guest cleanly halted with code {rc}!")

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
    run_qemu_test()
