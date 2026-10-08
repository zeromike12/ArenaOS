#!/usr/bin/env python3
"""Reliable, disassembly-based CPU instruction architecture scanner for ArenaOS.

Scans FUNC symbol ranges of ELF binaries for floating-point and SIMD instructions:
  - SSE / SSE2 / SSE3 / SSSE3 / SSE4 instructions & %xmm operands
  - AVX / AVX2 / AVX-512 instructions & %ymm / %zmm operands
  - MMX instructions & %mm operands
  - Legacy 80387 x87 FPU mnemonics & %st(...) register stack operands

Ignores non-executable data, jump tables, and padding in .text by strictly
bounding scan ranges to [st_value, st_value + st_size) of defined FUNC symbols.

Usage:
  python3 tools/isa_audit.py <elf_path> [--strict-no-fp] [--json]
  python3 tools/isa_audit.py --self-test
"""

from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
import tempfile
from pathlib import Path

# Regular expressions matching vector and x87 registers
RE_XMM = re.compile(r"%xmm\d+\b")
RE_YMM = re.compile(r"%ymm\d+\b")
RE_ZMM = re.compile(r"%zmm\d+\b")
RE_MMX = re.compile(r"%mm\d+\b")
RE_ST = re.compile(r"%st(\(\d\))?(?![a-z0-9])")

# x87 FPU mnemonics
RE_X87_MNEMONIC = re.compile(
    r"^(f(add|sub|subr|mul|div|divr|ld|st|stp|xch|ild|ist|istp|nstcw|ldcw|nstsw|clex|"
    r"init|abs|chs|sqrt|sin|cos|rndint|comi|ucomi|nop|wait|ldenv|nenv|rstor|save|"
    r"prem|scale|xtract|ucom|com|tst|xam|ldz|ldpi|ldl2|ldl2e|ldln2|ldlg2|f2xm1|fyl2x|"
    r"incstp|decstp|ffree|fcmov|fucom|fcom|fbld|fbstp|fpatan|fptan|fyl2xp1|fprem1|fdecstp|"
    r"fincstp|fcmov)[a-z]*|fwait)$"
)

# SSE / SSE2 mnemonics that might operate purely on memory operands
RE_SSE_MNEMONIC = re.compile(
    r"^(mov[aslu]ps|mov[su]pd|movss|movsd|movhps|movlps|movhpd|movlpd|movntps|movntpd|"
    r"addss|addsd|addps|addpd|subss|subsd|subps|subpd|mulss|mulsd|mulps|mulpd|"
    r"divss|divsd|divps|divpd|sqrtss|sqrtsd|sqrtps|sqrtpd|"
    r"comiss|comisd|ucomiss|ucomisd|maxss|maxsd|maxps|maxpd|minss|minsd|minps|minpd|"
    r"andps|andpd|andnps|andnpd|orps|orpd|xorps|xorpd|"
    r"cvtss2sd|cvtsd2ss|cvtsi2ss|cvtsi2sd|cvttss2si|cvttsd2si|cvtss2si|cvtsd2si|"
    r"cvtdq2ps|cvtps2dq|cvttps2dq|cvtdq2pd|cvtpd2dq|cvttpd2dq|"
    r"shufps|shufpd|unpckhps|unpcklps|unpckhpd|unpcklpd|"
    r"ldmxcsr|stmxcsr|clflush|maskmovq|movntq)$"
)

# AVX mnemonics
RE_AVX_MNEMONIC = re.compile(r"^v[a-z0-9]+")


def extract_functions(elf_path: Path) -> list[tuple[int, int, str]]:
    cmd = ["readelf", "-sW", str(elf_path)]
    res = subprocess.run(cmd, capture_output=True, text=True, check=True)
    funcs = []
    for line in res.stdout.splitlines():
        parts = line.split()
        if len(parts) >= 8 and parts[3] == "FUNC" and parts[6] != "UND":
            try:
                addr = int(parts[1], 16)
                size = int(parts[2])
                name = parts[7]
            except ValueError:
                continue
            if size > 0:
                funcs.append((addr, size, name))
    return sorted(funcs, key=lambda x: x[0])


