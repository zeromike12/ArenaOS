#!/usr/bin/env python3
"""Assemble the exact, independently bootable Phase-14 release archive."""
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
from test_phase13_registry_guest import app_bundle, headless_bundle

BASE = 8 * 1024 * 1024
DISK_SIZE = 72 * 1024 * 1024
SECTOR = 512

TOOLS = (
    "phase14_archive_boot.py",
    "phase14-launch-windows.cmd",
    "phase14-vcon-windows.ps1",
    "phase13_archive_boot.py",
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
PHASE_DOCS = (
    "PLAN.md",
    "PROGRESS.md",
    "PIE-FIXTURE.md",
    "MANUAL-WINDOWS.md",
    "FINAL-REPORT.md",
    "PHASE15-HANDOFF.md",
    "TOOL-VERSIONS.txt",
)
ADR_FILES = (
    "0016-elf64-executable-contract.md",
    "0083-native-startup-abi-v2.md",
    "0092-installed-application-registry-and-launch-authority.md",
    "0093-bounded-native-image-envelope.md",
    "0094-process-owned-user-mapping-inventory.md",
    "0095-process-owned-native-vm.md",
    "0108-native-pie-scope.md",
    "0110-static-native-pie-and-kernel-placement.md",
)
LOGS = (
    "qualification-phase14-full-suite.log",
    "phase14-stability-t2.log",
    "phase14-stability-t3.log",
    "phase14-stability-100.log",
    "phase14-stability-t2-receipt.txt",
    "phase14-stability-t3-receipt.txt",
    "stability-receipt.txt",
    "serial-phase14-pie.log",
    "serial-phase14-no-entropy.log",
    "serial-phase14-stale-image.log",
)
NEGATIVE_LOGS = (
    "serial-phase14-neg-bad-magic.log",
    "serial-phase14-neg-unsupported-relocation.log",
    "serial-phase14-neg-relocation-outside-image.log",
    "serial-phase14-neg-relocation-span-overflow.log",
    "serial-phase14-neg-invalid-alignment.log",
    "serial-phase14-neg-writable-executable.log",
    "serial-phase14-neg-outside-placement-arena.log",
)


def copy_file(source: Path, target: Path) -> None:
    if not source.is_file():
        raise FileNotFoundError(source)
    shutil.copyfile(source, target)


def make_disk(path: Path, app: bytes, headless: bytes, pie: bytes) -> None:
    with tempfile.TemporaryDirectory(prefix="arena-phase14-afs1-") as temporary:
        first_path = Path(temporary) / "afs1.img"
        afs1.mkfs(first_path, BASE // SECTOR)
        first_region = first_path.read_bytes()

    total_blocks = (DISK_SIZE - BASE) // afs2.BLOCK
    volume = afs2.Volume(afs2.mkfs(total_blocks, volume_id=0x5048415345313441))
    root = volume.root_id()
    system = volume.mkdir(root, b"System")
    volume.mkdir(system, b"imported-afs1")
    volume.create(system, b"afs1-import-complete")
    volume.mkdir(system, b"Applications")
    volume.mkdir(system, b".apb1-staging")
    users = volume.mkdir(root, b"Users")
    user = volume.mkdir(users, b"user")
    desktop = volume.mkdir(user, b"Desktop")
    volume.mkdir(user, b"Documents")
    volume.mkdir(user, b".Trash")
    source = volume.create(desktop, b"phase13.apb1")
    volume.write(source, 0, app)
    source = volume.create(desktop, b"z-associated.txt")
    volume.write(source, 0, b"Phase 13 associated document\n")
    source = volume.create(desktop, b"zz-headless.apb1")
    volume.write(source, 0, headless)
    # Keep the established Phase-13 icon rows stable; the Phase-14 fixture is
    # a fourth source so the extracted witness can install it by real clicks.
    source = volume.create(desktop, b"phase14-pie.apb1")
    volume.write(source, 0, pie)
    afs2.check(volume)

    image = bytearray(DISK_SIZE)
    image[:BASE] = first_region
    image[BASE:] = volume.dev
    path.write_bytes(image)


def make_report_copy(source: Path, target: Path) -> None:
    report = source.read_text()
    report = report.replace(
        "phase14-complete archive SHA-256: PENDING",
        "phase14-complete archive SHA-256: recorded in the repository final report",
    )
    target.write_text(report)


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--output-dir", type=Path, default=ROOT / "build/phase14-complete")
    parser.add_argument(
        "--archive",
        type=Path,
        default=ROOT / "releases/checkpoints/phase14-complete/arenaos-phase14-complete-qemu-x86_64.tar.gz",
    )
    args = parser.parse_args()
    out = args.output_dir.resolve()
    archive = args.archive.resolve()
    if out == archive or out in archive.parents:
        raise ValueError("archive must be outside the extracted content directory")
    if out.exists():
        shutil.rmtree(out)
    out.mkdir(parents=True)

    build = ROOT / "build"
    copy_file(build / "arena-boot.efi", out / "arena-boot.efi")
    copy_file(build / "arena-esp.img", out / "arena-esp.img")
    code = Path(os.environ.get("ARENA_OVMF_CODE", ""))
    vars_template = Path(os.environ.get("ARENA_OVMF_VARS", ""))
    if not code.is_file() or not vars_template.is_file():
        raise FileNotFoundError("set ARENA_OVMF_CODE and ARENA_OVMF_VARS to qualified OVMF files")
    copy_file(code, out / "edk2-x86_64-code.fd")
    copy_file(vars_template, out / "ovmf-vars-template.img")

    pie = (ROOT / "userspace/phase14-pie/phase14-pie.apb1").read_bytes()
    parsed = apb1_format.parse_bundle(pie)
    fixture = (ROOT / "userspace/phase14-pie/fixture.elf").read_bytes()
    if parsed.payload != fixture or parsed.files[0][1] != b"bin/pie":
        raise ValueError("signed Phase-14 APB1 does not contain the exact genuine PIE fixture")
    (out / "phase14-pie.apb1").write_bytes(pie)
    (out / "phase14-pie.elf").write_bytes(fixture)
    app = app_bundle()
    headless = headless_bundle()
    (out / "phase13.apb1").write_bytes(app)
    (out / "zz-headless.apb1").write_bytes(headless)
    make_disk(out / "scratch-template.img", app, headless, pie)

    for name in TOOLS:
        copy_file(ROOT / "tools" / name, out / name)
    for name in PHASE_DOCS:
        source = ROOT / "docs/phase14" / name
        if name == "FINAL-REPORT.md":
            make_report_copy(source, out / name)
        else:
            copy_file(source, out / name)
    for name in ADR_FILES:
        copy_file(ROOT / "docs/adr" / name, out / f"ADR-{name}")
    for name in LOGS + NEGATIVE_LOGS:
        copy_file(build / name, out / name)

    tool_versions = (ROOT / "docs/phase14/TOOL-VERSIONS.txt").read_text()
    (out / "TOOL-VERSIONS.txt").write_text(tool_versions)
    suite_text = (build / "qualification-phase14-full-suite.log").read_text(errors="replace")
    if "ALL TESTS PASSED (" not in suite_text:
        raise ValueError("full historical/Phase-14 suite receipt is missing or not green")
    efi_sha = hashlib.sha256((out / "arena-boot.efi").read_bytes()).hexdigest()
    if (out / "stability-receipt.txt").read_text().split() != [efi_sha, "100/100"]:
        raise ValueError("final stability receipt does not qualify the archived EFI")

    readme = f"""ArenaOS Phase-14 qualified native PIE and ASLR release

This archive contains the exact qualified EFI and ESP, the matching fresh
100/100 stability receipt, OVMF CODE and VARS template, a pristine AFS1/AFS2
scratch-disk template with three signed APB1 applications, the signed PIE ELF,
historical suite and Phase-14 guest/stability evidence, source documentation,
ADRs, and the independent extracted QEMU witness.

EFI SHA-256: {efi_sha}

Requirements: Python 3, QEMU qemu-system-x86_64, and a POSIX host with Unix
sockets. Extract this archive and run:

    python3 phase14_archive_boot.py

The witness verifies every sha256sums.txt entry and the exact 100/100 EFI
receipt, then boots only these extracted files. It uses UEFI/OVMF, virtio
block, QEMU user networking, virtio RNG, virtio keyboard/tablet, and the
virtio-console test channel. It first installs and exercises the Phase-13
fixed-address ET_EXEC applications and runtime checks. It then installs the
signed Phase-14 APB1 using Desktop/filesd/packaged, checks the protected AFS2
executable and signature record, launches the PIE twice from All Applications,
checks relocated startup/data and two different kernel-selected bases, checks
exit 42 and exact process/Image teardown, and shuts down cleanly.

No checkout, build directory, runtime signing key, Rust toolchain, or hidden
fixture path is read. The APB1 signature comes from the repository's public
qualification test root and is a fixture signature, not a production key.
For a graphical Windows launch with a separate persistent disk, see
MANUAL-WINDOWS.md.
"""
    (out / "README.txt").write_text(readme)

    entries = sorted(path.name for path in out.iterdir() if path.is_file() and path.name != "sha256sums.txt")
    (out / "sha256sums.txt").write_text(
        "".join(f"{hashlib.sha256((out / name).read_bytes()).hexdigest()}  {name}\n" for name in entries)
    )
    archive.parent.mkdir(parents=True, exist_ok=True)
    archive.unlink(missing_ok=True)
    with archive.open("wb") as raw:
        with gzip.GzipFile(fileobj=raw, mode="wb", filename="", mtime=0, compresslevel=9) as gz:
            with tarfile.open(fileobj=gz, mode="w", format=tarfile.PAX_FORMAT) as tar:
                for path in sorted(out.iterdir(), key=lambda item: item.name):
                    info = tar.gettarinfo(str(path), arcname=f"phase14-complete/{path.name}")
                    info.mtime = 0
                    info.uid = info.gid = 0
                    info.uname = info.gname = ""
                    with path.open("rb") as source:
                        tar.addfile(info, source)
    digest = hashlib.sha256(archive.read_bytes()).hexdigest()
    archive.with_suffix(archive.suffix + ".sha256").write_text(f"{digest}  {archive.name}\n")
    print(f"archive contents: {out}")
    print(f"archive: {archive}")
    print(f"SHA-256: {digest}")


if __name__ == "__main__":
    try:
        main()
    except (OSError, ValueError, AssertionError) as error:
        sys.exit(f"Phase-14 archive build failed: {error}")
