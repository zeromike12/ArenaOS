#!/usr/bin/env python3
"""Assemble the self-contained Phase-12 qualification archive.

The archive is generated from the frozen EFI, local OVMF files, the guest
fixture, qualification receipts/logs, and the explicitly named documents.
It is extracted and booted separately with phase12_archive_boot.py.
"""
from __future__ import annotations

import argparse
import gzip
import hashlib
import os
import shutil
import sys
import tarfile
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "tools"))
import afs1
import afs2
import apb1_format

BASE = 8 * 1024 * 1024
DISK_SIZE = 72 * 1024 * 1024
SECTOR = 512

TOOLS = (
    "phase12_archive_boot.py",
    "phase10_archive_boot.py",
    "check_phase10_pixels.py",
    "network_fixture.py",
    "tcp_fixture.py",
    "udp_dns_fixture.py",
    "qmp.py",
    "vcon.py",
    "afs1.py",
    "afs2.py",
    "apb1_format.py",
)
DOCS = (
    "FINAL-REPORT.md",
    "PHASE13-HANDOFF.md",
    "COMPATIBILITY-HANDOFF.md",
    "PLAN.md",
    "PROGRESS.md",
    "SERVICE-INTEGRATION-AUDIT.md",
    "TOOL-VERSIONS.txt",
)
ADRS = (
    "0080-phase12-application-platform-and-abi.md",
    "0081-apb1-bundle-format.md",
    "0082-apb1-afs2-install-activation.md",
    "0083-native-startup-abi-v2.md",
    "0086-native-startup-exact-cap-inventory.md",
    "0087-native-process-groups-and-process-cap-lifecycle.md",
    "0088-phase12-desktop-capacity-envelope.md",
    "0090-desktop-badged-application-sessions.md",
    "0091-apb1-filesd-install-handoff.md",
)
LOGS = (
    "qualification-full-suite.log",
    "serial-phase12-apb1-guest.log",
    "serial-m12-startup.log",
    "serial-m12-scale.log",
    "serial-m85-resources.log",
    "serial-m9-resources.log",
    "serial-m10-apps.log",
    "serial-m10-dynamic.log",
    "serial-m10-boundaries.log",
    "serial-m10-client-death.log",
    "serial-m10-service-death.log",
    "serial-m10-files.log",
    "serial-m11-wm.log",
    "serial-m11-files.log",
    "stability-results.log",
)


def copy_file(source: Path, target: Path, *, required: bool = True) -> bool:
    if not source.is_file():
        if required:
            raise FileNotFoundError(source)
        return False
    shutil.copyfile(source, target)
    return True