def scan_elf(elf_path: Path) -> dict:
    funcs = extract_functions(elf_path)
    if not funcs:
        return {
            "file": str(elf_path),
            "functions_count": 0,
            "instructions_scanned": 0,
            "xmm_hits": 0,
            "ymm_hits": 0,
            "zmm_hits": 0,
            "mmx_hits": 0,
            "x87_hits": 0,
            "categories": {},
            "function_breakdown": {},
            "all_clean": False,
            "error": "No FUNC symbols found",
        }

    cmd = ["objdump", "-d", "--no-show-raw-insn", str(elf_path)]
    dis_res = subprocess.run(cmd, capture_output=True, text=True, check=True)

    ranges = [(addr, addr + size, name) for addr, size, name in funcs]

    total_scanned = 0
    xmm_count = 0
    ymm_count = 0
    zmm_count = 0
    mmx_count = 0
    x87_count = 0
    sse_mnemonic_count = 0
    avx_mnemonic_count = 0

    hits_by_fn = {}
    details = []

    for line in dis_res.stdout.splitlines():
        m = re.match(r"^\s*([0-9a-f]+):\t(.*)$", line)
        if not m:
            continue
        addr = int(m.group(1), 16)
        body = m.group(2).strip()
        if not body:
            continue

        # Check if instruction is inside a function
        owner = None
        for lo, hi, name in ranges:
            if lo <= addr < hi:
                owner = name
                break
        if owner is None:
            continue

        total_scanned += 1
        tokens = body.split()
        mn = tokens[0]

        is_xmm = bool(RE_XMM.search(body))
        is_ymm = bool(RE_YMM.search(body))
        is_zmm = bool(RE_ZMM.search(body))
        is_mmx = bool(RE_MMX.search(body))
        is_st = bool(RE_ST.search(body))
        is_x87_mn = bool(RE_X87_MNEMONIC.match(mn))
        is_sse_mn = bool(RE_SSE_MNEMONIC.match(mn))
        is_avx_mn = bool(RE_AVX_MNEMONIC.match(mn))

        hit_cats = []
        if is_xmm or is_sse_mn:
            hit_cats.append("SSE")
            xmm_count += 1
        if is_ymm or is_zmm or is_avx_mn:
            hit_cats.append("AVX")
            if is_ymm: ymm_count += 1
            if is_zmm: zmm_count += 1
            if is_avx_mn: avx_mnemonic_count += 1
        if is_mmx:
            hit_cats.append("MMX")
            mmx_count += 1
        if is_st or is_x87_mn:
            hit_cats.append("x87")
            x87_count += 1

        if hit_cats:
            hits_by_fn[owner] = hits_by_fn.get(owner, 0) + 1
            if len(details) < 50:
                details.append({
                    "addr": hex(addr),
                    "fn": owner,
                    "categories": hit_cats,
                    "insn": body,
                })

    all_fp_simd_hits = xmm_count + ymm_count + zmm_count + mmx_count + x87_count + avx_mnemonic_count
    all_clean = (all_fp_simd_hits == 0) and (total_scanned > 0)

    return {
        "file": str(elf_path),
        "functions_count": len(funcs),
        "instructions_scanned": total_scanned,
        "xmm_sse_hits": xmm_count,
        "ymm_avx_hits": ymm_count,
        "zmm_avx512_hits": zmm_count,
        "mmx_hits": mmx_count,
        "x87_fpu_hits": x87_count,
        "total_prohibited_hits": all_fp_simd_hits,
        "all_clean": all_clean,
        "top_offending_functions": sorted(hits_by_fn.items(), key=lambda x: -x[1])[:15],
        "sample_hits": details[:10],
    }


