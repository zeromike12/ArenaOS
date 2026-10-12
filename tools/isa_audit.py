#!/usr/bin/env python3
"""Reliable, whole-executable CPU instruction architecture scanner for ArenaOS.

Audits EVERY executable instruction in an ELF binary for floating-point and SIMD:
  - SSE / SSE2 / SSE3 / SSSE3 / SSE4 instructions & %xmm operands
  - AVX / AVX2 / AVX-512 instructions & %ymm / %zmm operands
  - MMX instructions & %mm operands
  - Legacy 80387 x87 FPU mnemonics & %st(...) register stack operands

Coverage Guarantee (Milestone G2.2.2 Whole-Executable Invariant):
  1. Complete Executable Code Coverage:
     Audits ALL executable instructions across all executable segments (PT_LOAD RX)
     and executable sections (.text / AX), regardless of whether symbol table metadata
     or FUNC symbols exist.
  2. Function Symbol Attribution:
     Function symbols are used solely to associate instructions with names for reporting.
     The absence of a FUNC symbol NEVER exempts an instruction from being audited.
  3. Strict Fail-Closed Policy:
     - Missing or empty disassembly output fails the audit immediately.
     - Zero scanned instructions is treated as an error (never reported as clean).
     - Instruction presence represents an architectural hazard regardless of reachability.
  4. Handoff Contract:
     - Distinguishes static instruction presence from proven runtime execution.
     - Separates CPU instruction enablement from cross-thread state preservation.

Usage:
  python3 tools/isa_audit.py <elf_path> [--strict] [--json]
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
    """Extract defined FUNC symbols for reporting attribution only."""
    cmd = ["readelf", "-sW", str(elf_path)]
    try:
        res = subprocess.run(cmd, capture_output=True, text=True, check=True)
    except subprocess.CalledProcessError:
        return []
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


def extract_executable_ranges(elf_path: Path) -> list[tuple[int, int, str]]:
    """Extract all executable address ranges from ELF sections and program headers."""
    ranges = []

    # 1. Inspect section headers for SHF_EXECINSTR (AX)
    cmd = ["readelf", "-SW", str(elf_path)]
    try:
        res = subprocess.run(cmd, capture_output=True, text=True, check=True)
        for line in res.stdout.splitlines():
            m = re.search(r"\[\s*\d+\]\s+(\S+)\s+\S+\s+([0-9a-fA-F]+)\s+[0-9a-fA-F]+\s+([0-9a-fA-F]+)\s+\S+\s+(\S+)", line)
            if m:
                sec_name = m.group(1)
                addr = int(m.group(2), 16)
                size = int(m.group(3), 16)
                flags = m.group(4)
                if "X" in flags and size > 0:
                    ranges.append((addr, addr + size, sec_name))
    except subprocess.CalledProcessError:
        pass

    # 2. Inspect program headers for PT_LOAD executable segments
    cmd = ["readelf", "-lW", str(elf_path)]
    try:
        res = subprocess.run(cmd, capture_output=True, text=True, check=True)
        for line in res.stdout.splitlines():
            if "LOAD" in line and " E" in line:
                parts = line.split()
                try:
                    idx = parts.index("LOAD")
                    va = int(parts[idx + 2], 16)
                    memsz = int(parts[idx + 5], 16)
                    if memsz > 0:
                        ranges.append((va, va + memsz, f"PT_LOAD_RX_0x{va:x}"))
                except (ValueError, IndexError):
                    continue
    except subprocess.CalledProcessError:
        pass

    if not ranges:
        # Default fallback: allow all disassembled addresses
        return [(0, 0xFFFFFFFFFFFFFFFF, "all_executable")]

    return sorted(ranges, key=lambda x: x[0])


def scan_elf(elf_path: Path) -> dict:
    if not elf_path.is_file():
        return {
            "file": str(elf_path),
            "error": "File not found",
            "all_clean": False,
            "total_prohibited_hits": 999999,
        }

    funcs = extract_functions(elf_path)
    exec_ranges = extract_executable_ranges(elf_path)

    cmd = ["objdump", "-d", "--no-show-raw-insn", str(elf_path)]
    try:
        dis_res = subprocess.run(cmd, capture_output=True, text=True, check=True)
    except subprocess.CalledProcessError as e:
        return {
            "file": str(elf_path),
            "error": f"objdump failed: {e}",
            "all_clean": False,
            "total_prohibited_hits": 999999,
        }

    lines = dis_res.stdout.splitlines()
    if not lines:
        return {
            "file": str(elf_path),
            "error": "Empty disassembly output",
            "all_clean": False,
            "total_prohibited_hits": 999999,
        }

    func_ranges = [(addr, addr + size, name) for addr, size, name in funcs]

    total_scanned = 0
    non_func_scanned = 0
    xmm_count = 0
    ymm_count = 0
    zmm_count = 0
    mmx_count = 0
    x87_count = 0
    sse_mnemonic_count = 0
    avx_mnemonic_count = 0

    hits_by_fn = {}
    details = []

    for line in lines:
        m = re.match(r"^\s*([0-9a-f]+):\t(.*)$", line)
        if not m:
            continue
        addr = int(m.group(1), 16)
        body = m.group(2).strip()
        if not body:
            continue

        # Invariant: Every instruction output by objdump -d in an executable region is audited
        in_exec = any(lo <= addr < hi for lo, hi, _ in exec_ranges)
        if not in_exec:
            continue

        # Symbol attribution: associate with function if known, else attribute to non-func address
        owner = None
        for lo, hi, name in func_ranges:
            if lo <= addr < hi:
                owner = name
                break
        if owner is None:
            owner = f"<non-func>@0x{addr:x}"
            non_func_scanned += 1

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
    # Fail-closed: Must have scanned at least one instruction, and zero hits
    all_clean = (all_fp_simd_hits == 0) and (total_scanned > 0)

    return {
        "file": str(elf_path),
        "functions_count": len(funcs),
        "instructions_scanned": total_scanned,
        "non_func_instructions_scanned": non_func_scanned,
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
    print("=== Running Comprehensive Whole-Executable ISA Audit Self-Test ===")
    
    # 1. Negative control: SSE math in symbolized C code
    neg_sse_c = """
    __attribute__((noinline)) double sse_math(double x, double y) {
        return x * y + 3.14159;
    }
    int main(int argc, char **argv) {
        (void)argv;
        return sse_math(argc, 2.0) > 0.0 ? 0 : 1;
    }
    """

    # 2. Negative control: x87 FPU math in symbolized C code
    neg_x87_c = """
    int main(int argc, char **argv) {
        (void)argv;
        volatile double a = (double)argc;
        volatile double b = 2.71828;
        volatile double c = a * b;
        return (int)c;
    }
    """

    # 3. Negative control: Prohibited instruction OUTSIDE any FUNC symbol (raw assembly label)
    neg_unsymbolized_s = """
    .global _start
    .text
    _start:
        xor %eax, %eax
        call raw_simd_helper
        mov %eax, %edi
        mov $60, %eax
        syscall

    /* Intentionally NOT marked as .type ..., @function */
    raw_simd_helper:
        movups %xmm0, %xmm1
        ret
    """

    # 4. Positive control: Pure integer freestanding binary
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
        neg_sse_src = Path(td) / "neg_sse.c"
        neg_sse_elf = Path(td) / "neg_sse.elf"
        neg_x87_src = Path(td) / "neg_x87.c"
        neg_x87_elf = Path(td) / "neg_x87.elf"
        neg_unsym_src = Path(td) / "neg_unsym.s"
        neg_unsym_elf = Path(td) / "neg_unsym.elf"
        neg_stripped_elf = Path(td) / "neg_stripped.elf"
        pos_src = Path(td) / "pos.c"
        pos_elf = Path(td) / "pos.elf"
        pos_stripped_elf = Path(td) / "pos_stripped.elf"

        neg_sse_src.write_text(neg_sse_c)
        neg_x87_src.write_text(neg_x87_c)
        neg_unsym_src.write_text(neg_unsymbolized_s)
        pos_src.write_text(pos_c)

        # 1. Build SSE negative control (compiled with SSE enabled)
        subprocess.run(["gcc", "-O2", "-msse2", "-mfpmath=sse", "-static", str(neg_sse_src), "-o", str(neg_sse_elf)], check=True)

        # 2. Build x87 negative control (compiled with -mno-sse, forcing GCC to emit 80387 x87 code)
        subprocess.run(["gcc", "-O2", "-mno-sse", "-mfpmath=387", "-static", str(neg_x87_src), "-o", str(neg_x87_elf)], check=True)

        # 3. Build unsymbolized negative control (raw assembly without FUNC symbols containing movups %xmm0, %xmm1)
        subprocess.run(["gcc", "-nostdlib", "-static", "-no-pie", str(neg_unsym_src), "-o", str(neg_unsym_elf)], check=True)

        # 4. Build stripped negative control
        subprocess.run(["strip", "--strip-all", "-o", str(neg_stripped_elf), str(neg_unsym_elf)], check=True)

        # 5. Build positive control (strictly general-regs-only, no-SSE, no-x87)
        subprocess.run([
            "gcc", "-O2", "-mgeneral-regs-only", "-mno-sse", "-mno-sse2", "-mno-mmx",
            "-mno-80387", "-msoft-float", "-ffreestanding", "-fno-builtin", "-fno-pic",
            "-fno-pie", "-nostdlib", "-static", "-no-pie", "-Wl,-e,main",
            str(pos_src), "-o", str(pos_elf)
        ], check=True)

        # 6. Build stripped positive control
        subprocess.run(["strip", "--strip-all", "-o", str(pos_stripped_elf), str(pos_elf)], check=True)

        neg_sse_res = scan_elf(neg_sse_elf)
        neg_x87_res = scan_elf(neg_x87_elf)
        neg_unsym_res = scan_elf(neg_unsym_elf)
        neg_stripped_res = scan_elf(neg_stripped_elf)
        pos_res = scan_elf(pos_elf)
        pos_stripped_res = scan_elf(pos_stripped_elf)

    neg_sse_ok = (not neg_sse_res["all_clean"]) and (neg_sse_res["xmm_sse_hits"] > 0)
    neg_x87_ok = (not neg_x87_res["all_clean"]) and (neg_x87_res["x87_fpu_hits"] > 0)
    neg_unsym_ok = (not neg_unsym_res["all_clean"]) and (neg_unsym_res["xmm_sse_hits"] > 0)
    neg_stripped_ok = (not neg_stripped_res["all_clean"]) and (neg_stripped_res["xmm_sse_hits"] > 0)
    pos_verified = pos_res["all_clean"] and (pos_res["instructions_scanned"] > 0)
    pos_stripped_verified = pos_stripped_res["all_clean"] and (pos_stripped_res["instructions_scanned"] > 0)

    print(f"Control 1 (SSE Math Symbolized):     {'DETECTED (PASS)' if neg_sse_ok else 'MISSED (FAIL)'} [hits: {neg_sse_res['total_prohibited_hits']}]")
    print(f"Control 2 (x87 FPU Math Symbolized): {'DETECTED (PASS)' if neg_x87_ok else 'MISSED (FAIL)'} [x87 hits: {neg_x87_res['x87_fpu_hits']}]")
    print(f"Control 3 (SIMD Outside FUNC Range): {'DETECTED (PASS)' if neg_unsym_ok else 'MISSED (FAIL)'} [hits: {neg_unsym_res['total_prohibited_hits']}]")
    print(f"Control 4 (SIMD in Stripped Binary): {'DETECTED (PASS)' if neg_stripped_ok else 'MISSED (FAIL)'} [hits: {neg_stripped_res['total_prohibited_hits']}]")
    print(f"Control 5 (Integer-Only Clean):      {'VERIFIED CLEAN (PASS)' if pos_verified else 'FALSE POSITIVE (FAIL)'} [insns: {pos_res['instructions_scanned']}]")
    print(f"Control 6 (Stripped Integer Clean):  {'VERIFIED CLEAN (PASS)' if pos_stripped_verified else 'FALSE POSITIVE (FAIL)'} [insns: {pos_stripped_res['instructions_scanned']}]")

    all_passed = (neg_sse_ok and neg_x87_ok and neg_unsym_ok and neg_stripped_ok and pos_verified and pos_stripped_verified)

    if all_passed:
        print("WHOLE-EXECUTABLE ISA AUDIT: ALL CONTROLS PASSED!")
        return 0
    else:
        print("WHOLE-EXECUTABLE ISA AUDIT: CONTROL CHECK FAILED!")
        return 1


def main():
    parser = argparse.ArgumentParser(description="Deterministic whole-executable ISA FP/SIMD instruction auditor for ArenaOS.")
    parser.add_argument("elf", nargs="?", type=Path, help="Path to ELF binary")
    parser.add_argument("--self-test", action="store_true", help="Execute comprehensive positive and negative control self-tests")
    parser.add_argument("--strict", "--strict-no-fp", dest="strict", action="store_true", help="Fail closed (exit 1) if any FP or SIMD instruction is found")
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
    if "error" in report:
        print(f"ISA AUDIT ERROR: {report['error']}", file=sys.stderr)
        sys.exit(1)

    if args.json:
        print(json.dumps(report, indent=2))
    else:
        print("=" * 72)
        print(f"ArenaOS CPU Instruction Architecture Audit: {report['file']}")
        print("=" * 72)
        print(f"Functions Audited:    {report['functions_count']}")
        print(f"Instructions Scanned: {report['instructions_scanned']} (non-FUNC: {report.get('non_func_instructions_scanned', 0)})")
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
            print("Top Offending Functions / Regions:")
            for fn, count in report["top_offending_functions"]:
                print(f"  {fn:<36} : {count:4d} hits")
            print("Sample Instruction Hits:")
            for sample in report["sample_hits"][:5]:
                print(f"  {sample['addr']} [{sample['fn']}]: {sample['insn']} ({','.join(sample['categories'])})")
        print("=" * 72)

    if args.strict and not report["all_clean"]:
        sys.exit(1)
    sys.exit(0)


if __name__ == "__main__":
    main()
