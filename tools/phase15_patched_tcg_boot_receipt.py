#!/usr/bin/env python3
"""Write a strict per-boot receipt for the locally patched QEMU TCG gate."""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path


STRICT_MARKERS = (
    "m15:test:thread_isolation_block_yield_preempt: PASS",
    "m15:test:process_isolation_slot_reuse: PASS",
    "m15:test:optional_cpu_feature_reporting: PASS",
    "m15:test:xf_delivery_probe: PASS",
    "m15:test:mf_ud_fault_containment: PASS",
    "m15:test:mmx_state: PASS",
)
CONTAINMENT = (
    "fault=Some(113), sibling=Some(2a), peer=Some(2a), "
    "sibling_and_peer_survived=true, fault_slot_released=true, "
    "resource_baseline_restored=true"
)


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--manifest", type=Path, required=True)
    parser.add_argument("--serial", type=Path, required=True)
    parser.add_argument("--qemu-stderr", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--boot", type=int, required=True)
    parser.add_argument("--cpu-profile", required=True)
    parser.add_argument("--qemu-exit", type=int, required=True)
    parser.add_argument("--stability-verdict", choices=("PASS", "FAIL"), required=True)
    parser.add_argument("--reason", default="")
    args = parser.parse_args()

    manifest = json.loads(args.manifest.read_text())
    serial = args.serial.read_text(errors="replace")
    qemu_stderr = args.qemu_stderr.read_text(errors="replace")
    emu = manifest["emulator"]
    image = manifest["strict_arenaos_image"]
    firmware = manifest["firmware"]

    result_line = "m15: RESULT PASS (6/6)"
    checks = {
        "qemu_exit_zero": args.qemu_exit == 0,
        "strict_result_pass_6_of_6_once": serial.count(result_line) == 1,
        "all_strict_assertions_once": all(serial.count(marker) == 1 for marker in STRICT_MARKERS),
        "genuine_vector_19_recorded": "vector=0x13 (#XF SIMD Floating Point)" in serial,
        "expected_fault_status_0x113": "fault=Some(113)" in serial,
        "same_process_sibling_and_independent_peer_survived": (
            "sibling_and_peer_survived=true" in serial and "sibling=Some(2a), peer=Some(2a)" in serial
        ),
        "fault_thread_and_scheduler_slot_reclaimed": "fault_slot_released=true" in serial,
        "resource_baseline_restored": "resource_baseline_restored=true" in serial,
        "declared_clean_shutdown": "halting via UEFI ResetSystem(shutdown)" in serial,
        "no_guest_failure_or_panic": "RESULT FAIL" not in serial and "PANIC" not in serial,
    }
    payload = {
        "schema_version": 1,
        "boot": args.boot,
        "cpu_profile": args.cpu_profile,
        "backend_id": manifest["backend_id"],
        "emulator": emu["binary_sha256"],
        "accelerator": "tcg (explicit -accel tcg)",
        "efi_sha256": image["efi_sha256"],
        "esp_sha256": image["esp_sha256"],
        "ovmf_code_sha256": firmware["code_sha256"],
        "ovmf_vars_template_sha256": firmware["vars_template_sha256"],
        "qemu_exit_status": args.qemu_exit,
        "semantic_guest_result": result_line if checks["strict_result_pass_6_of_6_once"] else "FAIL or missing",
        "vector_19_instruction_generated": checks["genuine_vector_19_recorded"],
        "fault_status": "0x113" if checks["expected_fault_status_0x113"] else None,
        "checks": checks,
        "stability_verdict": args.stability_verdict,
        "stability_failure_reason": args.reason,
        "serial_log": {"path": args.serial.name, "sha256": sha256(args.serial)},
        "qemu_stderr_log": {"path": args.qemu_stderr.name, "sha256": sha256(args.qemu_stderr)},
        "qualification_pass": all(checks.values()) and args.stability_verdict == "PASS",
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n")
    print(json.dumps({"boot": args.boot, "qualification_pass": payload["qualification_pass"],
                      "checks": checks}, sort_keys=True))
    return 0 if payload["qualification_pass"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
