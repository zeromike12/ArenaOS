#!/usr/bin/env python3
"""Phase 16: build the FP/SSE2 preservation app under the ADR-0113 profile.

This is intentionally separate from run.py: the canonical runtime and apps
keep their integer-only flags and the mandatory no-FP ISA audit. This builder
compiles the same canonical runtime sources plus app/c_fp_app.c with the
Phase 15 baseline ring-3 profile (-march=x86-64 -msse2, AVX family disabled)
and applies a *positive* audit instead: SSE2 instructions must be present
(the proof is meaningless otherwise) and AVX/AVX-512 must be absent.

Usage:  python3 build_fp_app.py [--cc gcc|clang|both]
Output: build/fp-<cc>/c-fp-<cc>.elf
Status: build/static evidence; guest execution is P16.5 (docs/phase16/PLAN.md).
"""
from __future__ import annotations

import argparse
import os
import subprocess
import sys
from pathlib import Path

EXP = Path(__file__).resolve().parent
sys.path.insert(0, str(EXP))
import run as c2run  # reuse the canonical source list and compiler table

FP_FLAGS = [
    # Replace run.py's integer-only block (-mgeneral-regs-only -msoft-float
    # -mno-sse -mno-sse2 -mno-mmx -mno-80387) with the ADR-0113 baseline.
    "-march=x86-64", "-msse2", "-mno-avx", "-mno-avx2", "-mno-avx512f",
]
INTEGER_ONLY = {
    "-mgeneral-regs-only", "-msoft-float", "-mno-sse", "-mno-sse2",
    "-mno-mmx", "-mno-80387",
}


def fp_flags(root: Path, cc: str) -> list[str]:
    flags = [f for f in c2run.common_flags(root) if f not in INTEGER_ONLY] + FP_FLAGS
    if cc == "clang":
        # zig cc mis-parses -march=x86-64 (reports unknown CPU 'x86'); the
        # x86_64-freestanding target already defaults to the baseline CPU
        # with SSE2, so only the AVX refusals are needed explicitly.
        flags = [f for f in flags if f != "-march=x86-64"]
    return flags


def build(cc: str) -> Path:
    out = EXP / "build" / f"fp-{cc}"
    obj = out / "obj"
    obj.mkdir(parents=True, exist_ok=True)
    flags = fp_flags(EXP, cc)
    compiler = c2run.COMPILERS[cc]
    objects = []
    for src in c2run.RUNTIME_SRC:
        o = obj / (Path(src).stem + ".o")
        c2run.run(compiler + flags + ["-c", str(EXP / src), "-o", str(o)])
        objects.append(o)
    app_o = obj / "c_fp_app.o"
    c2run.run(compiler + flags + ["-c", str(EXP / "app/c_fp_app.c"), "-o", str(app_o)])
    elf = out / f"c-fp-{cc}.elf"
    crt0 = next(o for o in objects if o.name == "crt0.o")
    rest = [o for o in objects if o.name != "crt0.o"]
    c2run.run(
        compiler + flags + c2run.LINK_FLAGS
        + ["-Wl,-T," + str(EXP / "link/arena-user.ld"), str(crt0), str(app_o)]
        + [str(o) for o in rest] + ["-o", str(elf)]
    )
    return elf


def audit(elf: Path) -> None:
    """SSE2 present, AVX absent — checked from the disassembly, not flags."""
    dis = subprocess.run(
        ["objdump", "-d", str(elf)], capture_output=True, text=True, check=True
    ).stdout.lower()
    sse2 = sum(dis.count(m) for m in ("movsd", "addsd", "mulsd", "divsd", "movss", "addss"))
    avx = sum(dis.count(m) for m in ("ymm", "zmm", "vmov", "vadd", "vmul"))
    x87 = sum(dis.count(m) for m in ("fld", "fmul", "fstp"))
    undef = subprocess.run(["nm", "-u", str(elf)], capture_output=True, text=True).stdout.strip()
    print(f"{elf}: sse2={sse2} x87={x87} avx={avx} undefined=[{undef}]")
    if sse2 == 0:
        raise SystemExit(f"FAIL {elf}: no SSE2 instructions — FP proof would be vacuous")
    if avx:
        raise SystemExit(f"FAIL {elf}: AVX-family instructions present")
    if undef:
        raise SystemExit(f"FAIL {elf}: undefined symbols: {undef}")
    print(f"FP-AUDIT PASS {elf.name}")


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--cc", choices=["gcc", "clang", "both"], default="both")
    args = ap.parse_args()
    for cc in (["gcc", "clang"] if args.cc == "both" else [args.cc]):
        audit(build(cc))


if __name__ == "__main__":
    main()
