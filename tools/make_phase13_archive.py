#!/usr/bin/env python3
"""Assemble the frozen Phase-13 qualification archive.

Inputs are the final EFI/ESP, the qualified local OVMF firmware, deterministic
signed APB1 guest fixtures, the AFS1/AFS2 disk template, logs, receipts, tools,
and phase documentation. The extracted boot witness has no checkout or build
directory dependency.
"""
from __future__ import annotations

import argparse
import gzip
import hashlib
import os
import re
import shutil
import sys
import tarfile
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "tools"))
import afs1
import afs2
from test_phase13_registry_guest import app_bundle, headless_bundle

BASE = 8 * 1024 * 1024
DISK_SIZE = 72 * 1024 * 1024
SECTOR = 512

TOOLS = (
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
    "PHASE14-HANDOFF.md",
    "TOOL-VERSIONS.txt",
)
PHASE12_DOCS = (
    "FINAL-REPORT.md",
    "PHASE13-HANDOFF.md",
    "COMPATIBILITY-HANDOFF.md",
    "ARCHITECTURE-AUDIT.md",
    "BASELINE-RECEIPT.md",
    "PLAN.md",
    "PROGRESS.md",
    "SERVICE-INTEGRATION-AUDIT.md",
)
ADR_NAMES = (
    "0080-phase12-application-platform-and-abi.md",
    "0081-apb1-bundle-format.md",
    "0082-apb1-afs2-install-activation.md",
    "0083-native-startup-abi-v2.md",
    "0084-bounded-native-heap.md",
    "0085-native-per-thread-tls-fsbase.md",
    "0086-native-startup-exact-cap-inventory.md",
    "0087-native-process-groups-and-process-cap-lifecycle.md",
    "0088-phase12-desktop-capacity-envelope.md",
    "0089-phase12-ipc-burst-capacity.md",
    "0090-desktop-badged-application-sessions.md",
    "0091-apb1-filesd-install-handoff.md",
    "0092-installed-application-registry-and-launch-authority.md",
    "0093-bounded-native-image-envelope.md",
    "0094-process-owned-user-mapping-inventory.md",
    "0095-process-owned-native-vm.md",
    "0096-scalable-native-heap.md",
    "0097-multiple-ordinary-windows-per-application-session.md",
    "0098-headless-installed-application-launch.md",
    "0099-desktop-app-instance-process-groups.md",
    "0100-process-cap-exit-status.md",
    "0101-signed-helper-allowlist.md",
    "0102-native-thread-yield.md",
    "0103-desktop-manager-fail-stop.md",
    "0104-native-byte-streams.md",
    "0105-helper-standard-streams.md",
    "0106-native-user-threads.md",
    "0107-native-synchronization-domains.md",
    "0108-native-pie-scope.md",
    "0109-phase13-mixed-workload-receipt.md",
)
LOGS = (
    "qualification-phase13-final-suite.log",
    "qualification-phase13-apb1-oracle.log",
    "stability-phase13-final-100.log",
    "serial-phase13-registry.log",
    "serial-phase13-registry-favorites-reload.log",
    "stability-receipt.txt",
)


def copy_file(source: Path, target: Path, *, required: bool = True) -> bool:
    if not source.is_file():
        if required:
            raise FileNotFoundError(source)
        return False
    shutil.copyfile(source, target)
    return True


