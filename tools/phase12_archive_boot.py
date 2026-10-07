#!/usr/bin/env python3
"""Boot an extracted Phase-12 archive and qualify its native platform.

This entry point uses only files beside this script plus a local QEMU
installation. The delegated Phase-10 witness checks the exact EFI receipt,
keyboard, desktop pixels, native app lifecycle, network/console fixtures and
clean shutdown. The callback then installs the bundled signed APB1 fixture
through the real Desktop icon and verifies AFS2 readback from the attached
disk image.
"""
import sys
import time
from pathlib import Path

import afs1
import afs2
import apb1_format
import phase10_archive_boot as witness
import qmp

ROOT = Path(__file__).resolve().parent
AFS2_BASE = 8 * 1024 * 1024
SOURCE_PATH = "/Users/user/Desktop/editor.apb1"

PHASE12_MARKERS = (
    "filesd: AFS2 mounted",
    "[desktop] AFS2 file service online",
    "servicemgr: extended cap occupancy [32,127)=0",
    "servicemgr: reserved APB1 slot127 descriptor=1 kind=12 rights=6",
)


def install_and_verify(sock: Path, serial: Path, _image: Path, disk: Path) -> str:
    bundle = (ROOT / "editor.apb1").read_bytes()
    parsed = apb1_format.parse_bundle(bundle)
    version = int.from_bytes(parsed.metadata[64 + 104:64 + 112], "little")
    install_prefix = f"/System/Applications/com.arena.editor/{version}/"
    text = lambda: serial.read_text(errors="replace") if serial.exists() else ""

    def wait(predicate, message: str, timeout_s: float = 90) -> None:
        end = time.monotonic() + timeout_s
        while time.monotonic() < end:
            if predicate():
                return
            time.sleep(0.05)
        raise TimeoutError(message)

    # The earlier keyboard and native-app lifecycle witness can legitimately
    # update desktop preferences in AFS1. Snapshot after those actions and
    # compare across the protected AFS2 package install itself.
    before_afs1 = disk.read_bytes()[:AFS2_BASE]
    before = text().count("[desktop] APB1 installed; signed version=")
    conn = qmp.Qmp(str(sock), connect_timeout_s=5)

    def point(x: int, y: int, down: bool | None = None) -> None:
        events = [
            {"type": "abs", "data": {"axis": "x", "value": (x * 32767 + 799) // 799}},
            {"type": "abs", "data": {"axis": "y", "value": (y * 32767 + 599) // 599}},
        ]
        if down is not None:
            events.append({"type": "btn", "data": {"button": "left", "down": down}})
        conn.command("input-send-event", events=events)

    def click(x: int, y: int) -> None:
        point(x, y, True)
        point(x, y, False)
        point(780, 500)

    try:
        wait(lambda: "[desktop] desktop surface shows /Users/user/Desktop" in text(),
             "Desktop did not list the seeded APB1 fixture")
        click(52, 58)
        time.sleep(0.1)
        click(52, 58)
        wait(lambda: text().count("[desktop] APB1 installed; signed version=") == before + 1,
             "real Desktop APB1 action did not finish")
    finally:
        conn.close()

    guest_serial = text()
    for marker in (
        "[desktop] APB1 authority boundary: ordinary Filesd capability denied protected install",
        "packaged: APB1 scope proof: install-only endpoint denied generic LIST",
        "packaged: APB1 scope proof: same-kind capability with wrong rights denied (kind=12 rights=14)",
        "servicemgr: APB1 wrong-kind handoff refused kind=3 rights=5",
    ):
        if marker not in guest_serial:
            raise AssertionError(f"extracted guest omitted protected-install proof: {marker}")

    raw_disk = disk.read_bytes()
    audit = afs1.audit(disk)
    if audit:
        raise AssertionError(f"extracted guest left AFS1 structurally invalid: {audit}")
    if raw_disk[:AFS2_BASE] != before_afs1:
        raise AssertionError("APB1 installation modified the AFS1 region")
    volume = afs2.Volume(raw_disk[AFS2_BASE:])
    afs2.check(volume)
    tree = afs2.walk(volume)
    if tree.get(SOURCE_PATH) != bundle:
        raise AssertionError("Desktop APB1 source changed during install")

    expected_files = {}
    payload_offset = parsed.payload_offset
    for _kind, path, size, _digest in parsed.files:
        name = path.decode("utf-8")
        expected_files[install_prefix + name] = bundle[payload_offset:payload_offset + size]
        payload_offset += size
    for path, content in expected_files.items():
        if tree.get(path) != content:
            raise AssertionError(f"installed AFS2 readback differs: {path}")
    if tree.get(install_prefix + "APB1.record") != parsed.metadata + parsed.signature:
        raise AssertionError("installed APB1 signed record did not read back exactly")
    if "/System/.apb1-staging" not in tree or any(
        path.startswith("/System/.apb1-staging/") for path in tree
    ):
        raise AssertionError("private APB1 staging tree was not empty after activation")

    return (
        f"APB1 version={version} bundle-sha256={parsed.bundle_digest.hex()} "
        f"signed-record=PASS payload-files={len(expected_files)} verified-readback=PASS "
        "source-preserved=PASS AFS1-preserved=PASS"
    )


def main() -> None:
    if sys.argv[1:] not in ([], ["--unqualified-smoke"]):
        raise ValueError("usage: phase12_archive_boot.py [--unqualified-smoke]")
    qualified = not sys.argv[1:]
    witness.boot(
        qualified=qualified,
        extra_markers=PHASE12_MARKERS,
        label="PHASE12",
        after=install_and_verify,
        launches=2,
    )


if __name__ == "__main__":
    try:
        main()
    except (OSError, ValueError, RuntimeError, AssertionError, TimeoutError) as exc:
        sys.exit(f"EXTRACTED PHASE12 BOOT FAILED: {exc}")
