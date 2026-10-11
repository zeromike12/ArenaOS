#!/usr/bin/env python3
"""Run Phase-15 ring-3 FP/SIMD proofs on baseline and AVX-capable QEMU CPUs.

The image must already be built. This runner intentionally does not rebuild
the EFI so it can also verify an exact artifact supplied by a qualification
workflow.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import arena_env  # noqa: E402
import mtest  # noqa: E402

EXPECTED = (
    "thread_isolation_block_yield_preempt",
    "process_isolation_slot_reuse",
    "xf_delivery_probe",
    "mf_ud_fault_containment",
)
SHUTDOWN_FEED = [(b"arena>", 1, b"shutdown\r")]


def check_serial(label: str, serial: str, rc: int | None, avx_expected: bool) -> bool:
    ok = True

    def check(condition: bool, message: str) -> None:
        nonlocal ok
        print(f"[{label}] {'PASS' if condition else 'FAIL'}: {message}")
        ok &= condition

    check(rc == 0, f"guest semantic exit status is zero (rc={rc})")
    check("PANIC" not in serial and "RESULT FAIL" not in serial,
          "no kernel panic or failed in-guest result")
    for name in EXPECTED:
        marker = f"m15:test:{name}: PASS"
        check(serial.count(marker) == 1, f"guest assertion {name}")
    check(serial.count("m15: RESULT PASS (4/4)") == 1,
          "all Phase-15 guest cases passed")
    if "m15:test:xf_delivery_probe: FAIL" in serial:
        fallback = re.search(
            r"minimal #XF probe did not receive vector 19: status=(0x[0-9a-f]+), "
            r"pre-MXCSR=(0x[0-9a-f]+), post-MXCSR=(0x[0-9a-f]+)",
            serial,
        )
        check(
            fallback is not None
            and tuple(int(value, 16) for value in fallback.groups())
            == (0x1F0000001F01, 0x1F00, 0x1F01),
            "failed #XF trial records clean pre-state and invalid-operation post-state",
        )
        check(
            "minimal #XF probe receipts: fault=Some(1f0000001f01), "
            "sibling=Some(2a), peer=Some(2a), sibling_and_peer_survived=true, "
            "fault_slot_released=true, resource_baseline_restored=true" in serial,
            "failed #XF trial records survivor and exact teardown receipts",
        )
    check("halting via UEFI ResetSystem(shutdown)" in serial,
          "kernel reached declared clean shutdown")
    results = re.findall(r"^(m\d+): RESULT (PASS|FAIL) \((\d+)/(\d+)\)$",
                         serial, re.MULTILINE)
    check(bool(results) and all(state == "PASS" for _, state, _, _ in results),
          "every reported historical and Phase-15 result passed")
    configured = re.search(
        r"feature discovery after FP setup:.*\bavx=(true|false)\s+osxsave=(true|false)",
        serial,
    )
    check(configured is not None, "configured CPUID record is present")
    if configured:
        check(configured.group(1) == str(avx_expected).lower(),
              f"QEMU AVX hardware flag is {str(avx_expected).lower()}")
        check(configured.group(2) == "false",
              "OSXSAVE is clear after kernel initialization")
    inherited = re.search(r"firmware OSXSAVE was (true|false)", serial)
    check(inherited is not None, "inherited OSXSAVE state is present")
    if inherited:
        expected_xcr0 = "XCR0=Some(3)" if inherited.group(1) == "true" else "XCR0=None"
        check(expected_xcr0 in serial,
              f"inherited OSXSAVE has the expected XCR0 policy ({expected_xcr0})")
    return ok


def main() -> int:
    esp = arena_env.REPO_ROOT / "build/arena-esp.img"
    efi = arena_env.REPO_ROOT / "build/arena-boot.efi"
    if not esp.is_file() or not efi.is_file():
        print("error: build/arena-boot.efi and build/arena-esp.img must exist; build first",
              file=sys.stderr)
        return 2

    from pyfatfs.PyFatFS import PyFatFS

    fs = PyFatFS(str(esp), read_only=True)
    try:
        if fs.getbytes("/EFI/BOOT/BOOTX64.EFI") != efi.read_bytes():
            print("error: ESP EFI does not match build/arena-boot.efi", file=sys.stderr)
            return 2
    finally:
        fs.close()

    runs = (
        ("phase15-avx", "max,+nx,+smep,+smap", True),
        ("phase15-qemu64", "qemu64,+nx,+smep,+smap", False),
    )
    ok = True
    for label, cpu, avx_expected in runs:
        scratch = arena_env.make_scratch_disk(afs2=True)
        rc, serial, elapsed = mtest.boot(
            label,
            esp,
            SHUTDOWN_FEED,
            scratch,
            timeout_s=mtest.TIMEOUT_S,
            cpu=cpu,
        )
        arena_env.build_dir().joinpath(f"serial-{label}.log").write_text(serial)
        print(f"[{label}] elapsed {elapsed:.1f}s; serial: build/serial-{label}.log")
        ok &= check_serial(label, serial, rc, avx_expected)
    if ok:
        print("Phase-15 native guest qualification: PASS (baseline and AVX-capable CPU profiles)")
        return 0
    print("Phase-15 native guest qualification: FAIL (see both CPU-profile receipts)")
    return 1


if __name__ == "__main__":
    raise SystemExit(main())
