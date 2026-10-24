#!/usr/bin/env python3
"""Independently verify a freshly extracted Phase 15 modified-TCG package."""

from __future__ import annotations

import argparse
import hashlib
import json
import subprocess
import sys
import tempfile
from pathlib import Path


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def run(command: list[str], description: str) -> None:
    completed = subprocess.run(command, text=True, stdout=subprocess.PIPE,
                               stderr=subprocess.STDOUT)
    if completed.returncode != 0:
        raise SystemExit(f"{description} failed (exit {completed.returncode}):\n{completed.stdout}")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("package_root", type=Path)
    parser.add_argument("--output", type=Path,
                        help="optional machine-readable verification receipt outside the package")
    args = parser.parse_args()
    root = args.package_root.resolve()
    manifest_path = root / "manifest.json"
    if not manifest_path.is_file():
        raise SystemExit(f"missing package manifest: {manifest_path}")
    manifest = json.loads(manifest_path.read_text())
    if manifest.get("schema_version") != 1:
        raise SystemExit("unsupported Phase 15 package manifest schema")
    files = manifest.get("files")
    if not isinstance(files, dict) or not files:
        raise SystemExit("package manifest has no file hash map")

    expected_paths = set(files)
    actual_paths = {path.relative_to(root).as_posix() for path in root.rglob("*")
                    if path.is_file() and path != manifest_path}
    if actual_paths != expected_paths:
        missing = sorted(expected_paths - actual_paths)
        extra = sorted(actual_paths - expected_paths)
        raise SystemExit(f"package file inventory differs; missing={missing}, unlisted={extra}")
    for name, expected in sorted(files.items()):
        path = (root / name).resolve()
        if root not in path.parents or not path.is_file():
            raise SystemExit(f"unsafe or missing package path: {name}")
        actual = sha256(path)
        if actual != expected:
            raise SystemExit(f"package SHA-256 mismatch for {name}: expected {expected}, got {actual}")

    overlay = root / "overlay"
    backend = overlay / "docs/phase15/patched-tcg-qualification.json"
    source_root = overlay / "docs/phase15/experiments/qemu-tcg-xm-prototype"
    paths = manifest["verification_paths"]
    runner = overlay / "tools/phase15_patched_tcg_qualification.py"
    python = sys.executable

    with tempfile.TemporaryDirectory(prefix="phase15-patched-tcg-verify-") as temporary:
        temp = Path(temporary)
        run([python, str(runner), "verify", str(root / paths["strict_direct"]),
             "--manifest", str(backend)], "strict two-profile receipt verification")
        for tier in ("T2", "T3", "T4"):
            run([python, str(runner), "verify", str(root / paths["stability"][tier]),
                 "--manifest", str(backend)], f"strict {tier} receipt verification")
        run([
            python, str(overlay / "tools/verify_phase15_xm_controls.py"),
            "--manifest", str(backend),
            "--source", str(source_root / "source/independent-xm-controls.s"),
            "--efi", str(root / paths["controls_efi"]),
            "--esp", str(root / paths["controls_esp"]),
            "--patched-binary", str(root / paths["patched_qemu"]),
            "--unpatched-binary", str(root / paths["unpatched_qemu"]),
            "--patched-log", str(source_root / "logs/patched-expanded-controls.serial.log"),
            "--unpatched-log", str(source_root / "logs/unpatched-expanded-controls.serial.log"),
            "--patched-stderr", str(source_root / "logs/patched-expanded-controls.qemu-stderr.log"),
            "--unpatched-stderr", str(source_root / "logs/unpatched-expanded-controls.qemu-stderr.log"),
            "--patched-exit", str(source_root / "logs/patched-expanded-controls.exit"),
            "--unpatched-exit", str(source_root / "logs/unpatched-expanded-controls.exit"),
            "--output", str(temp / "controls-verification.json"),
        ], "expanded independent control verification")

    regression = manifest.get("registered_regression", {})
    regression_log = root / regression.get("log", "")
    if regression.get("verdict") != "PASS" or not regression_log.is_file():
        raise SystemExit("registered full regression is not marked PASS or its raw log is missing")
    marker = f"ALL TESTS PASSED ({regression.get('suite_count')} test suites)"
    if marker not in regression_log.read_text(errors="replace"):
        raise SystemExit(f"full-regression log does not contain its claimed suite count: {marker}")

    final = manifest.get("final_source_checks", {})
    expected_phase14 = {
        "pie_sha256": "d7b0a8cf9c735c3898a867d824563f06b0d949df80fa1a9c96f9180a395fea2e",
        "signed_apb1_sha256": "04add54f09042353fc42511978c82f8b8efb87f7d5f0657f9116f456276b3dfa",
    }
    if final.get("canonical_build") != "PASS" or final.get("kernel_isa_audit") != "PASS":
        raise SystemExit("canonical build or kernel ISA audit is not marked PASS")
    if any(final.get(key) != value for key, value in expected_phase14.items()):
        raise SystemExit("protected Phase 14 PIE/APB1 identities changed or are absent")
    if final.get("host_tests") != "PASS":
        raise SystemExit("required host tests are not marked PASS")

    receipt = {
        "schema_version": 1,
        "package_manifest_sha256": sha256(manifest_path),
        "file_hashes_verified": len(files),
        "strict_two_profile": "PASS",
        "strict_stability": {tier: "PASS" for tier in ("T2", "T3", "T4")},
        "independent_controls": "PASS patched; expected failures observed unpatched",
        "registered_regression": {
            "suite_count": regression["suite_count"],
            "verdict": regression["verdict"],
        },
        "final_source_checks": final,
        "backend_label": "locally modified QEMU TCG; not upstream-QEMU or hardware-backed",
        "phase15_acceptance": "qualification candidate pending M/C review",
    }
    if args.output:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(json.dumps(receipt, indent=2, sort_keys=True) + "\n")
    print(json.dumps(receipt, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
