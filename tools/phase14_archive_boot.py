#!/usr/bin/env python3
"""Independent extracted Phase-14 static PIE and preservation witness.

This script uses only files beside itself plus Python 3 and QEMU. It first
runs the Phase-13 installed native application witness, then installs the
checked-in signed Phase-14 APB1 through the live Desktop/filesd/packaged path,
verifies its protected AFS2 record, and launches it twice from All Applications.
"""
from __future__ import annotations

import hashlib
import re
import sys
import time
from pathlib import Path

import afs2
import apb1_format
import phase10_archive_boot as phase10
import phase13_archive_boot as phase13
import qmp

ROOT = Path(__file__).resolve().parent
AFS2_BASE = 8 * 1024 * 1024
APP_ID = "org.arenaos.phase14pie"
SOURCE = "/Users/user/Desktop/zzz-phase14-pie.apb1"
PASS = re.compile(
    r"\[phase14-pie\] PASS base=0x([0-9a-f]+) entry=0x([0-9a-f]+) "
    r"relocated=0x([0-9a-f]+) data=0x([0-9a-f]+) bss=0x([0-9a-f]+) exit=42"
)
PLACEMENT = re.compile(
    r"\[arena INFO  aslr\] PIE placement image=\d+ pid=\d+ "
    r"bias=0x([0-9a-f]+) base=0x([0-9a-f]+) entry=0x([0-9a-f]+) "
    r"candidates=(\d+); RX/R/RW finalized, W\^X and lower/upper/stack guards checked"
)
COUNTERS = re.compile(
    r"measured frames/records/processes/regions/pages/maps/caps="
    r"(\d+)/(\d+)/(\d+)/(\d+)/(\d+)/(\d+)/(\d+)\r?\n"
)
DETAILS = re.compile(r"native resource detail values=(\d+(?:/\d+){14})\r?\n")


def wait(predicate, message: str, timeout_s: float = 90) -> None:
    deadline = time.monotonic() + timeout_s
    while time.monotonic() < deadline:
        if predicate():
            return
        time.sleep(0.05)
    raise TimeoutError(message)


def log_text(serial: Path) -> str:
    return serial.read_text(errors="replace") if serial.exists() else ""


def disk_tree(disk: Path) -> dict[str, bytes]:
    deadline = time.monotonic() + 10
    last_error: Exception | None = None
    while time.monotonic() < deadline:
        try:
            volume = afs2.Volume(disk.read_bytes()[AFS2_BASE:])
            afs2.check(volume)
            return afs2.walk(volume)
        except Exception as error:
            last_error = error
            time.sleep(0.1)
    raise AssertionError(f"AFS2 guest image did not settle for readback: {last_error}")


