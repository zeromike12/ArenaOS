#!/usr/bin/env python3
"""C2 ISA audit (EXPERIMENTAL): scans only FUNC symbol ranges for FP/SIMD instructions.

A whole-section scan misreads data in .text (pointer tables decode as x87 opcodes), so
this walks each FUNC symbol's [addr, addr+size) range with objdump and checks:
  - any instruction with an xmm/ymm/zmm/mm/st operand  -> FAIL
  - any x87 mnemonic (f-prefixed arithmetic/load/store/control)  -> FAIL
Exit 0 only when every audited ELF has zero hits and at least one function was scanned.

Usage: python3 c2_isa_audit.py build/guest-gcc/c2-console-gcc.elf ...
"""
from __future__ import annotations

import json
import re
import subprocess
import sys
from pathlib import Path

VEC_OPERAND = re.compile(r"%(xmm|ymm|zmm|mm)\d+|%st(\(\d\))?(?![a-z0-9])")
X87_MNEMONIC = re.compile(
    r"^(f(add|sub|subr|mul|div|divr|ld|st|stp|xch|ild|ist|istp|nstcw|ldcw|nstsw|clex|"
    r"init|abs|chs|sqrt|sin|cos|rndint|comi|ucomi|nop|wait|ldenv|nenv|rstor|save|"
    r"prem|scale|xtract|ucom|com|tst|xam|ldz|ldpi|ldl2|ldl2e|ldln2|ldlg2|f2xm1|fyl2x|"
    r"incstp|decstp|ffree|fcmov|fucom|fcom|fbld|fbstp|fpatan|fptan|fyl2xp1|fprem1|fdecstp|"
    r"fincstp|fcmov)[a-z]*|fwait)$")
FUNC_LINE = re.compile(r"^\s*\d+:\s+([0-9a-f]+)\s+(\d+)\s+FUNC\s+\S+\s+\S+\s+\S+\s+\S+\s+(\S+)")


def functions(elf: Path) -> list[tuple[int, int, str]]:
    out = subprocess.run(["readelf", "-sW", str(elf)], capture_output=True, text=True, check=True).stdout
    funcs = []
    for line in out.splitlines():
        parts = line.split()
        # readelf -sW: Num: Value Size Type Bind Vis Ndx Name
        if len(parts) >= 8 and parts[3] == "FUNC" and parts[6] != "UND":
            try:
                addr, size = int(parts[1], 16), int(parts[2])
            except ValueError:
                continue
            if size:
                funcs.append((addr, size, parts[7]))
    return funcs


def scan(elf: Path) -> dict:
    funcs = functions(elf)
    lo_hi = [(a, a + s, n) for a, s, n in funcs]
    dis = subprocess.run(["objdump", "-d", "--no-show-raw-insn", str(elf)],
                         capture_output=True, text=True, check=True).stdout
    vec_hits, x87_hits, scanned = [], [], 0
    ranges = sorted(lo_hi)
    for line in dis.splitlines():
        m = re.match(r"^\s*([0-9a-f]+):\t(.*)$", line)
        if not m:
            continue
        addr = int(m.group(1), 16)
        body = m.group(2).strip()
        if not body:
            continue
        owner = None
        for lo, hi, name in ranges:
            if lo <= addr < hi:
                owner = name
                break
        if owner is None:
            continue  # not inside a FUNC symbol (data or padding): not code
        scanned += 1
        mn = body.split()[0]
        if VEC_OPERAND.search(body):
            vec_hits.append({"addr": hex(addr), "fn": owner, "insn": body})
        if X87_MNEMONIC.match(mn):
            x87_hits.append({"addr": hex(addr), "fn": owner, "insn": body})
    return {"elf": str(elf), "functions": len(funcs), "instructions_scanned": scanned,
            "vector_operand_hits": vec_hits, "x87_hits": x87_hits,
            "pass": bool(funcs) and scanned > 0 and not vec_hits and not x87_hits}


