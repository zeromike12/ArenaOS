#!/usr/bin/env python3
"""Extracted Phase-13 installed-app/runtime witness.

The witness uses the bundled EFI, firmware, scratch AFS2 image, signed APB1
fixtures, Python helpers, and a local QEMU binary. It does not read the source
checkout or a build directory. The guest performs real installation,
registry-backed Open With/All Applications launches, window/helper/stream/
thread work, headless reap, and identity-resource teardown.
"""
from __future__ import annotations

import hashlib
import re
import sys
import time
from pathlib import Path

import afs1
import afs2
import apb1_format
import phase10_archive_boot as phase10
import qmp

ROOT = Path(__file__).resolve().parent
AFS2_BASE = 8 * 1024 * 1024
APP_ID = "org.arenaos.phase13app"
HEADLESS_ID = "org.arenaos.zzheadless"
APP_SOURCE = "/Users/user/Desktop/phase13.apb1"
HEADLESS_SOURCE = "/Users/user/Desktop/zz-headless.apb1"
DOCUMENT = "/Users/user/Desktop/z-associated.txt"
DOCUMENT_BYTES = b"Phase 13 associated document\n"

COUNTERS = re.compile(
    r"measured frames/records/processes/regions/pages/maps/caps="
    r"(\d+)/(\d+)/(\d+)/(\d+)/(\d+)/(\d+)/(\d+)\r?\n"
)
DETAILS = re.compile(
    r"native resource detail values=" + r"(\d+(?:/\d+){14})\r?\n"
)


def wait(predicate, message: str, timeout_s: float = 90) -> None:
    deadline = time.monotonic() + timeout_s
    while time.monotonic() < deadline:
        if predicate():
            return
        time.sleep(0.05)
    raise TimeoutError(message)


def serial_text(serial: Path) -> str:
    return serial.read_text(errors="replace") if serial.exists() else ""


def disk_tree(disk: Path) -> dict[str, bytes]:
    deadline = time.monotonic() + 10
    last_error: Exception | None = None
    while time.monotonic() < deadline:
        try:
            raw = disk.read_bytes()
            volume = afs2.Volume(raw[AFS2_BASE:])
            afs2.check(volume)
            return afs2.walk(volume)
        except Exception as exc:  # AFS2 pages may be between durable updates.
            last_error = exc
            time.sleep(0.1)
    raise AssertionError(f"AFS2 guest image did not settle for host readback: {last_error}")


def parse_ppm(path: Path) -> bytes:
    raw = path.read_bytes()
    match = re.match(rb"P6\s+(\d+)\s+(\d+)\s+255\s", raw)
    if not match or (int(match[1]), int(match[2])) != (800, 600):
        raise ValueError("extracted guest did not produce the expected 800x600 PPM")
    pixels = raw[match.end():]
    if len(pixels) != 800 * 600 * 3:
        raise ValueError("extracted guest screenshot has a truncated pixel buffer")
    return pixels


def crop(pixels: bytes, x: int, y: int, width: int, height: int) -> bytes:
    return b"".join(
        pixels[((y + row) * 800 + x) * 3:((y + row) * 800 + x + width) * 3]
        for row in range(height)
    )


def marker_set(pixels: bytes) -> bool:
    points = ((80, 100), (130, 150), (156, 170))
    expected = (bytes((0xE0, 0x40, 0x20)), bytes((0xF0, 0xD0, 0x20)), bytes((0x20, 0xD0, 0x80)))
    return tuple(
        pixels[(y * 800 + x) * 3:(y * 800 + x) * 3 + 3] for x, y in points
    ) == expected


