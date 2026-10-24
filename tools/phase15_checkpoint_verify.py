#!/usr/bin/env python3
"""Verify and exercise an extracted Phase 15 diagnostic checkpoint.

This deliberately verifies a diagnostic checkpoint, not a qualified release
candidate.  Genuine vector-19 (#XF) delivery remains an external gate.
"""
from __future__ import annotations

import hashlib
import json
import os
import re
import shutil
import subprocess
import sys
from pathlib import Path


HERE = Path(__file__).resolve()
SOURCE = HERE.parents[1]
ARCHIVE = SOURCE.parent
RUNTIME = ARCHIVE / "runtime"
ARTIFACTS = ARCHIVE / "artifacts"
DIAGNOSTIC = ARTIFACTS / "diagnostic"
STRICT = ARTIFACTS / "strict-unqualified"
EFI_SHA = "e85e0b5dd9e96f3129e7f2385d548084c56c16801432a9d41243b9ebc23a2282"
ESP_SHA = "b7a50113498033dd488ccd62acfebeed96d6a483d891cffed997cb79ef09bf42"
STRICT_EFI_SHA = "bbbc63df271c3d7d5109789369f72e093b02ef77c77ebe25273b6da4c00d238e"
STRICT_ESP_SHA = "33dd1576ab597a7d0b3e9a9100bc618e9b251a7a645e77e79d8852a404c40a6a"
STATE_FIXTURE_SHA = "786808ad21686ace59684f01ed1e69b47bd104c53442b2d7df9e646b9103b32b"
XF_FIXTURE_SHA = "1f057068b020baefe06f7f3b36bc0beeafcb879bd93d60c5ea66a3cce616c8d3"
MMX_FIXTURE_SHA = "4750c0efc9e20adc90f98829ab76c4ce9ddaabc0b1315dfb9bff427cdd9df785"
PIE_SHA = "d7b0a8cf9c735c3898a867d824563f06b0d949df80fa1a9c96f9180a395fea2e"
APB1_SHA = "04add54f09042353fc42511978c82f8b8efb87f7d5f0657f9116f456276b3dfa"