NEG_SRC = """__attribute__((noinline)) double scale(double a, double b) { return a * b + 1.5; }
int main(int argc, char **argv) { (void)argv; return scale(argc, 2.0) > 0.0 ? 0 : 1; }
"""
POS_SRC = """__attribute__((noinline)) unsigned mix(unsigned a, unsigned b) { return (a * 2654435761u) ^ (b + 0x9e3779b9u); }
int main(int argc, char **argv) { (void)argv; return (int)(mix((unsigned)argc, 7u) & 1u); }
"""


X87_SRC = """static volatile double xa = 1.5, xb = 2.0, xc;
__attribute__((noinline)) void mulx(void) { xc = xa * xb; }
int main(int argc, char **argv) { (void)argv; mulx(); return argc > 99 ? 1 : 0; }
"""


def self_test() -> int:
    """Negative control (SSE double math must FAIL) and positive control (integer-only,
    C2 profile, freestanding link must PASS). Both are built here, not stored."""
    import tempfile
    with tempfile.TemporaryDirectory() as td:
        d = Path(td)
        (d / "neg.c").write_text(NEG_SRC)
        (d / "pos.c").write_text(POS_SRC)
        (d / "x87.c").write_text(X87_SRC)
        subprocess.run(["gcc", "-O2", "-msse2", "-mfpmath=sse", "-static", "-o", str(d / "neg.elf"),
                        str(d / "neg.c")], check=True)
        subprocess.run(["gcc", "-O2", "-mgeneral-regs-only", "-mno-sse", "-mno-sse2", "-mno-mmx",
                        "-mno-80387", "-msoft-float", "-ffreestanding", "-fno-builtin", "-fno-pic",
                        "-fno-pie", "-nostdlib", "-static", "-no-pie", "-Wl,-e,main", "-o",
                        str(d / "pos.elf"), str(d / "pos.c")], check=True)
        # x87 negative control: -mfpmath=387 double math with the vector and x87 gate
        # flags off. Clang (Zig) emits x87 for this profile too (C2.8 finding).
        subprocess.run(["gcc", "-O2", "-mfpmath=387", "-mno-sse", "-mno-sse2", "-m80387",
                        "-ffreestanding", "-fno-builtin", "-fno-pic", "-fno-pie", "-nostdlib",
                        "-static", "-no-pie", "-Wl,-e,main", "-o", str(d / "x87.elf"),
                        str(d / "x87.c")], check=True)
        neg = scan(d / "neg.elf")
        pos = scan(d / "pos.elf")
        x87 = scan(d / "x87.elf")
    ok = ((not neg["pass"]) and bool(neg["vector_operand_hits"]) and pos["pass"]
          and (not x87["pass"]) and bool(x87["x87_hits"]))
    print(json.dumps({"negative_control_fails_as_expected": not neg["pass"],
                      "negative_vector_hits": len(neg["vector_operand_hits"]),
                      "x87_control_fails_as_expected": not x87["pass"],
                      "x87_control_hits": len(x87["x87_hits"]),
                      "positive_control_passes_as_expected": pos["pass"],
                      "positive_instructions": pos["instructions_scanned"]}))
    print("ISA-AUDIT-SELFTEST", "PASS" if ok else "FAIL")
    return 0 if ok else 1


def main(argv: list[str]) -> int:
    if argv == ["--self-test"]:
        return self_test()
    if not argv:
        print("usage: c2_isa_audit.py ELF...", file=sys.stderr)
        return 2
    results = [scan(Path(a)) for a in argv]
    for r in results:
        print(json.dumps({k: v for k, v in r.items() if k not in ("vector_operand_hits", "x87_hits")}))
        for h in (r["vector_operand_hits"] + r["x87_hits"])[:20]:
            print("  HIT", h)
    ok = all(r["pass"] for r in results)
    print("ISA-AUDIT", "PASS" if ok else "FAIL", f"elfs={len(results)}")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
