#!/usr/bin/env python3
"""Turn a completed stability-loop log into a Phase 15 JSON receipt."""
from __future__ import annotations

import argparse
import hashlib
import json
import re
import subprocess
from datetime import datetime, timezone
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
ARTIFACT_SOURCE = "6586a0a18981932ae7048ca267fa425411f8e463"
EFI_SHA = "e85e0b5dd9e96f3129e7f2385d548084c56c16801432a9d41243b9ebc23a2282"
ESP_SHA = "b7a50113498033dd488ccd62acfebeed96d6a483d891cffed997cb79ef09bf42"
QEMU_SHA = "0539587908d9a9b83ccdce7a48d968b50f827476b9852814e07445e9402df562"
STATE_FIXTURE_SHA = "786808ad21686ace59684f01ed1e69b47bd104c53442b2d7df9e646b9103b32b"
XF_FIXTURE_SHA = "1f057068b020baefe06f7f3b36bc0beeafcb879bd93d60c5ea66a3cce616c8d3"
PIE_SHA = "d7b0a8cf9c735c3898a867d824563f06b0d949df80fa1a9c96f9180a395fea2e"
APB1_SHA = "04add54f09042353fc42511978c82f8b8efb87f7d5f0657f9116f456276b3dfa"
EXPECTED = {2: 10, 3: 25, 4: 100}
FINAL = re.compile(r"stability: (\d+)/(\d+) boots fully green in (\d+)s \(fail=(\d+)\)")


def sha(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("tier", type=int, choices=(2, 3, 4))
    parser.add_argument("log", type=Path)
    parser.add_argument("output", type=Path)
    parser.add_argument("--driver-commit", required=True, help="commit checked out when this loop was started")
    args = parser.parse_args()
    log_path = args.log if args.log.is_absolute() else ROOT / args.log
    text = log_path.read_text(errors="replace")
    rows = FINAL.findall(text)
    if not rows:
        raise SystemExit("completed stability summary is missing")
    successful, attempted, duration, failed = map(int, rows[-1])
    required = EXPECTED[args.tier]
    if (successful, attempted, failed) != (required, required, 0):
        raise SystemExit(f"stability run is not green: {successful}/{attempted}, failures={failed}")
    if "STABILITY: PASS" not in text:
        raise SystemExit("stability footer does not say PASS")

    driver = ROOT / "tools/stability_loop.sh"
    pixels = []
    if args.tier == 4:
        for boot in range(1, 101):
            receipt = ROOT / f"build/stability-pixels-boot-{boot}.txt"
            if not receipt.is_file():
                raise SystemExit(f"T4 pixel receipt missing: boot {boot}")
            content = receipt.read_text()
            if not all(line in content for line in ("800x600 ", "INPUT ", "DRAG ", "RELAUNCH ", "LIFECYCLE 2 2")):
                raise SystemExit(f"T4 pixel receipt malformed: boot {boot}")
            pixels.append({"boot": boot, "sha256": sha(receipt)})

    receipt = {
        "tier": f"T{args.tier}",
        "mode": "diagnostic-only",
        "official_qualification": "BLOCKED",
        "source_commit": ARTIFACT_SOURCE,
        "driver_commit": args.driver_commit,
        "stability_script_sha256": sha(driver),
        "test_executable_identities": {
            "phase15_kernel_state_fixture_sha256": STATE_FIXTURE_SHA,
            "strict_xf_probe_elf_sha256_included_but_not_run_in_diagnostic_loop": XF_FIXTURE_SHA,
            "phase14_pie_elf_sha256": PIE_SHA,
            "phase14_signed_apb1_sha256": APB1_SHA,
        },
        "efi_sha256": EFI_SHA,
        "esp_sha256": ESP_SHA,
        "efi_bytes": (ROOT / "build/phase15-diagnostic-efi-6586a0a.efi").stat().st_size,
        "esp_bytes": (ROOT / "build/phase15-diagnostic-esp-6586a0a.img").stat().st_size,
        "backend": "QEMU 11.0.2 TCG",
        "qemu_binary_sha256": QEMU_SHA,
        "cpu_profile": "qemu64,+nx,+smep,+smap",
        "attempted_boots": attempted,
        "successful_boots": successful,
        "failed_boots": failed,
        "timeouts": 0,
        "retries": 0,
        "boot_timeout_seconds": 90,
        "duration_seconds": duration,
        "pass_conditions": [
            "fresh OVMF vars and scratch disk on every boot",
            "all in-guest RESULT verdicts match the first boot exactly",
            "shipping network, keyboard, virtio-console, and desktop fixtures pass",
            "TCP request/response/FIN and exactly three host DNS fixture requests",
            "QMP desktop pixels, input, drag, relaunch, and two-process lifecycle",
            "real service-manager restart and clean UEFI ResetSystem shutdown",
            "no panic, semantic failure, timeout, or QEMU nonzero exit",
        ],
        "phase15_acceptance": "diagnostic image reports XF NOT RUN; genuine vector-19 containment not qualified",
        "t4_pixel_receipts": pixels if args.tier == 4 else None,
        "log": str(log_path.relative_to(ROOT)) if log_path.is_relative_to(ROOT) else str(log_path),
        "recorded_at_utc": datetime.now(timezone.utc).isoformat(),
    }
    output = args.output if args.output.is_absolute() else ROOT / args.output
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps(receipt, indent=2, sort_keys=True) + "\n")
    print(f"{receipt['tier']} diagnostic receipt: {successful}/{attempted}, {duration}s, #XF remains BLOCKED")
    print(output)


if __name__ == "__main__":
    main()
