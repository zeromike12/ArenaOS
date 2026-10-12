#!/usr/bin/env python3
"""Build a self-contained, reproducible Phase 15 diagnostic checkpoint."""
from __future__ import annotations

import argparse
import gzip
import hashlib
import json
import os
import shutil
import subprocess
import tarfile
import tempfile
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
ARTIFACT_SOURCE_COMMIT = "6586a0a18981932ae7048ca267fa425411f8e463"
DIAGNOSTIC_EFI = "phase15-diagnostic-efi-6586a0a.efi"
DIAGNOSTIC_ESP = "phase15-diagnostic-esp-6586a0a.img"
STRICT_EFI = "phase15-strict-efi-6586a0a.efi"
STRICT_ESP = "phase15-strict-esp-6586a0a.img"


def sha256(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            h.update(chunk)
    return h.hexdigest()


def copy_file(source: Path, destination: Path) -> None:
    if not source.is_file():
        raise FileNotFoundError(source)
    destination.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(source, destination)


def copy_tree(source: Path, destination: Path) -> None:
    if not source.is_dir():
        raise FileNotFoundError(source)
    shutil.copytree(source, destination, symlinks=True)


def copy_runtime(qemu_root: Path, musl_root: Path, pydeps: Path, stage: Path) -> None:
    runtime = stage / "runtime"
    (runtime / "qemu/bin").mkdir(parents=True)
    copy_file(qemu_root / "bin/qemu-system-x86_64", runtime / "qemu/bin/qemu-system-x86_64")
    copy_tree(qemu_root / "lib", runtime / "qemu/lib")
    share_source = qemu_root / "share"
    share_dest = runtime / "qemu/share"
    share_dest.mkdir(parents=True)
    for current, dirs, files in os.walk(share_source):
        dirs[:] = sorted(d for d in dirs if d != "__pycache__")
        rel = Path(current).relative_to(share_source)
        target_dir = share_dest / rel
        target_dir.mkdir(exist_ok=True)
        for name in sorted(files):
            if name in ("edk2-aarch64-code.fd", "edk2-arm-vars.fd"):
                continue
            source = Path(current) / name
            if source.is_symlink():
                (target_dir / name).symlink_to(os.readlink(source))
            else:
                shutil.copy2(source, target_dir / name)
    copy_file(musl_root / "lib/libc.so", runtime / "musl/lib/libc.so")
    firmware = runtime / "firmware"
    copy_file(qemu_root / "share/qemu/edk2-x86_64-code.fd", firmware / "edk2-x86_64-code.fd")
    copy_file(qemu_root / "share/qemu/edk2-i386-vars.fd", firmware / "edk2-i386-vars.fd")

    vendor = runtime / "python"
    vendor.mkdir(parents=True)
    for name in ("pyfatfs", "fs", "pkg_resources", "setuptools", "_distutils_hack"):
        copy_tree(pydeps / name, vendor / name)
    for name in ("six.py", "appdirs.py"):
        copy_file(pydeps / name, vendor / name)
    for pattern in (
        "pyfatfs-*.dist-info", "fs-*.dist-info", "appdirs-*.dist-info",
        "six-*.dist-info", "setuptools-*.dist-info",
    ):
        for source in sorted(pydeps.glob(pattern)):
            copy_tree(source, vendor / source.name)


def copy_receipts(build: Path, stage: Path) -> None:
    target = stage / "receipts/raw"
    target.mkdir(parents=True)
    patterns = (
        "phase15-historical-diagnostic*",
        "phase15-rerun-*",
        "phase15-stability-t2*",
        "phase15-stability-t3*",
        "phase15-stability-t4*",
        "phase15-osxsave-seed-*",
        "phase15-canonical-build.log",
        "phase15-final-canonical-build.log",
        "phase15-targeted-run.log",
        "serial-phase15-avx.log",
        "serial-phase15-qemu64.log",
        "serial-phase15-osxsave-seed.log",
        "serial-m8-fault*.log",
        "serial-m8-dependencies*.log",
        "serial-phase14-*.log",
        "serial-m10-boundaries*.log",
        "serial-m84-stage*.log",
        "phase15-mshv-probe.log",
        "phase15-nitro-probe.log",
        "phase15-kvm-probe.log",
        "phase15-checkpoint-verification-*.log",
        "stability-expected.txt",
        "stability-receipt.txt",
        "stability-serial.log",
        "tcp-stability.log",
        "udp-dns-stability-boot-*.log",
        "stability-pixel-actor.log",
        "stability-display-boot-*.ppm",
        "stability-pixels-boot-*.txt",
    )
    copied: set[Path] = set()
    for pattern in patterns:
        for source in sorted(build.glob(pattern)):
            if source.is_file() and source not in copied:
                copy_file(source, target / source.name)
                copied.add(source)
    required = {
        "phase15-historical-diagnostic.log",
        "phase15-historical-diagnostic-serial.log",
        "phase15-rerun-phase14-negative.log",
        "phase15-rerun-m10-boundaries-red.log",
        "phase15-rerun-m84-stage.log",
        "phase15-rerun-m8-dependencies.log",
        "phase15-rerun-m8-fault.log",
        "phase15-stability-t2.log",
        "phase15-stability-t3.log",
        "phase15-stability-t4-diagnostic.log",
        "phase15-osxsave-seed-receipt.json",
        "phase15-osxsave-seed-run.log",
        "stability-receipt.txt",
    }
    missing = sorted(required - {path.name for path in copied})
    if missing:
        raise FileNotFoundError(f"required qualification receipts missing: {', '.join(missing)}")


def write_readme(stage: Path, source_commit: str, t4: dict) -> None:
    (stage / "README.md").write_text(f"""# ArenaOS Phase 15 diagnostic checkpoint

This archive is an independently verified **diagnostic checkpoint**, not a
qualified Phase 15 release candidate. Genuine unmasked SIMD vector 19 (`#XF`)
delivery remains unverified on the available QEMU 11.0.2 TCG backend. The
strict image is included to preserve the real failing acceptance check. The
diagnostic image explicitly reports `XF NOT RUN`.

## Identity

- Source snapshot commit: `{source_commit}`.
- EFI/ESP artifact build commit: `{ARTIFACT_SOURCE_COMMIT}`.
- Backend: QEMU 11.0.2 TCG, CPU profile `qemu64,+nx,+smep,+smap`.
- Diagnostic T4: {t4['successful_boots']}/{t4['attempted_boots']} boots,
  {t4['failed_boots']} failures, {t4['timeouts']} timeouts, no retries;
  duration {t4['duration_seconds']} seconds.
- Official T2/T3/T4 qualification: **BLOCKED** because the diagnostic image
  omits genuine `#XF` acceptance and stock TCG did not deliver vector 19.

## Verify after extraction

On an x86-64 Linux host with Python 3, `tar`, and a working network loopback
interface, extract into a clean path without spaces and run the verifier:

```sh
mkdir -p /tmp/p15
tar -xzf /path/to/phase15-diagnostic-checkpoint.tar.gz -C /tmp/p15
cd /tmp/p15/ArenaOS-Phase15-Diagnostic-Checkpoint
python3 -I source/tools/phase15_checkpoint_verify.py
```

No Rust toolchain, repository checkout, signing key, root privilege, or KVM
device is used by the verifier. The guest network tests use QEMU user networking
and host loopback fixtures on TCP port 54321 and UDP port 1053.

The verifier checks the manifest, pins all four EFI/ESP identities, checks the
EFI embedded in each FAT ESP, boots the diagnostic ESP using only the bundled
QEMU/OVMF runtime, and runs the signed Phase-14 APB1/PIE guest workflow against
that same extracted ESP. It requires clean UEFI shutdown, Phase 15 diagnostic
thread/process/MMX/`#MF`/`#UD` evidence, and two distinct PIE placements with
resource cleanup. It explicitly rejects a strict Phase 15 PASS marker.

`source/` is a `git archive` source snapshot. `artifacts/diagnostic/` contains
the exact image used by the T2/T3/T4 diagnostic loops. `artifacts/strict-unqualified/`
contains the original strict test image, whose genuine `#XF`
failure remains a release blocker. `receipts/raw/` preserves the complete
historical-run log, all five focused recovery runs, OSXSAVE seed coverage, and
the stability-gate logs and per-boot receipts.
""")


def file_manifest(stage: Path, source_commit: str, t4: dict) -> dict:
    files: dict[str, dict] = {}
    for path in sorted(p for p in stage.rglob("*") if p.is_file() and p.name != "MANIFEST.json"):
        rel = path.relative_to(stage).as_posix()
        files[rel] = {"bytes": path.stat().st_size, "sha256": sha256(path)}
    qemu = stage / "runtime/qemu/bin/qemu-system-x86_64"
    qemu_lib = stage / "runtime/qemu/lib"
    loader = stage / "runtime/musl/lib/libc.so"
    qemu_share = stage / "runtime/qemu/share/qemu"
    version_cmd = ["env", f"LD_LIBRARY_PATH={qemu_lib}", str(loader), str(qemu), "-L", str(qemu_share), "--version"]
    qemu_version = subprocess.run(version_cmd, check=True, capture_output=True, text=True).stdout.splitlines()[0]
    manifest = {
        "archive_type": "diagnostic-checkpoint",
        "qualified": False,
        "source_commit": source_commit,
        "artifact_source_commit": ARTIFACT_SOURCE_COMMIT,
        "status": "PHASE 15 — IMPLEMENTED / HISTORICAL REGRESSIONS TESTED / EXCEPTION QUALIFICATION BLOCKED",
        "xf": {"result": "BLOCKED", "backend": "QEMU 11.0.2 TCG", "vector_19_delivered": False},
        "diagnostic_t4": t4,
        "backend": {
            "runtime": qemu_version,
            "accelerator": "TCG",
            "cpu_profile": "qemu64,+nx,+smep,+smap",
            "qemu_binary_sha256": sha256(qemu),
            "kvm_available_on_test_host": False,
            "mshv_available_on_test_host": False,
            "mshv_probe": "QEMU advertised mshv; explicit -accel mshv failed because /dev/mshv was absent",
        },
        "build_toolchain": {
            "rustc": "1.97.0 (2d8144b78 2026-07-07)",
            "rustc_sha256": "df13f58759c0662831983e3a6501c63c1fc12ea60ec4e1d1ac35e5fe43c500c0",
            "cargo": "1.97.0 (c980f4866 2026-06-30)",
            "llvm_lld": "22.1.6",
            "rust_lld_sha256": "8e5130a02d8f3473ed4c4b971d43c301205c89854bb467257e64eafe4bf21ca8",
        },
        "test_executables": {
            name: {
                "bytes": (stage / "artifacts/fixtures" / name).stat().st_size,
                "sha256": sha256(stage / "artifacts/fixtures" / name),
            }
            for name in (
                "arena-phase15-fpu-fixture",
                "arena-phase15-xf-probe",
                "arena-phase15-mmx-probe",
                "phase14-pie.elf",
                "phase14-pie.apb1",
            )
        },
        "files": files,
    }
    (stage / "MANIFEST.json").write_text(json.dumps(manifest, indent=2, sort_keys=True) + "\n")
    return manifest


def create_tar(stage: Path, output: Path) -> None:
    output.parent.mkdir(parents=True, exist_ok=True)
    top = "ArenaOS-Phase15-Diagnostic-Checkpoint"
    with output.open("wb") as raw:
        with gzip.GzipFile(filename="", mode="wb", fileobj=raw, compresslevel=6, mtime=0) as gz:
            with tarfile.open(fileobj=gz, mode="w", format=tarfile.PAX_FORMAT) as tar:
                root_info = tarfile.TarInfo(top)
                root_info.type = tarfile.DIRTYPE
                root_info.mode = 0o755
                root_info.uid = root_info.gid = 0
                root_info.uname = root_info.gname = ""
                root_info.mtime = 0
                tar.addfile(root_info)
                for path in sorted(stage.rglob("*")):
                    rel = Path(top) / path.relative_to(stage)
                    info = tar.gettarinfo(str(path), arcname=rel.as_posix())
                    info.uid = info.gid = 0
                    info.uname = info.gname = ""
                    info.mtime = 0
                    if info.isfile():
                        with path.open("rb") as stream:
                            tar.addfile(info, stream)
                    else:
                        tar.addfile(info)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, default=ROOT / "build/phase15-diagnostic-checkpoint.tar.gz")
    parser.add_argument("--qemu-root", type=Path, default=ROOT / "build/onboarding/qemu")
    parser.add_argument("--musl-root", type=Path, default=ROOT / "build/onboarding/musl")
    parser.add_argument("--python-deps", type=Path, default=Path("/tmp/arenaos-pydeps"))
    parser.add_argument("--t4-receipt", type=Path, default=ROOT / "build/phase15-stability-t4-diagnostic-receipt.json")
    args = parser.parse_args()

    status = subprocess.run(["git", "status", "--porcelain"], cwd=ROOT, check=True, text=True, capture_output=True).stdout
    if status.strip():
        raise SystemExit("working tree must be clean before building the source snapshot archive")
    source_commit = subprocess.run(["git", "rev-parse", "HEAD"], cwd=ROOT, check=True, text=True, capture_output=True).stdout.strip()
    t4 = json.loads(args.t4_receipt.read_text())
    if t4.get("mode") != "diagnostic-only" or t4.get("official_qualification") != "BLOCKED":
        raise SystemExit("T4 receipt must retain its diagnostic-only and official BLOCKED labels")

    with tempfile.TemporaryDirectory(prefix="arenaos-phase15-checkpoint-") as temp:
        stage = Path(temp) / "checkpoint"
        stage.mkdir()
        source = stage / "source"
        source.mkdir()
        snapshot = Path(temp) / "source.tar"
        with snapshot.open("wb") as stream:
            subprocess.run(["git", "archive", "--format=tar", source_commit], cwd=ROOT, check=True, stdout=stream)
        with tarfile.open(snapshot, "r") as tar:
            tar.extractall(source, filter="data")

        build = ROOT / "build"
        for source_name, directory, dest_name in (
            (DIAGNOSTIC_EFI, "artifacts/diagnostic", "arena-boot.efi"),
            (DIAGNOSTIC_ESP, "artifacts/diagnostic", "arena-esp.img"),
            (STRICT_EFI, "artifacts/strict-unqualified", "arena-boot.efi"),
            (STRICT_ESP, "artifacts/strict-unqualified", "arena-esp.img"),
        ):
            copy_file(build / source_name, stage / directory / dest_name)
        fixture_dir = stage / "artifacts/fixtures"
        fixture_dir.mkdir(parents=True)
        compiled_fixtures = Path("userspace/phase15-fpu/target/x86_64-unknown-none/release")
        for name in (
            "arena-phase15-fpu-fixture",
            "arena-phase15-xf-probe",
            "arena-phase15-mmx-probe",
        ):
            copy_file(ROOT / compiled_fixtures / name, fixture_dir / name)
        copy_file(source / "userspace/phase14-pie/fixture.elf", fixture_dir / "phase14-pie.elf")
        copy_file(source / "userspace/phase14-pie/phase14-pie.apb1", fixture_dir / "phase14-pie.apb1")
        copy_file(build / "phase15-diagnostic-graphics-profile.txt", stage / "artifacts/diagnostic/graphics-profile.txt")
        copy_file(build / "phase15-strict-graphics-profile.txt", stage / "artifacts/strict-unqualified/graphics-profile.txt")
        copy_runtime(args.qemu_root.resolve(), args.musl_root.resolve(), args.python_deps.resolve(), stage)
        copy_receipts(build, stage)
        write_readme(stage, source_commit, t4)
        manifest = file_manifest(stage, source_commit, t4)
        create_tar(stage, args.output.resolve())
        checksum = sha256(args.output.resolve())
        sidecar = args.output.resolve().with_suffix(args.output.suffix + ".sha256")
        sidecar.write_text(f"{checksum}  {args.output.name}\n")
    print(f"checkpoint: {args.output.resolve()}")
    print(f"archive sha256: {checksum}")
    print(f"files in manifest: {len(manifest['files'])}; source commit: {source_commit}")
    print("qualification: diagnostic checkpoint only; #XF blocked; no release-candidate claim")


if __name__ == "__main__":
    main()