def digest(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            h.update(chunk)
    return h.hexdigest()


def require_hash(path: Path, expected: str) -> None:
    actual = digest(path)
    if actual != expected:
        raise AssertionError(f"{path.relative_to(ARCHIVE)}: SHA-256 {actual}, expected {expected}")
    print(f"PASS hash {path.relative_to(ARCHIVE)} {actual}", flush=True)


def check_manifest() -> dict:
    manifest_path = ARCHIVE / "MANIFEST.json"
    manifest = json.loads(manifest_path.read_text())
    if manifest.get("archive_type") != "diagnostic-checkpoint" or manifest.get("qualified") is not False:
        raise AssertionError("manifest does not identify an unqualified diagnostic checkpoint")
    checked = 0
    for rel, record in manifest["files"].items():
        path = ARCHIVE / rel
        if not path.is_file():
            raise AssertionError(f"manifest file is missing: {rel}")
        if path.stat().st_size != record["bytes"] or digest(path) != record["sha256"]:
            raise AssertionError(f"manifest mismatch: {rel}")
        checked += 1
    print(f"PASS manifest {checked} files; source commit {manifest['source_commit']}", flush=True)
    return manifest


def verify_esp(esp: Path, efi: Path) -> None:
    vendor = RUNTIME / "python"
    sys.path.insert(0, str(vendor))
    from pyfatfs.PyFatFS import PyFatFS  # type: ignore[import-not-found]

    fs = PyFatFS(str(esp), read_only=True)
    try:
        embedded = fs.getbytes("/EFI/BOOT/BOOTX64.EFI")
    finally:
        fs.close()
    if embedded != efi.read_bytes():
        raise AssertionError(f"{esp.name} does not contain its paired exact EFI")
    print(f"PASS embedded EFI {esp.relative_to(ARCHIVE)} matches {efi.name}", flush=True)


def prepare_environment() -> tuple[list[str], Path]:
    qemu = RUNTIME / "qemu" / "bin" / "qemu-system-x86_64"
    qemu_lib = RUNTIME / "qemu" / "lib"
    qemu_share = RUNTIME / "qemu" / "share" / "qemu"
    loader = RUNTIME / "musl" / "lib" / "libc.so"
    code = RUNTIME / "firmware" / "edk2-x86_64-code.fd"
    vars_template = RUNTIME / "firmware" / "edk2-i386-vars.fd"
    for path in (qemu, loader, code, vars_template):
        if not path.is_file():
            raise AssertionError(f"runtime file is missing: {path.relative_to(ARCHIVE)}")
    qemu_command = ["env", f"LD_LIBRARY_PATH={qemu_lib}", str(loader), str(qemu), "-L", str(qemu_share), "-accel", "tcg"]
    version = subprocess.run(qemu_command + ["--version"], check=True, text=True, capture_output=True).stdout
    if "QEMU emulator version 11.0.2" not in version:
        raise AssertionError(f"unexpected QEMU runtime: {version.strip()}")

    os.environ["ARENA_QEMU"] = " ".join(qemu_command)
    os.environ["ARENA_OVMF_CODE"] = str(code)
    os.environ["ARENA_OVMF_VARS"] = str(vars_template)
    os.environ["ARENA_PHASE15_HISTORICAL_DIAGNOSTIC"] = "1"
    print(f"PASS emulator {version.splitlines()[0]} TCG; no hardware-backend claim", flush=True)
    return qemu_command, vars_template


def verify_diagnostic_boot() -> None:
    sys.path.insert(0, str(SOURCE / "tools"))
    import arena_env  # type: ignore[import-not-found]
    import mtest  # type: ignore[import-not-found]

    esp = DIAGNOSTIC / "arena-esp.img"
    efi = DIAGNOSTIC / "arena-boot.efi"
    mtest.build = lambda _label, desktop=False: esp  # type: ignore[assignment]
    rc, serial, elapsed = mtest.run_qemu("phase15-checkpoint-diagnostic", esp)
    log = arena_env.build_dir() / "serial-phase15-checkpoint-diagnostic.log"
    log.write_text(serial)
    if rc != 0:
        raise AssertionError(f"diagnostic boot verdict rc={rc}; see {log}")
    for milestone, count in ((1, 8), (2, 21), (3, 13), (4, 9), (5, 6), (6, 6), (7, 2), (11, 8), (12, 7)):
        marker = f"m{milestone}: RESULT PASS ({count}/{count})"
        if marker not in serial:
            raise AssertionError(f"diagnostic boot omitted {marker}; see {log}")
    required = (
        "m15: DIAGNOSTIC PASS (5/5; XF NOT RUN)",
        "m15:test:thread_isolation_block_yield_preempt: PASS",
        "m15:test:process_isolation_slot_reuse: PASS",
        "m15:test:optional_cpu_feature_reporting: PASS",
        "m15:test:mf_ud_fault_containment: PASS",
        "m15:test:mmx_state: PASS",
        "halting via UEFI ResetSystem(shutdown)",
    )
    for marker in required:
        if marker not in serial:
            raise AssertionError(f"diagnostic boot omitted {marker!r}; see {log}")
    if re.search(r"^m15: RESULT PASS", serial, re.MULTILINE):
        raise AssertionError("diagnostic-only Phase 15 result was mislabeled as strict acceptance")
    print(f"PASS diagnostic guest boot M1-M7/M11-M12, M15 5/5 without #XF, clean shutdown ({elapsed:.1f}s)", flush=True)


def verify_phase14_pie() -> None:
    sys.path.insert(0, str(SOURCE / "tools"))
    import mtest  # type: ignore[import-not-found]
    import test_phase14_pie_guest  # type: ignore[import-not-found]

    esp = DIAGNOSTIC / "arena-esp.img"
    mtest.build = lambda _label, desktop=False: esp  # type: ignore[assignment]
    test_phase14_pie_guest.main()
    print("PASS signed Phase-14 APB1 install and two distinct native PIE placements with teardown", flush=True)


def main() -> None:
    manifest = check_manifest()
    require_hash(DIAGNOSTIC / "arena-boot.efi", EFI_SHA)
    require_hash(DIAGNOSTIC / "arena-esp.img", ESP_SHA)
    require_hash(STRICT / "arena-boot.efi", STRICT_EFI_SHA)
    require_hash(STRICT / "arena-esp.img", STRICT_ESP_SHA)
    fixture_dir = ARTIFACTS / "fixtures"
    require_hash(fixture_dir / "arena-phase15-fpu-fixture", STATE_FIXTURE_SHA)
    require_hash(fixture_dir / "arena-phase15-xf-probe", XF_FIXTURE_SHA)
    require_hash(fixture_dir / "arena-phase15-mmx-probe", MMX_FIXTURE_SHA)
    require_hash(fixture_dir / "phase14-pie.elf", PIE_SHA)
    require_hash(fixture_dir / "phase14-pie.apb1", APB1_SHA)
    verify_esp(DIAGNOSTIC / "arena-esp.img", DIAGNOSTIC / "arena-boot.efi")
    verify_esp(STRICT / "arena-esp.img", STRICT / "arena-boot.efi")
    prepare_environment()
    verify_diagnostic_boot()
    verify_phase14_pie()
    print("CHECKPOINT VERIFICATION COMPLETE: diagnostic evidence only; genuine #XF remains blocked; not a qualified release candidate")


if __name__ == "__main__":
    try:
        main()
    except Exception as error:
        sys.exit(f"PHASE 15 DIAGNOSTIC CHECKPOINT VERIFICATION FAILED: {error}")