def make_disk(path: Path, app: bytes, headless: bytes) -> None:
    with tempfile.TemporaryDirectory(prefix="arena-phase13-afs1-") as temporary:
        first_path = Path(temporary) / "afs1.img"
        afs1.mkfs(first_path, BASE // SECTOR)
        first_region = first_path.read_bytes()
    total_blocks = (DISK_SIZE - BASE) // afs2.BLOCK
    volume = afs2.Volume(afs2.mkfs(total_blocks, volume_id=0x5048415345313341))
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
    source = volume.create(desktop, b"phase13.apb1")
    volume.write(source, 0, app)
    source = volume.create(desktop, b"z-associated.txt")
    volume.write(source, 0, b"Phase 13 associated document\n")
    source = volume.create(desktop, b"zz-headless.apb1")
    volume.write(source, 0, headless)
    if not imported:
        raise AssertionError("archive AFS1 import directory missing")
    afs2.check(volume)
    data = bytearray(DISK_SIZE)
    data[:BASE] = first_region
    data[BASE:] = volume.dev
    path.write_bytes(data)


def archive_report_copy(source: Path, target: Path) -> None:
    report = source.read_text()
    phrase = "phase13-complete archive SHA-256:"
    lines = report.splitlines()
    output: list[str] = []
    index = 0
    while index < len(lines):
        line = lines[index]
        if phrase in line:
            output.append(
                line.split(phrase, 1)[0]
                + phrase
                + " recorded in the repository final report (outer archive self-reference omitted)"
            )
            if not line.split(phrase, 1)[1].strip() and index + 1 < len(lines):
                continuation = lines[index + 1].strip().strip("`")
                if re.fullmatch(r"[0-9a-f]{64}", continuation):
                    index += 1
        else:
            output.append(line)
        index += 1
    target.write_text("\n".join(output) + "\n")


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--output-dir", type=Path, default=ROOT / "build/phase13-complete")
    parser.add_argument(
        "--archive",
        type=Path,
        default=ROOT / "releases/checkpoints/phase13-complete/arenaos-phase13-complete-qemu-x86_64.tar.gz",
    )
    parser.add_argument("--smoke", action="store_true", help="allow incomplete suite/stability receipts")
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

    # Build deterministic, real native ELF/APB1 fixtures from the qualified
    # guest applications, then seed the pristine archive AFS2 template.
    app = app_bundle()
    headless = headless_bundle()
    (out / "phase13.apb1").write_bytes(app)
    (out / "zz-headless.apb1").write_bytes(headless)
    make_disk(out / "scratch-template.img", app, headless)

    for name in TOOLS:
        copy_file(ROOT / "tools" / name, out / name)
    for name in PHASE_DOCS:
        copy_file(ROOT / "docs" / "phase13" / name, out / name, required=not args.smoke)
    for name in PHASE12_DOCS:
        copy_file(ROOT / "docs" / "phase12" / name, out / f"phase12-{name}", required=not args.smoke)
    if (ROOT / "docs/phase13/FINAL-REPORT.md").is_file():
        archive_report_copy(ROOT / "docs/phase13/FINAL-REPORT.md", out / "FINAL-REPORT.md")
    elif not args.smoke:
        raise FileNotFoundError(ROOT / "docs/phase13/FINAL-REPORT.md")
    for name in ADR_NAMES:
        copy_file(ROOT / "docs/adr" / name, out / f"ADR-{name}", required=not args.smoke)

    for name in LOGS:
        copy_file(build / name, out / name, required=not args.smoke)
    # These focused guest receipts provide short, inspectable subsystem logs
    # in addition to the complete historical-suite and mixed-workload logs.
    for name in ("serial-m10-desktop.log", "serial-m11-wm.log", "serial-m12-scale.log"):
        copy_file(build / name, out / name, required=not args.smoke)

    tool_versions = f"""ArenaOS Phase-13 tool versions
rustc: rustc 1.97.0 (2d8144b78 2026-07-07)
cargo: cargo 1.97.0 (c980f4866 2026-06-30)
rustfmt: rustfmt 1.9.0-stable (2d8144b788 2026-07-07)
qemu-system-x86_64: QEMU emulator version 10.0.11 (Debian 1:10.0.11+ds-0+deb13u1)
OVMF: edk2-ovmf 2025.02-8+deb13u1, 4 MiB CODE/VARS
host: Debian GNU/Linux 13 (trixie), QEMU/OVMF from signed Debian snapshot repository
installation: unprivileged extraction under /tmp; no root or sudo
"""
    (out / "TOOL-VERSIONS.txt").write_text(tool_versions)

    readme = """ArenaOS Phase-13 qualification archive

This package contains the exact qualified EFI and ESP, matching 100/100
stability receipt, OVMF CODE and VARS template, fresh AFS1/AFS2 scratch disk,
two signed APB1 fixtures, Phase-13 and Phase-12 closeout documents, ADRs,
qualification logs, tool versions, and the standalone guest witness.

Requirements: Python 3, QEMU qemu-system-x86_64, and a POSIX host with Unix
sockets. The witness uses only files beside this README plus the local QEMU
binary. From this extracted directory run:

    python3 phase13_archive_boot.py

The witness verifies sha256sums.txt and the EFI/100-boot receipt, boots these
files, installs both signed packages through Desktop, checks installed AFS2
payloads and records, explicitly opens a read-only document with the installed
handler, searches and launches through All Applications, observes three real
windows, helper and stream transfers, native VM/heap/threads/synchronization,
headless Process-cap exit, identity-resource teardown, and clean UEFI shutdown.
It also repeats the historical desktop, console, network, and process checks.
No checkout, build tree, Rust installation, or signing key is read.

The fixture signer is the public RFC 8032 test-vector key in apb1_format.py;
these packages are qualification fixtures, not production keys. The archive's
FINAL-REPORT.md copy omits its own outer archive digest to avoid an impossible
cryptographic self-reference; the repository final report records that exact
digest.
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
                for path in sorted(out.iterdir(), key=lambda item: item.name):
                    def normalized(info: tarfile.TarInfo) -> tarfile.TarInfo:
                        info.mtime = 0
                        info.uid = info.gid = 0
                        info.uname = info.gname = ""
                        return info

                    tar.add(
                        path,
                        arcname=f"phase13-complete/{path.name}",
                        recursive=False,
                        filter=normalized,
                    )
    digest = hashlib.sha256(archive.read_bytes()).hexdigest()
    archive.with_suffix(archive.suffix + ".sha256").write_text(f"{digest}  {archive.name}\n")
    print(f"archive contents: {out}")
    print(f"archive: {archive}")
    print(f"SHA-256: {digest}")


if __name__ == "__main__":
    try:
        main()
    except (OSError, ValueError, AssertionError) as exc:
        sys.exit(f"Phase-13 archive build failed: {exc}")