def callback(sock: Path, serial: Path, image: Path, disk: Path) -> str:
    # Phase-13 launch coverage runs first on the same boot and uses the
    # unchanged fixed-address ET_EXEC package/runtime path.
    phase13_proof = phase13.callback(sock, serial, image, disk)
    conn = qmp.Qmp(str(sock), connect_timeout_s=5)

    def log() -> str:
        return log_text(serial)

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

    def right_click(x: int, y: int) -> None:
        point(x, y)
        point(x, y, "right", True)
        point(x, y, "right", False)

    def key(qcode: str) -> None:
        conn.command("input-send-event", events=[qmp.Qmp._ev(qcode, True), qmp.Qmp._ev(qcode, False)])

    try:
        wait(lambda: "[desktop] desktop surface shows /Users/user/Desktop" in log(),
             "Desktop did not remain live after the Phase-13 witness")
        bundle = (ROOT / "phase14-pie.apb1").read_bytes()
        parsed = apb1_format.parse_bundle(bundle)
        executable = (ROOT / "phase14-pie.elf").read_bytes()
        if parsed.payload != executable or parsed.files[0][1] != b"bin/pie":
            raise AssertionError("bundled signed APB1 does not contain the exact Phase-14 ELF")

        installed_before = log().count("[desktop] APB1 installed; signed version=")
        # The Phase-14 source is fourth in the desktop's sorted order; the
        # three established Phase-13 icons keep their coordinates.
        # Open the selected APB1 from its real desktop context menu. The
        # production Open action invokes the same filesd/packaged installer
        # affordance as a double click, without host-timing ambiguity.
        right_click(52, 280)
        time.sleep(0.1)
        click(84, 296)
        wait(lambda: log().count("[desktop] APB1 installed; signed version=") == installed_before + 1,
             "Desktop did not install the signed Phase-14 APB1 through packaged/filesd")
        tree = disk_tree(disk)
        version = int.from_bytes(parsed.metadata[64 + 104:64 + 112], "little")
        prefix = f"/System/Applications/{APP_ID}/{version}/"
        if tree.get(SOURCE) != bundle:
            raise AssertionError("the source APB1 changed during installation")
        if tree.get(prefix + "bin/pie") != executable:
            raise AssertionError("AFS2 protected installed record does not contain the exact PIE ELF")
        if tree.get(prefix + "APB1.record") != parsed.metadata + parsed.signature:
            raise AssertionError("AFS2 installed signed record differs from the fixture")
        if any(path.startswith("/System/.apb1-staging/") for path in tree):
            raise AssertionError("APB1 staging residue remained after install")

        baseline_rows = COUNTERS.findall(log())
        detail_rows = DETAILS.findall(log())
        if not baseline_rows or not detail_rows:
            raise AssertionError("native resource baseline receipt is missing")
        baseline = tuple(map(int, baseline_rows[-1]))
        detail = tuple(map(int, detail_rows[-1].split("/")))
        pass_count = len(PASS.findall(log()))
        exit_count = log().count("[desktop] child Process-cap exit status=42")
        for ordinal in (1, 2):
            click(94, 12)
            conn.type_text("Phase14 PIE", gap_s=0.025)
            key("ret")
            expected = pass_count + ordinal
            wait(lambda expected=expected: len(PASS.findall(log())) == expected,
                 f"All Applications did not run Phase-14 PIE instance {ordinal}", 60)
            wait(lambda ordinal=ordinal: log().count("[desktop] child Process-cap exit status=42")
                 == exit_count + ordinal,
                 f"Phase-14 PIE instance {ordinal} did not exit with status 42", 60)
            wait(lambda baseline=baseline, detail=detail:
                 bool(COUNTERS.findall(log()))
                 and tuple(map(int, COUNTERS.findall(log())[-1])) == baseline
                 and bool(DETAILS.findall(log()))
                 and tuple(map(int, DETAILS.findall(log())[-1].split("/"))) == detail,
                 f"Phase-14 PIE instance {ordinal} did not return process/Image resources exactly", 60)

        app_runs = PASS.findall(log())[-2:]
        kernel_runs = PLACEMENT.findall(log())[-2:]
        if len(app_runs) != 2 or len(kernel_runs) != 2:
            raise AssertionError("two application and kernel placement receipts were not observed")
        for app, kernel in zip(app_runs, kernel_runs):
            app_base, app_entry = int(app[0], 16), int(app[1], 16)
            bias, base, entry = (int(value, 16) for value in kernel[:3])
            candidate_count = int(kernel[3])
            if app_base != base or app_entry != entry or app_base != bias:
                raise AssertionError("Startup ABI base/entry differs from the kernel placement receipt")
            if not 0x400000000000 <= bias < 0x600000000000 or bias % (2 * 1024 * 1024):
                raise AssertionError(f"kernel selected an invalid PIE placement: {bias:#x}")
            if candidate_count < (1 << 23):
                raise AssertionError(f"placement candidate range collapsed: {candidate_count}")
        bases = [int(row[0], 16) for row in app_runs]
        if bases[0] == bases[1]:
            raise AssertionError(f"same signed image reused one ASLR base: {bases[0]:#x}")
        return (
            phase13_proof + "; signed APB1 PIE installed and AFS2 record checked; "
            f"two relocated launches at {bases[0]:#x} and {bases[1]:#x}; "
            "Startup ABI v2, relocated state, exit 42, and exact teardown verified"
        )
    finally:
        conn.close()


def verify_inputs() -> str:
    manifest = (ROOT / "sha256sums.txt").read_text().splitlines()
    for line in manifest:
        digest, name = line.split(maxsplit=1)
        relative = Path(name)
        target = ROOT / relative
        if relative.is_absolute() or ".." in relative.parts or target.resolve().parent != ROOT:
            raise ValueError(f"unsafe archive manifest entry: {name}")
        if hashlib.sha256(target.read_bytes()).hexdigest() != digest:
            raise ValueError(f"archive entry hash mismatch: {name}")
    efi_sha = hashlib.sha256((ROOT / "arena-boot.efi").read_bytes()).hexdigest()
    if (ROOT / "stability-receipt.txt").read_text().split() != [efi_sha, "100/100"]:
        raise ValueError("the archive EFI is not the exact fresh 100/100 qualified artifact")
    suite = (ROOT / "qualification-phase14-full-suite.log").read_text(errors="replace")
    if "ALL TESTS PASSED (" not in suite:
        raise ValueError("the full historical/Phase-14 suite receipt is missing or failed")
    return efi_sha


def main() -> None:
    if sys.argv[1:]:
        raise ValueError("usage: phase14_archive_boot.py")
    sha = verify_inputs()
    phase10.boot(
        qualified=True,
        label="PHASE14",
        after=callback,
        launches=4,
        retirements=7,
    )
    print(f"EXTRACTED PHASE14 PASS: EFI SHA-256 {sha}; suite and 100/100 receipts verified")


if __name__ == "__main__":
    try:
        main()
    except Exception as error:
        sys.exit(f"EXTRACTED PHASE14 BOOT FAILED: {error}")
