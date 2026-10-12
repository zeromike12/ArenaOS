#!/usr/bin/env python3
"""Assemble a deterministic portable strict Phase 15 execution kit."""
from __future__ import annotations

import argparse
import gzip
import hashlib
import json
import shutil
import subprocess
import tarfile
import tempfile
from pathlib import Path
from typing import Any


EFI_SHA = "bbbc63df271c3d7d5109789369f72e093b02ef77c77ebe25273b6da4c00d238e"
ESP_SHA = "33dd1576ab597a7d0b3e9a9100bc618e9b251a7a645e77e79d8852a404c40a6a"
XF_SHA = "1f057068b020baefe06f7f3b36bc0beeafcb879bd93d60c5ea66a3cce616c8d3"
TOOLS = (
    "phase15_external/README.md",
    "phase15_external/requirements.txt",
    "phase15_external/run_linux.sh",
    "phase15_external/run_windows.ps1",
    "phase15_external/run_external.py",
    "phase15_external/verify_receipt.py",
    "phase15_external/make_kit.py",
    "test_m15_guest.py", "mtest.py", "arena_env.py", "qmp.py", "vcon.py",
    "afs1.py", "tcp_fixture.py", "udp_dns_fixture.py",
)
SOURCE_REVIEW = (
    "kernel/kernel/src/m15.rs",
    "kernel/kernel/src/elf.rs",
    "kernel/kernel/src/arch/x86_64/idt.rs",
    "userspace/phase15-fpu/src/xf_probe.rs",
)


def digest(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            h.update(block)
    return h.hexdigest()


def git_value(root: Path, *args: str) -> str | None:
    result = subprocess.run(["git", "-C", str(root), *args],
                            text=True, capture_output=True)
    return result.stdout.strip() if result.returncode == 0 else None


def add_file(root: Path, stage: Path, source: Path, relative: str) -> None:
    if not source.is_file():
        raise FileNotFoundError(f"required kit source is missing: {source}")
    target = stage / relative
    target.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(source, target)


def manifest_files(stage: Path) -> dict[str, dict[str, Any]]:
    entries: dict[str, dict[str, Any]] = {}
    for path in sorted(p for p in stage.rglob("*") if p.is_file()):
        rel = path.relative_to(stage).as_posix()
        entries[rel] = {"bytes": path.stat().st_size, "sha256": digest(path)}
    return entries


def write_deterministic_tar(stage: Path, output: Path) -> None:
    output.parent.mkdir(parents=True, exist_ok=True)
    with output.open("wb") as raw:
        with gzip.GzipFile(filename="", mode="wb", fileobj=raw, mtime=0) as zipped:
            with tarfile.open(fileobj=zipped, mode="w") as archive:
                for path in sorted(p for p in stage.rglob("*") if p.is_file()):
                    rel = path.relative_to(stage).as_posix()
                    info = archive.gettarinfo(str(path), arcname=rel)
                    info.uid = info.gid = 0
                    info.uname = info.gname = ""
                    info.mtime = 0
                    with path.open("rb") as stream:
                        archive.addfile(info, stream)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source-root", type=Path, default=Path(__file__).resolve().parents[2])
    parser.add_argument("--efi", type=Path, required=True)
    parser.add_argument("--esp", type=Path, required=True)
    parser.add_argument("--xf-probe", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    root = args.source_root.resolve()
    efi, esp, xf = (path.expanduser().resolve() for path in
                    (args.efi, args.esp, args.xf_probe))
    for path, expected, name in ((efi, EFI_SHA, "strict EFI"),
                                 (esp, ESP_SHA, "strict ESP"),
                                 (xf, XF_SHA, "CPL3 #XF fixture")):
        if not path.is_file() or digest(path) != expected:
            raise SystemExit(f"{name} is missing or does not match the pinned SHA-256: {path}")
    try:
        from pyfatfs.PyFatFS import PyFatFS
    except ImportError as error:
        raise SystemExit("pyfatfs==1.1.0 is required to check the ESP's embedded EFI") from error
    fs = PyFatFS(str(esp), read_only=True)
    try:
        if fs.getbytes("/EFI/BOOT/BOOTX64.EFI") != efi.read_bytes():
            raise SystemExit("strict ESP does not contain the exact paired EFI")
    finally:
        fs.close()

    output = args.output.expanduser().resolve()
    with tempfile.TemporaryDirectory(prefix="phase15-external-kit-") as temp:
        stage = Path(temp)
        for rel in TOOLS:
            add_file(root / "tools", stage, root / "tools" / rel,
                     "tools/" + rel)
        for rel in SOURCE_REVIEW:
            add_file(root, stage, root / rel, "source-review/" + rel)
        add_file(root, stage, efi, "artifacts/arena-boot.efi")
        add_file(root, stage, esp, "artifacts/arena-esp.img")
        add_file(root, stage, xf, "artifacts/arena-phase15-xf-probe")
        license_path = root / "LICENSE"
        if license_path.is_file():
            add_file(root, stage, license_path, "LICENSE")

        commit = git_value(root, "rev-parse", "HEAD")
        branch = git_value(root, "branch", "--show-current")
        status = git_value(root, "status", "--porcelain")
        clean = status == "" if status is not None else None
        files = manifest_files(stage)
        manifest = {
            "format": "arenaos.phase15.external-kit.v1",
            "source_commit": commit,
            "source_branch": branch,
            "source_clean": clean,
            "strict_image_build_source_commit": "6586a0a18981932ae7048ca267fa425411f8e463",
            "qualified_release": False,
            "files": files,
        }
        (stage / "KIT-MANIFEST.json").write_text(
            json.dumps(manifest, indent=2, sort_keys=True) + "\n")
        write_deterministic_tar(stage, output)
    print(f"kit: {output}")
    print(f"bytes: {output.stat().st_size}")
    print(f"sha256: {digest(output)}")
    print(f"source commit: {commit}")
    print("qualification: external execution kit only; Phase 15 remains unqualified")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