def callback(sock: Path, serial: Path, image: Path, disk: Path) -> str:
    log = lambda: serial_text(serial)
    conn = qmp.Qmp(str(sock), connect_timeout_s=5)
    wait(lambda: bool(COUNTERS.findall(log())) and bool(DETAILS.findall(log())),
         "native resource baseline receipts were not emitted")
    boot_counts = tuple(map(int, COUNTERS.findall(log())[-1]))
    boot_detail = tuple(map(int, DETAILS.findall(log())[-1].split("/")))

    def point(x: int, y: int, button: str | None = None, down: bool | None = None) -> None:
        events = [
            {"type": "abs", "data": {"axis": "x", "value": (x * 32767 + 799) // 799}},
            {"type": "abs", "data": {"axis": "y", "value": (y * 32767 + 599) // 599}},
        ]
        if button is not None and down is not None:
            events.append({"type": "btn", "data": {"button": button, "down": down}})
        conn.command("input-send-event", events=events)

    def click(x: int, y: int) -> None:
        point(x, y, "left", True)
        point(x, y, "left", False)
        point(780, 500)

    def key(qcode: str) -> None:
        conn.command("input-send-event", events=[qmp.Qmp._ev(qcode, True), qmp.Qmp._ev(qcode, False)])

    def chord(qcode: str) -> None:
        conn.command(
            "input-send-event",
            events=[qmp.Qmp._ev("ctrl", True), qmp.Qmp._ev(qcode, True),
                    qmp.Qmp._ev(qcode, False), qmp.Qmp._ev("ctrl", False)],
        )

    def right_click(x: int, y: int) -> None:
        point(x, y)
        point(x, y, "right", True)
        point(x, y, "right", False)

    def screenshot() -> bytes:
        conn.command("screendump", filename=str(image), format="ppm")
        return parse_ppm(image)

    def app_install(source_y: int, expected_count: int, expected_source: str, app_id: str) -> None:
        click(52, source_y)
        time.sleep(0.1)
        click(52, source_y)
        wait(lambda: log().count("[desktop] APB1 installed; signed version=") == expected_count,
             f"Desktop did not install {app_id} through protected APB1 activation")
        tree = disk_tree(disk)
        source = tree.get(expected_source)
        bundle = (ROOT / ("phase13.apb1" if app_id == APP_ID else "zz-headless.apb1")).read_bytes()
        if source != bundle:
            raise AssertionError(f"AFS2 changed the original APB1 source: {expected_source}")
        parsed = apb1_format.parse_bundle(bundle)
        version = int.from_bytes(parsed.metadata[64 + 104:64 + 112], "little")
        prefix = f"/System/Applications/{app_id}/{version}/"
        offset = parsed.payload_offset
        for _kind, name, size, _digest in parsed.files:
            entry = prefix + name.decode("utf-8")
            if tree.get(entry) != bundle[offset:offset + size]:
                raise AssertionError(f"AFS2 installed payload mismatch: {entry}")
            offset += size
        if tree.get(prefix + "APB1.record") != parsed.metadata + parsed.signature:
            raise AssertionError(f"AFS2 installed signed record mismatch: {app_id}")

    try:
        wait(lambda: "[desktop] desktop surface shows /Users/user/Desktop" in log(),
             "Desktop did not enumerate the seeded APB1 fixtures")
        app_install(58, 1, APP_SOURCE, APP_ID)
        app_install(206, 2, HEADLESS_SOURCE, HEADLESS_ID)
        audit = afs1.audit(disk)
        if audit:
            raise AssertionError(f"APB1 installation damaged the AFS1 import area: {audit}")
        if disk_tree(disk).get(DOCUMENT) != DOCUMENT_BYTES:
            raise AssertionError("seeded document does not match the read-only handoff fixture")

        # Open With saves only a descriptive default, then separately offers
        # the selected document as a read-only File capability.
        right_click(52, 132)
        click(72, 172)
        conn.type_text("phase13", gap_s=0.025)
        chord("d")
        wait(lambda: "[desktop] AFS2 application handler default saved" in log(),
             "Open With did not persist the selected text handler")
        tree = disk_tree(disk)
        assoc = tree.get("/Users/user/.arena-app-associations", b"")
        if assoc[:4] != b"ASOC" or APP_ID.encode() not in assoc:
            raise AssertionError("the Open With default was not persisted in AFS2")
        key("ret")
        wait(lambda: "[phase13-app] exact read-only document capability verified" in log(),
             "Open With app did not receive and verify its exact read-only File capability")
        for marker, description in (
            ("[phase13-vm] guarded reserve, lazy commit, RW/RO/RX protection, W^X refusal, exact release/accounting passed", "VM"),
            ("[phase13-heap] lazy 16 MiB VM heap, 256 KiB Vec, 64-page commit batches, reuse, 64 KiB alignment, fallible OOM passed", "heap"),
            ("[phase13-threads] four concurrent ring-3 threads shared heap and read-only VM; distinct FS.base TLS, quota, exact stack caps, join/detach, and cleanup passed", "user-thread"),
            ("[phase13-sync] contended Mutex, multi-waiter Condvar wake-one/all, sequence-before-wait, Once contention, timeout, invalid-cap refusal, and key accounting passed", "synchronization"),
            ("[phase13-helper-stream] owner woke child; exact stdin/stdout bytes transferred; EOF after reap", "helper byte stream"),
            ("[phase13-helper] crashed helper status=262 observed and reaped", "helper crash and reap"),
            ("[phase13-helper] unknown signed helper ID refused without spawn", "helper allowlist refusal"),
            ("[desktop] native stream channel stdout reached EOF", "stdout EOF"),
            ("[desktop] native stream channel stderr reached EOF", "stderr EOF"),
        ):
            wait(lambda marker=marker: marker in log(), f"installed document app omitted {description} proof")

        # The All Applications search is backed by the same verified catalog.
        # Pointer launch creates a distinct ordinary AppInstance; its ELF
        # creates three independently backed compositor windows.
        before_launcher = screenshot()
        click(120, 12)
        wait(lambda: crop(screenshot(), 184, 120, 432, 356)
             != crop(before_launcher, 184, 120, 432, 356),
             "All Applications did not open its registry-backed surface")
        conn.type_text("phase13", gap_s=0.025)
        searched = screenshot()
        if crop(searched, 184, 120, 432, 356) == crop(before_launcher, 184, 120, 432, 356):
            raise AssertionError("All Applications search did not alter its real UI surface")
        click(250, 220)
        wait(lambda: "[phase13-installed-app] ABI-v2 startup verified; real window published" in log(),
             "All Applications did not launch the installed native ELF", timeout_s=60)
        wait(lambda: "[phase13-multiwindow] one process owns three separately backed ordinary windows" in log(),
             "installed AppInstance did not create three ordinary compositor windows", timeout_s=60)
        wait(lambda: marker_set(screenshot()),
             "three independently published window surfaces were not visible")
        for marker in (
            "[phase13-stream] output full; extra write returned WouldBlock",
            "[phase13-stream] stdout partial transfer reached the broker",
            "[phase13-stream] stderr channel reached the broker",
        ):
            wait(lambda marker=marker: log().count(marker) >= 1,
                 f"installed app omitted native stream proof: {marker}")
        conn.type_text("native-stream", gap_s=0.025)
        key("ret")
        wait(lambda: "[phase13-stream] stdin received exact native keyboard bytes" in log(),
             "Startup ABI standard input did not carry real keyboard bytes")

        # Launch the signed headless package by its registry metadata and
        # observe its exact Process-cap exit. It owns no window or Desktop cap.
        before_detail = [tuple(map(int, row.split("/"))) for row in DETAILS.findall(log())][-1]
        click(120, 12)
        conn.type_text("runtime", gap_s=0.025)
        key("ret")
        wait(lambda: "[phase13-headless] Startup ABI v2 verified one attenuated Notification and one SyncDomain; no window caps present" in log(),
             "verified headless package did not launch from All Applications")
        wait(lambda: "[desktop] verified headless application spawned; ordinary windows=0; no surface or Desktop endpoint inherited" in log(),
             "headless launch inherited a window or Desktop endpoint")
        wait(lambda: "[phase13-headless] timer completed; process exiting for manager reap" in log(),
             "headless timer did not complete")
        wait(lambda: "[desktop] child Process-cap exit status=42" in log(),
             "application manager did not observe headless child exit through its Process cap")
        latest_detail = [tuple(map(int, row.split("/"))) for row in DETAILS.findall(log())][-1]
        if latest_detail[11] != before_detail[11]:
            raise AssertionError("headless app changed ordinary window count")

        # Close the document window and all three ordinary windows. Both app
        # groups release child processes, caps, stream objects, VM regions,
        # user stacks, and parked workers before clean shutdown.
        wait(lambda: bool(COUNTERS.findall(log())) and bool(DETAILS.findall(log())),
             "native resource receipts were not emitted")
        for index in range(4):
            before_windows = tuple(map(int, DETAILS.findall(log())[-1].split("/")))[11]
            key("f8")
            wait(lambda before_windows=before_windows:
                 tuple(map(int, DETAILS.findall(log())[-1].split("/")))[11] == before_windows - 1,
                 f"window close {index + 1}/4 did not retire one ordinary surface", timeout_s=45)
        wait(lambda: tuple(map(int, COUNTERS.findall(log())[-1]))[1:] == boot_counts[1:],
             "installed AppInstance/helper resources did not return to boot identity baseline", timeout_s=90)
        final_detail = tuple(map(int, DETAILS.findall(log())[-1].split("/")))
        if final_detail[1:] != boot_detail[1:]:
            raise AssertionError(f"native thread/VM/IPC/window counts did not return to baseline: {boot_detail} -> {final_detail}")
        if "[phase13-threads] closing the document window woke and joined both live workers" not in log():
            raise AssertionError("document close did not wake and join its two live user threads")
        if "[desktop] AppInstance ProcessGroup teardown members=2; final members=0" not in log():
            raise AssertionError("installed application's helper was not reaped with its AppInstance")
        if disk_tree(disk).get(DOCUMENT) != DOCUMENT_BYTES:
            raise AssertionError("read-only handler changed its source document")
        return (
            "two signed APB1 packages installed and exact AFS2 payloads verified; "
            "Open With read-only File cap and AFS2 default; All Applications search and native launch; "
            "three real windows; helper/stream/thread/VM/synchronization proofs; headless Process-cap reap; "
            "four windows and identity resources returned to baseline"
        )
    finally:
        conn.close()


def main() -> None:
    if sys.argv[1:] not in ([], ["--unqualified-smoke"]):
        raise ValueError("usage: phase13_archive_boot.py [--unqualified-smoke]")
    qualified = not sys.argv[1:]
    phase10.boot(
        qualified=qualified,
        label="PHASE13",
        after=callback,
        launches=4,
        retirements=5,
    )


if __name__ == "__main__":
    try:
        main()
    except Exception as exc:
        # Do not hide a real fixture failure behind a traceback in an archive
        # user's terminal; the message is also preserved by the boot wrapper.
        sys.exit(f"EXTRACTED PHASE13 BOOT FAILED: {exc}")