def run_self_test() -> int:
    print("=== Running ISA Audit Self-Test ===")
    neg_c = """
    __attribute__((noinline)) double sse_math(double x, double y) {
        return x * y + 3.14159;
    }
    int main(int argc, char **argv) {
        (void)argv;
        return sse_math(argc, 2.0) > 0.0 ? 0 : 1;
    }
    """

    pos_c = """
    __attribute__((noinline)) unsigned int int_math(unsigned int a, unsigned int b) {
        return (a * 2654435761u) ^ (b + 0x9e3779b9u);
    }
    int main(int argc, char **argv) {
        (void)argv;
        return (int)(int_math((unsigned int)argc, 42u) & 1u);
    }
    """

    with tempfile.TemporaryDirectory() as td:
        neg_src = Path(td) / "neg.c"
        neg_elf = Path(td) / "neg.elf"
        pos_src = Path(td) / "pos.c"
        pos_elf = Path(td) / "pos.elf"

        neg_src.write_text(neg_c)
        pos_src.write_text(pos_c)

        # Build negative control (with SSE math)
        subprocess.run(["gcc", "-O2", "-msse2", "-mfpmath=sse", "-static", str(neg_src), "-o", str(neg_elf)], check=True)

        # Build positive control (strictly general-regs-only, no-SSE, no-x87)
        subprocess.run([
            "gcc", "-O2", "-mgeneral-regs-only", "-mno-sse", "-mno-sse2", "-mno-mmx",
            "-mno-80387", "-msoft-float", "-ffreestanding", "-fno-builtin", "-fno-pic",
            "-fno-pie", "-nostdlib", "-static", "-no-pie", "-Wl,-e,main",
            str(pos_src), "-o", str(pos_elf)
        ], check=True)

        neg_res = scan_elf(neg_elf)
        pos_res = scan_elf(pos_elf)

    neg_detected = (not neg_res["all_clean"]) and (neg_res["xmm_sse_hits"] > 0)
    pos_verified = pos_res["all_clean"] and (pos_res["instructions_scanned"] > 0)

    print(f"Negative control (SSE math present): {'DETECTED (PASS)' if neg_detected else 'MISSED (FAIL)'} [hits: {neg_res['total_prohibited_hits']}]")
    print(f"Positive control (Integer-only):     {'VERIFIED CLEAN (PASS)' if pos_verified else 'FALSE POSITIVE (FAIL)'} [insns: {pos_res['instructions_scanned']}]")

    if neg_detected and pos_verified:
        print("ISA AUDIT SELF-TEST: PASS")
        return 0
    else:
        print("ISA AUDIT SELF-TEST: FAIL")
        return 1


def main():
    parser = argparse.ArgumentParser(description="Deterministic ISA FP/SIMD instruction auditor for ArenaOS.")
    parser.add_argument("elf", nargs="?", type=Path, help="Path to ELF binary")
    parser.add_argument("--self-test", action="store_true", help="Execute positive and negative control self-tests")
    parser.add_argument("--strict-no-fp", action="store_true", help="Fail (exit 1) if any FP or SIMD instruction is found")
    parser.add_argument("--json", action="store_true", help="Emit report in JSON format")
    args = parser.parse_args()

    if args.self_test:
        sys.exit(run_self_test())

    if not args.elf:
        parser.print_help()
        sys.exit(2)

    if not args.elf.exists():
        print(f"Error: File '{args.elf}' not found.", file=sys.stderr)
        sys.exit(1)

    report = scan_elf(args.elf)

    if args.json:
        print(json.dumps(report, indent=2))
    else:
        print("=" * 72)
        print(f"ArenaOS CPU Instruction Architecture Audit: {report['file']}")
        print("=" * 72)
        print(f"Functions Audited:    {report['functions_count']}")
        print(f"Instructions Scanned: {report['instructions_scanned']}")
        print(f"SSE / %xmm Hits:      {report['xmm_sse_hits']}")
        print(f"AVX / %ymm/%zmm Hits: {report['ymm_avx_hits'] + report['zmm_avx512_hits']}")
        print(f"MMX / %mm Hits:       {report['mmx_hits']}")
        print(f"x87 FPU / %st Hits:   {report['x87_fpu_hits']}")
        print(f"Total Prohibited:     {report['total_prohibited_hits']}")
        print("-" * 72)
        if report["all_clean"]:
            print("ISA AUDIT VERDICT: CLEAN (No floating-point or SIMD instructions found)")
        else:
            print("ISA AUDIT VERDICT: PROHIBITED FP/SIMD INSTRUCTIONS DETECTED")
            print("Top Offending Functions:")
            for fn, count in report["top_offending_functions"]:
                print(f"  {fn:<36} : {count:4d} hits")
            print("Sample Instruction Hits:")
            for sample in report["sample_hits"][:5]:
                print(f"  {sample['addr']} [{sample['fn']}]: {sample['insn']} ({','.join(sample['categories'])})")
        print("=" * 72)

    if args.strict_no_fp and not report["all_clean"]:
        sys.exit(1)
    sys.exit(0)


if __name__ == "__main__":
    main()