def make_disk(path: Path, bundle: bytes) -> None:
    with tempfile.TemporaryDirectory(prefix="arena-phase12-afs1-") as tmp:
        afs1_path = Path(tmp) / "afs1.img"
        afs1.mkfs(afs1_path, BASE // SECTOR)
        first_region = afs1_path.read_bytes()
    total_blocks = (DISK_SIZE - BASE) // afs2.BLOCK
    image = afs2.mkfs(total_blocks, volume_id=0x5048415345313241)
    volume = afs2.Volume(image)
    root = volume.root_id()
    system = volume.mkdir(root, b"System")
    imported = volume.mkdir(system, b"imported-afs1")
    volume.create(system, b"afs1-import-complete")
    volume.mkdir(system, b"Applications")
    volume.mkdir(system, b".apb1-staging")
    users = volume.mkdir(root, b"Users")
    user = volume.mkdir(users, b"user")
    desktop = volume.mkdir(user, b"Desktop")
    volume.mkdir(user, b"Documents")
    volume.mkdir(user, b".Trash")
    source = volume.create(desktop, b"editor.apb1")
    volume.write(source, 0, bundle)
    if not imported or not afs2.check(volume):
        raise AssertionError("fresh archive AFS2 fixture failed structural audit")
    data = bytearray(DISK_SIZE)
    data[:BASE] = first_region
    data[BASE:] = volume.dev
    path.write_bytes(data)


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--output-dir", type=Path, default=ROOT / "build/phase12-complete")
    parser.add_argument("--archive", type=Path, default=ROOT / "build/phase12-complete.tar.gz")
    parser.add_argument("--smoke", action="store_true", help="allow pending suite/stability receipts")
    args = parser.parse_args()

    out = args.output_dir.resolve()
    archive = args.archive.resolve()
    if out == archive or out in archive.parents:
        raise ValueError("archive must be outside the extracted content directory")
    if out.exists():
        shutil.rmtree(out)
    out.mkdir(parents=True)

    build = ROOT / "build"
    efi = build / "arena-boot.efi"
    esp = build / "arena-esp.img"
    code = Path(os.environ.get("ARENA_OVMF_CODE", ""))
    vars_template = Path(os.environ.get("ARENA_OVMF_VARS", ""))
    if not code.is_file() or not vars_template.is_file():
        raise FileNotFoundError("set ARENA_OVMF_CODE and ARENA_OVMF_VARS to the qualified firmware")
    copy_file(efi, out / "arena-boot.efi")
    copy_file(esp, out / "arena-esp.img")
    copy_file(code, out / "edk2-x86_64-code.fd")
    copy_file(vars_template, out / "ovmf-vars-template.img")
    fixture = apb1_format.default_fixtures()["editor.apb1"]
    (out / "editor.apb1").write_bytes(fixture)
    make_disk(out / "scratch-template.img", fixture)

    for name in TOOLS:
        copy_file(ROOT / "tools" / name, out / name)
    for name in DOCS:
        copy_file(ROOT / "docs" / "phase12" / name, out / name, required=not args.smoke)
    for name in ADRS:
        copy_file(ROOT / "docs" / "adr" / name, out / ("ADR-" + name), required=not args.smoke)

    receipt_source = build / "stability-receipt.txt"
    if receipt_source.is_file():
        copy_file(receipt_source, out / "stability-receipt.txt")
    elif args.smoke:
        digest = hashlib.sha256(efi.read_bytes()).hexdigest()
        (out / "stability-receipt.txt").write_text(f"{digest} 0/0\n")
    else:
        raise FileNotFoundError("100/100 stability receipt is required")

    for name in LOGS:
        copy_file(build / name, out / name, required=not args.smoke)
    for i in range(1, 101):
        copy_file(
            build / f"stability-pixels-boot-{i}.txt",
            out / f"stability-pixels-boot-{i}.txt",
            required=not args.smoke,
        )

    readme = """ArenaOS Phase-12 qualification archive

This archive contains the exact EFI and ESP, its 100/100 stability receipt,
the bundled OVMF firmware and variable template, a preformatted AFS1/AFS2
disk with the signed editor.apb1 test package, qualification tools/logs, and
the Phase-12 closeout documents.

Requirements: Python 3, QEMU qemu-system-x86_64, and a POSIX host with Unix
sockets. The included OVMF code and variable template are used by the boot
witness. From this directory run:

    python3 phase12_archive_boot.py

The witness verifies sha256sums.txt, checks the EFI/100-boot receipt, boots
only these extracted files, launches and retires two native applications,
installs the signed APB1 fixture through the Desktop, verifies the signed
record and every payload file from AFS2 readback, confirms AFS1/source
preservation and clean UEFI shutdown. It also exercises the historical
network and console fixtures. No checkout or build directory is read.

The APB1 signer in apb1_format.py is the public RFC 8032 test vector key;
this fixture is for qualification only and is not a production signing key.
"""
    (out / "README.txt").write_text(readme)

    names = sorted(p.name for p in out.iterdir() if p.is_file() and p.name != "sha256sums.txt")
    (out / "sha256sums.txt").write_text(
        "".join(f"{hashlib.sha256((out / name).read_bytes()).hexdigest()}  {name}\n" for name in names)
    )
    if archive.exists():
        archive.unlink()
    archive.parent.mkdir(parents=True, exist_ok=True)
    with archive.open("wb") as raw:
        with gzip.GzipFile(fileobj=raw, mode="wb", filename="", mtime=0, compresslevel=9) as gz:
            with tarfile.open(fileobj=gz, mode="w", format=tarfile.PAX_FORMAT) as tar:
                for path in sorted(out.iterdir(), key=lambda p: p.name):
                    tar.add(path, arcname=f"phase12-complete/{path.name}", recursive=False)
    digest = hashlib.sha256(archive.read_bytes()).hexdigest()
    archive.with_suffix(archive.suffix + ".sha256").write_text(f"{digest}  {archive.name}\n")
    print(f"archive contents: {out}")
    print(f"archive: {archive}")
    print(f"SHA-256: {digest}")


if __name__ == "__main__":
    try:
        main()
    except (OSError, ValueError, AssertionError) as exc:
        sys.exit(f"Phase-12 archive build failed: {exc}")
