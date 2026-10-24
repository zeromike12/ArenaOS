#!/usr/bin/env python3
"""Verify patched/unpatched QEMU results for the independent #XM controls."""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


PATCHED_MARKERS = (
    "PASS: valid SQRTSS, invalid unmasked, no fault",
    "PASS: masked invalid SQRTSS sets IE without vector 19",
    "PASS: old IE unmasked; valid SQRTSS did not fault and kept IE",
    "PASS: invalid SQRTSS faulted again with IE already sticky; XMM0[127:0] preserved",
    "PASS: saved RIP matched the exact unmasked SQRTSS opcode",
    "PASS: unmasked scalar SQRTSS preserved all 128 XMM0 bits",
    "PASS: CR4.OSXMMEXCPT=0 selected vector 6",
    "PASS: saved RIP matched the exact OSXMMEXCPT-disabled opcode",
    "PASS: unmasked DIVSS divide-by-zero delivered vector 19, ZE=1",
    "PASS: saved RIP matched the exact DIVSS opcode",
    "PASS: a new unmasked ZE fault delivered vector 19 with old IE already set",
    "PASS: DIVSS exception preserved all 128 destination bits",
    "PASS: sticky IE preserved with new ZE; full XMM0 destination preserved",
    "PASS: packed SQRTPS exceptional lane faulted; all 128 XMM0 bits preserved",
    "PASS: actual saved RIP matched all six faulting opcode addresses",
    "RESULT PASS: expanded independent TCG #XM controls",
)
UNPATCHED_PASS = (
    "PASS: valid SQRTSS, invalid unmasked, no fault",
    "PASS: masked invalid SQRTSS sets IE without vector 19",
    "PASS: old IE unmasked; valid SQRTSS did not fault and kept IE",
    "PASS: sticky IE=1 was present before the new ZE operation",
)
UNPATCHED_FAIL = (
    "FAIL: newly raised IE did not fault with IE already sticky",
    "FAIL: unmasked invalid #XM or XMM0 preservation control",
    "FAIL: OSXMMEXCPT-disabled control",
    "FAIL: unmasked divide-by-zero control",
    "FAIL: sticky IE plus new ZE did not deliver vector 19",
    "FAIL: packed SQRTPS precise exception/destination control",
    "FAIL: expected total vector or precise-RIP receipt count",
    "RESULT FAIL: expanded independent TCG #XM controls; one or more controls failed",
)


def require_markers(serial: str, markers: tuple[str, ...], label: str) -> dict[str, bool]:
    results = {marker: serial.count(marker) == 1 for marker in markers}
    failed = [marker for marker, passed in results.items() if not passed]
    if failed:
        raise SystemExit(f"{label} control log missing/duplicated markers: {failed}")
    return results


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--manifest", type=Path, required=True)
    parser.add_argument("--source", type=Path, required=True)
    parser.add_argument("--efi", type=Path, required=True)
    parser.add_argument("--esp", type=Path, required=True)
    parser.add_argument("--patched-binary", type=Path, required=True)
    parser.add_argument("--unpatched-binary", type=Path, required=True)
    parser.add_argument("--patched-log", type=Path, required=True)
    parser.add_argument("--unpatched-log", type=Path, required=True)
    parser.add_argument("--patched-stderr", type=Path, required=True)
    parser.add_argument("--unpatched-stderr", type=Path, required=True)
    parser.add_argument("--patched-exit", type=Path, required=True)
    parser.add_argument("--unpatched-exit", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()

    manifest = json.loads(args.manifest.read_text())
    controls = manifest["independent_controls"]
    emulator = manifest["emulator"]
    expected = {
        "source": controls["source_sha256"],
        "efi": controls["efi_sha256"],
        "esp": controls["esp_sha256"],
        "patched_binary": emulator["binary_sha256"],
        "unpatched_binary": emulator["unpatched_binary_sha256"],
    }
    paths = {
        "source": args.source,
        "efi": args.efi,
        "esp": args.esp,
        "patched_binary": args.patched_binary,
        "unpatched_binary": args.unpatched_binary,
    }
    actual = {}
    for name, path in paths.items():
        if not path.is_file():
            raise SystemExit(f"missing {name}: {path}")
        actual[name] = sha256(path)
        if actual[name] != expected[name]:
            raise SystemExit(f"{name} hash mismatch: expected {expected[name]}, got {actual[name]}")

    patched = args.patched_log.read_text(errors="replace")
    unpatched = args.unpatched_log.read_text(errors="replace")
    patched_exit = int(args.patched_exit.read_text().strip())
    unpatched_exit = int(args.unpatched_exit.read_text().strip())
    if patched_exit != 33 or unpatched_exit != 35:
        raise SystemExit(f"unexpected isa-debug-exit statuses: patched={patched_exit}, unpatched={unpatched_exit}")
    patched_checks = require_markers(patched, PATCHED_MARKERS, "patched")
    unpatched_pass = require_markers(unpatched, UNPATCHED_PASS, "unpatched control").keys()
    unpatched_fail = require_markers(unpatched, UNPATCHED_FAIL, "unpatched expected-failure control").keys()
    for path in (args.patched_stderr, args.unpatched_stderr):
        if not path.is_file():
            raise SystemExit(f"missing QEMU stderr receipt: {path}")

    receipt = {
        "schema_version": 1,
        "backend_label": "locally modified QEMU TCG; source-matched unpatched QEMU control",
        "accelerator": "tcg (explicit -accel tcg)",
        "source_sha256": actual["source"],
        "control_efi_sha256": actual["efi"],
        "control_esp_sha256": actual["esp"],
        "patched_qemu_sha256": actual["patched_binary"],
        "unpatched_qemu_sha256": actual["unpatched_binary"],
        "patched_qemu_exit": patched_exit,
        "patched_semantic_result": "PASS",
        "patched_checks": patched_checks,
        "unpatched_qemu_exit": unpatched_exit,
        "unpatched_semantic_result": "EXPECTED FAIL: TCG did not deliver SIMD floating-point exceptions",
        "unpatched_controls_observed_pass": list(unpatched_pass),
        "unpatched_controls_observed_fail": list(unpatched_fail),
        "raw_logs": {
            "patched_serial_sha256": sha256(args.patched_log),
            "unpatched_serial_sha256": sha256(args.unpatched_log),
            "patched_stderr_sha256": sha256(args.patched_stderr),
            "unpatched_stderr_sha256": sha256(args.unpatched_stderr),
        },
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(receipt, indent=2, sort_keys=True) + "\n")
    print(f"patched controls PASS; unpatched baseline failed expected exception cases; receipt={args.output}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
