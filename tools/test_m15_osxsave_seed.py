#!/usr/bin/env python3
"""Build and boot the diagnostic XCR0=7 entry path on an AVX-capable CPU."""

from __future__ import annotations

import hashlib
import json
import os
import subprocess
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import arena_env  # noqa: E402
import mtest  # noqa: E402

ROOT = arena_env.REPO_ROOT
BUILD = arena_env.build_dir()
LABEL = "phase15-osxsave-seed"


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main() -> int:
    env = os.environ.copy()
    rust_bin = arena_env.rust_bin()
    env["ARENA_RUST_BIN"] = str(rust_bin)
    env["PATH"] = f"{rust_bin}:{env.get('PATH', '')}"
    env.pop("ARENA_GRAPHICS_FIXTURE", None)
    env["ARENA_PHASE15_HISTORICAL_DIAGNOSTIC"] = "1"
    env["ARENA_PHASE15_OSXSAVE_SEED"] = "1"
    command = ["bash", str(ROOT / "tools/build.sh"), "--image"]
    print(f"[{LABEL}] building controlled diagnostic image")
    with (BUILD / f"{LABEL}-build.log").open("w") as log:
        built = subprocess.run(command, cwd=ROOT, env=env, stdout=log,
                               stderr=subprocess.STDOUT, check=False)
    if built.returncode:
        print(f"build failed; see build/{LABEL}-build.log", file=sys.stderr)
        return built.returncode

    efi = BUILD / "arena-boot.efi"
    esp = BUILD / "arena-esp.img"
    qemu_version = subprocess.run(
        arena_env.qemu_cmd() + arena_env.qemu_data_args() + ["--version"],
        check=True, capture_output=True, text=True,
    ).stdout.splitlines()[0]
    source = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT,
                                     text=True).strip()
    rc, serial, elapsed = mtest.run_qemu(
        LABEL,
        esp,
        cpu="max,+nx,+smep,+smap",
    )
    serial_path = BUILD / f"serial-{LABEL}.log"
    serial_path.write_text(serial)
    expected = (
        rc == 0
        and "m15: OSXSAVE diagnostic entry seeded: CPUID.XSAVE=true AVX=true CR4.OSXSAVE=true XCR0=0x7" in serial
        and "firmware OSXSAVE was true" in serial
        and "XCR0=Some(3) OSXSAVE=off" in serial
        and "m15: DIAGNOSTIC PASS (5/5; XF NOT RUN)" in serial
        and "m15: RESULT PASS" not in serial
        and "halting via UEFI ResetSystem(shutdown)" in serial
    )
    receipt = {
        "source_commit": source,
        "efi_sha256": sha256(efi),
        "esp_sha256": sha256(esp),
        "backend": qemu_version,
        "accelerator": "TCG (no KVM/HVF device available)",
        "cpu_profile": "max,+nx,+smep,+smap",
        "mode": "diagnostic only; #XF acceptance not run",
        "attempted_boots": 1,
        "successful_boots": int(expected),
        "failed_boots": int(not expected),
        "timeouts": int(rc == 124),
        "duration_seconds": round(elapsed, 3),
        "seeded_state": "CR4.OSXSAVE=1, XCR0=0x7",
        "expected_sanitized_state": "XCR0=0x3, CR4.OSXSAVE=0",
        "serial_log": serial_path.name,
    }
    (BUILD / f"{LABEL}-receipt.json").write_text(
        json.dumps(receipt, indent=2, sort_keys=True) + "\n"
    )
    print(json.dumps(receipt, indent=2, sort_keys=True))
    print(f"[{LABEL}] {'PASS' if expected else 'FAIL'}")
    return 0 if expected else 1


if __name__ == "__main__":
    raise SystemExit(main())
