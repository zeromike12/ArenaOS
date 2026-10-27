#!/usr/bin/env python3
"""Run the Phase 15 independent #XM backend controls on both binaries.

This is the existing backend-suitability gate (see
docs/phase16/BACKEND-RECOVERY.md and tools/verify_phase15_xm_controls.py),
wrapped as one command so any new environment can re-establish — rather
than inherit — that the locally modified QEMU 11.1.2 TCG binary really
delivers genuine SIMD floating-point exceptions (vector 19):

  patched    must print RESULT PASS and exit 33 via isa-debug-exit
  unpatched  must expected-FAIL the same controls and exit 35

Both run the packaged control ESP from the Phase 15 qualification
archive with fresh OVMF vars, explicit TCG (the isolated-glibc wrappers
add ``-accel tcg``), and the manifest verifier checks every marker and
every hash. Raw serials, QEMU stderr, and exit codes are preserved in
``$ARENA_TEST_ATTEMPT_DIR/xm/`` when run under the new runner, else in
``build/backend-controls/``.

Environment: ARENA_PHASE15_PKG (default ~/phase15-pkg, the extracted
docs/phase15 qualification archive), ARENA_QEMU_PATCHED /
ARENA_QEMU_UNPATCHED (default ~/phase16-backend/qemu-{patched,unpatched}).
"""

from __future__ import annotations

import os
import shutil
import subprocess
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent


def main() -> int:
    home = Path.home()
    pkg = Path(os.environ.get("ARENA_PHASE15_PKG", home / "phase15-pkg"))
    wrappers = {
        "patched": Path(os.environ.get("ARENA_QEMU_PATCHED",
                                        home / "phase16-backend/qemu-patched")),
        "unpatched": Path(os.environ.get("ARENA_QEMU_UNPATCHED",
                                          home / "phase16-backend/qemu-unpatched")),
    }
    binaries = {
        "patched": pkg / "backend/qemu-system-x86_64-patched",
        "unpatched": pkg / "backend/qemu-system-x86_64-unpatched",
    }
    esp = pkg / "controls/independent-xm-controls-esp.img"
    efi = pkg / "controls/independent-xm-controls.efi"
    code = pkg / "firmware/OVMF_CODE_4M.fd"
    vars_template = pkg / "firmware/OVMF_VARS_4M.fd"
    for p in (esp, efi, code, vars_template, *binaries.values(),
              *wrappers.values()):
        if not p.is_file():
            print(f"FAIL: required backend-control input missing: {p}",
                  file=sys.stderr)
            return 2
    attempt = os.environ.get("ARENA_TEST_ATTEMPT_DIR")
    out = (Path(attempt) / "xm" if attempt else REPO / "build/backend-controls")
    out.mkdir(parents=True, exist_ok=True)

    for kind in ("patched", "unpatched"):
        vars_img = out / f"ovmf-vars-{kind}.fd"
        shutil.copyfile(vars_template, vars_img)
        cmd = [
            str(wrappers[kind]), "-machine", "q35", "-m", "128M",
            "-cpu", "qemu64,+nx,+smep,+smap",
            "-drive", f"if=pflash,format=raw,readonly=on,file={code}",
            "-drive", f"if=pflash,format=raw,file={vars_img}",
            "-drive", f"if=none,id=esp0,format=raw,file={esp}",
            "-device", "ide-hd,drive=esp0,bootindex=0",
            "-display", "none", "-serial", "stdio", "-monitor", "none",
            "-no-reboot", "-device", "isa-debug-exit,iobase=0xf4,iosize=0x04",
        ]
        with open(out / f"serial-{kind}.log", "wb") as logf, \
                open(out / f"qemu-stderr-{kind}.log", "wb") as errf:
            try:
                rc = subprocess.run(cmd, stdin=subprocess.DEVNULL,
                                    stdout=logf, stderr=errf,
                                    timeout=120).returncode
            except subprocess.TimeoutExpired:
                rc = 124
        (out / f"exit-{kind}.txt").write_text(f"{rc}\n")
        print(f"[backend-controls] {kind}: qemu exit {rc}")

    verify = subprocess.run([
        sys.executable, str(REPO / "tools/verify_phase15_xm_controls.py"),
        "--manifest", str(REPO / "docs/phase15/patched-tcg-qualification.json"),
        "--source", str(REPO / "docs/phase15/experiments/qemu-tcg-xm-prototype"
                              "/source/independent-xm-controls.s"),
        "--efi", str(efi), "--esp", str(esp),
        "--patched-binary", str(binaries["patched"]),
        "--unpatched-binary", str(binaries["unpatched"]),
        "--patched-log", str(out / "serial-patched.log"),
        "--unpatched-log", str(out / "serial-unpatched.log"),
        "--patched-stderr", str(out / "qemu-stderr-patched.log"),
        "--unpatched-stderr", str(out / "qemu-stderr-unpatched.log"),
        "--patched-exit", str(out / "exit-patched.txt"),
        "--unpatched-exit", str(out / "exit-unpatched.txt"),
        "--output", str(out / "xm-verify.json"),
    ])
    print(f"[backend-controls] verifier exit {verify.returncode}; evidence in {out}")
    return verify.returncode


if __name__ == "__main__":
    raise SystemExit(main())
