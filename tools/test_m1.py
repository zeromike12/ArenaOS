#!/usr/bin/env python3
"""Milestone 1 automated boot test (docs/TESTING.md, ADR-0005).

Pipeline: build kernel image -> fresh ESP image -> fresh OVMF_VARS -> boot in
QEMU/EDK2 headless -> capture serial -> assert marker grammar -> assert the VM
terminated by itself (kernel called UEFI ResetSystem(shutdown); -no-reboot
makes QEMU exit 0) within the timeout.

Verdict logic (three distinct failure modes):
  * timeout            -> kernel hung
  * nonzero QEMU exit  -> kernel crashed / reset loop
  * markers            -> kernel ran but a self-test failed or panicked

Exit code: 0 = PASS, 1 = FAIL (with the serial tail printed for diagnosis).
"""

import re
import shutil
import subprocess
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import arena_env  # noqa: E402
import espimg  # noqa: E402

MILESTONE = "m1"
EXPECTED_TESTS = [
    "serial_loopback",
    "cpu_long_mode",
    "cpuid_sane",
    "gdt_installed",
    "idt_installed",
    "memory_map",
    "wide_math",
    "timer_irq_absorbed",
]
TIMEOUT_S = 120  # TCG is slow; a healthy boot takes <10s
MEM_MIB = 512


def build() -> Path:
    print("[test-m1] building kernel image + ESP ...")
    subprocess.run(
        ["bash", str(arena_env.REPO_ROOT / "tools/build.sh"), "--image"],
        check=True,
        capture_output=True,
        text=True,
    )
    esp = arena_env.REPO_ROOT / "build/arena-esp.img"
    assert esp.exists(), "build.sh did not produce the ESP image"
    return esp


def run_qemu(esp: Path) -> tuple[int, str, float]:
    bdir = arena_env.build_dir()
    vars_img = bdir / "ovmf-vars.img"
    shutil.copyfile(arena_env.ovmf_vars_template(), vars_img)  # fresh NVRAM every run
    serial_log = bdir / "serial.log"
    if serial_log.exists():
        serial_log.unlink()

    cmd = (
        arena_env.qemu_cmd()
        + arena_env.qemu_data_args()
        + [
            "-M", "q35",
            "-m", f"{MEM_MIB}M",
            "-cpu", "qemu64,+nx",
            "-drive", f"if=pflash,format=raw,readonly=on,file={arena_env.ovmf_code()}",
            "-drive", f"if=pflash,format=raw,file={vars_img}",
            "-drive", f"format=raw,file={esp}",
            "-display", "none",
            "-serial", f"file:{serial_log}",
            "-no-reboot",
        ]
    )
    print("[test-m1] booting QEMU/EDK2:", " ".join(cmd[:3]), "...")
    t0 = time.monotonic()
    proc = subprocess.run(cmd, capture_output=True, text=True, timeout=TIMEOUT_S)
    dt = time.monotonic() - t0
    serial = serial_log.read_text(errors="replace") if serial_log.exists() else ""
    return proc.returncode, serial, dt


def evaluate(rc: int, serial: str, dt: float) -> bool:
    ok = True

    def check(cond: bool, msg: str) -> None:
        nonlocal ok
        print(f"[test-m1] {'PASS' if cond else 'FAIL'}: {msg}")
        ok = ok and cond

    if "PANIC" in serial:
        # Surface the panic line for diagnosis before anything else.
        for line in serial.splitlines():
            if "PANIC" in line:
                print(f"[test-m1] kernel panic marker: {line.strip()}")
    check("PANIC" not in serial, "no kernel panic on serial")

    for name in EXPECTED_TESTS:
        m = re.search(rf"^{MILESTONE}:test:{re.escape(name)}: (PASS|FAIL)(.*)$",
                      serial, re.MULTILINE)
        if m is None:
            check(False, f"self-test '{name}' reported (marker missing)")
        else:
            reason = m.group(2).strip()
            check(m.group(1) == "PASS", f"self-test '{name}' {reason}")

    m = re.search(rf"^{MILESTONE}: RESULT (PASS|FAIL) \((\d+)/(\d+)\)$", serial, re.MULTILINE)
    check(m is not None, "milestone RESULT line present")
    if m:
        check(m.group(1) == "PASS", f"RESULT is PASS ({m.group(2)}/{m.group(3)})")

    # rc==0 alone does NOT prove a clean halt: a triple fault also ends with
    # QEMU exiting 0 under -no-reboot. The kernel's declared-halt log line is
    # the discriminator (it is printed only on the ResetSystem path).
    check(
        "halting via UEFI ResetSystem(shutdown)" in serial,
        "kernel declared its clean halt (ResetSystem path reached)",
    )
    check(rc == 0, f"QEMU exited cleanly via kernel ResetSystem shutdown (rc={rc}, {dt:.1f}s)")
    return ok


def main() -> int:
    esp = build()
    try:
        rc, serial, dt = run_qemu(esp)
    except subprocess.TimeoutExpired:
        print(f"[test-m1] FAIL: VM did not terminate within {TIMEOUT_S}s (kernel hang?)")
        return 1
    arena_env.build_dir().joinpath("serial-m1.log").write_text(serial)
    ok = evaluate(rc, serial, dt)
    print(f"[test-m1] {'=' * 46}")
    print(f"[test-m1] MILESTONE 1: {'PASS' if ok else 'FAIL'}  (serial: build/serial-m1.log)")
    if not ok:
        tail = [ln for ln in serial.splitlines() if ln.strip()][-25:]
        print("[test-m1] serial tail:")
        for ln in tail:
            print("   |", ln)
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
