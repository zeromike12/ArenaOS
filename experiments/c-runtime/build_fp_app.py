#!/usr/bin/env python3
"""Phase 16: build and audit the FP/SSE2 integration app (ADR-0113 profile).

Separate from run.py by design: the canonical runtime/apps keep their
integer-only flags and mandatory no-FP audit (c2_isa_audit.py). This builder
compiles the same runtime sources + app/c_fp_app.c + app/fp_probe.S with the
baseline ring-3 FP profile and applies an instruction-aware positive audit:

  * SSE/SSE2 instructions REQUIRED in the FP app object (else the arithmetic
    proof would be vacuous), x87 instructions REQUIRED (F1/F3 claim them);
  * AVX-family instructions FORBIDDEN everywhere: detected per-instruction
    from the disassembly -- any mnemonic beginning with 'v' that is not in a
    small non-VEX allowlist (verr/verw/vmlaunch-kin do not occur; the list is
    explicit), any ymm/zmm register operand, and any mask-register (k0-k7)
    operand. Substring scanning of the whole listing is not used.
  * no undefined symbols.

Controls (`--self-test`, also run by default): a -mavx2 helper MUST fail the
audit; an integer-only helper MUST fail (no SSE2); an SSE2-only helper MUST
fail (no x87); the host comparator test (host/test_fp_image_check.c) MUST
pass all 7 corruption cases. A mutant application (probe flips one captured
byte, -DFP_PROBE_MUTATE) is built into build/fp-mutant-<cc>/ as an isolated
guest negative control; it is never a qualification artifact.

Usage: python3 build_fp_app.py [--cc gcc|clang|both]
"""
from __future__ import annotations

import argparse
import re
import subprocess
import sys
from pathlib import Path

EXP = Path(__file__).resolve().parent
sys.path.insert(0, str(EXP))
import run as c2run

INTEGER_ONLY = {
    "-mgeneral-regs-only", "-msoft-float", "-mno-sse", "-mno-sse2",
    "-mno-mmx", "-mno-80387",
}
SSE_MNEMONICS = (
    "movsd", "addsd", "subsd", "mulsd", "divsd", "movss", "addss", "subss",
    "mulss", "divss", "movaps", "movups", "movapd", "movupd", "ucomisd",
    "ucomiss", "cvtsd2ss", "cvtss2sd", "pxor", "movdqa", "movdqu",
)
X87_PREFIXES = ("fld", "fst", "fadd", "fsub", "fmul", "fdiv", "fcom", "fucom",
                "fxch", "fninit", "finit", "fldcw", "fnstcw", "fnstsw")
NON_VEX_V = {"verr", "verw", "vldmxcsr_is_vex_never_allow"}  # only verr/verw
INSN_RE = re.compile(r"^\s*[0-9a-f]+:\s+(?:[0-9a-f]{2} )+\s*([a-z0-9.]+)\s*(.*)$")


def fp_flags(cc: str, extra: list[str] | None = None) -> list[str]:
    flags = [f for f in c2run.common_flags(EXP) if f not in INTEGER_ONLY]
    flags += ["-msse2", "-mno-avx", "-mno-avx2", "-mno-avx512f"]
    if cc == "gcc":
        flags += ["-march=x86-64"]  # zig cc mis-parses this; its x86_64
        # freestanding target already defaults to the baseline CPU + SSE2.
    return flags + (extra or [])


def build(cc: str, out_name: str, extra: list[str] | None = None) -> tuple[Path, Path]:
    out = EXP / "build" / out_name
    obj = out / "obj"
    obj.mkdir(parents=True, exist_ok=True)
    compiler = c2run.COMPILERS[cc]
    flags = fp_flags(cc, extra)
    objects = []
    for src in c2run.RUNTIME_SRC:
        o = obj / (Path(src).stem + ".o")
        c2run.run(compiler + flags + ["-c", str(EXP / src), "-o", str(o)])
        objects.append(o)
    probe_o = obj / "fp_probe.o"
    c2run.run(compiler + flags + ["-c", str(EXP / "app/fp_probe.S"), "-o", str(probe_o)])
    app_o = obj / "c_fp_app.o"
    c2run.run(compiler + flags + ["-c", str(EXP / "app/c_fp_app.c"), "-o", str(app_o)])
    elf = out / f"c-fp-{cc}.elf"
    crt0 = next(o for o in objects if o.name == "crt0.o")
    # Archive the runtime exactly like run.py: on-demand member extraction is
    # what keeps legacy_debug.o (SYS_DEBUG_WRITE) out of ordinary images.
    lib = out / "libarena_c_fp.a"
    if lib.exists():
        lib.unlink()
    c2run.run(c2run.ar_cmd(cc) + ["rcs", str(lib)]
              + [str(o) for o in objects if o.name != "crt0.o"])
    c2run.run(compiler + flags + c2run.LINK_FLAGS
              + ["-Wl,-T," + str(EXP / "link/arena-user.ld"), str(crt0), str(app_o),
                 str(probe_o), str(lib), "-o", str(elf)])
    return elf, app_o


def instructions(path: Path):
    dis = subprocess.run(["objdump", "-d", str(path)],
                         capture_output=True, text=True, check=True).stdout
    for line in dis.splitlines():
        m = INSN_RE.match(line)
        if m:
            yield m.group(1), m.group(2)


def audit_obj(path: Path) -> dict:
    sse = x87 = avx = 0
    avx_examples = []
    for mnemonic, operands in instructions(path):
        base = mnemonic.rstrip("bwlq") if mnemonic.startswith("mov") else mnemonic
        if mnemonic in SSE_MNEMONICS or base in SSE_MNEMONICS:
            sse += 1
        if any(mnemonic.startswith(p) for p in X87_PREFIXES):
            x87 += 1
        is_avx = (
            (mnemonic.startswith("v") and len(mnemonic) > 1
             and mnemonic not in NON_VEX_V)
            or "ymm" in operands or "zmm" in operands
            or re.search(r"\bk[0-7]\b", operands) is not None
        )
        if is_avx:
            avx += 1
            if len(avx_examples) < 3:
                avx_examples.append(f"{mnemonic} {operands}".strip())
    return {"sse": sse, "x87": x87, "avx": avx, "avx_examples": avx_examples}


def audit(elf: Path, app_o: Path) -> None:
    a_elf, a_app = audit_obj(elf), audit_obj(app_o)
    undef = subprocess.run(["nm", "-u", str(elf)], capture_output=True, text=True).stdout.strip()
    print(f"{elf.name}: elf={a_elf} app={a_app} undefined=[{undef}]")
    check = [
        (a_elf["avx"] == 0, f"AVX-family instructions present: {a_elf['avx_examples']}"),
        (a_app["avx"] == 0, f"AVX in app object: {a_app['avx_examples']}"),
        (a_app["sse"] > 0, "no SSE/SSE2 instructions in app object (proof vacuous)"),
        (a_app["x87"] > 0, "no x87 instructions in app object (F1 claims x87)"),
        (a_elf["sse"] > 0 and a_elf["x87"] > 0, "linked ELF lost SSE2/x87 code"),
        (not undef, f"undefined symbols: {undef}"),
    ]
    for ok, msg in check:
        if not ok:
            raise SystemExit(f"FP-AUDIT FAIL {elf.name}: {msg}")
    print(f"FP-AUDIT PASS {elf.name}")


def self_test() -> None:
    tmp = EXP / "build" / "fp-selftest"
    tmp.mkdir(parents=True, exist_ok=True)
    cases = {
        "avx.c": ("__attribute__((target(\"avx2\"))) double f(double a,double b){return a*b;}",
                  ["-mavx2"], "AVX helper must be flagged"),
        "int.c": ("int f(int a,int b){return a*b;}", ["-mno-sse", "-msoft-float"],
                  "integer helper must lack SSE2"),
        "sse.c": ("double f(double a,double b){return a*b;}", ["-msse2", "-mno-80387"],
                  "SSE2-only helper must lack x87"),
    }
    for name, (code, flags, why) in cases.items():
        src, obj = tmp / name, tmp / (name + ".o")
        src.write_text(code)
        subprocess.run(["gcc", "-O2", "-c", str(src), "-o", str(obj)] + flags, check=True)
        a = audit_obj(obj)
        print(f"self-test {name}: {a}")
        if name == "avx.c" and a["avx"] == 0:
            raise SystemExit(f"FP-AUDIT SELF-TEST FAIL: {why}")
        if name == "int.c" and a["sse"] != 0:
            raise SystemExit(f"FP-AUDIT SELF-TEST FAIL: {why}")
        if name == "sse.c" and a["x87"] != 0:
            raise SystemExit(f"FP-AUDIT SELF-TEST FAIL: {why}")
    host = tmp / "test_fp_image_check"
    subprocess.run(["gcc", "-O1", "-Wall", "-Wextra", "-Werror",
                    "-fsanitize=address,undefined", "-fno-sanitize-recover=all",
                    str(EXP / "host/test_fp_image_check.c"), "-o", str(host)], check=True)
    out = subprocess.run([str(host)], capture_output=True, text=True)
    print(out.stdout.strip())
    if out.returncode != 0:
        raise SystemExit("FP-AUDIT SELF-TEST FAIL: comparator host control")
    # Execute the probe's no-syscall control path on the host CPU (all
    # user-mode instructions): real hardware validates the FXSAVE layout
    # expectations; the mutant build of the same path must be rejected.
    for name, extra, want_rc in (("probe_host", [], 0),
                                 ("probe_host_mutant", ["-DFP_PROBE_MUTATE"], 1)):
        exe = tmp / name
        subprocess.run(["gcc", "-O1", "-Wall", "-Wextra", "-Werror"] + extra
                       + [str(EXP / "host/test_fp_probe_host.c"),
                          str(EXP / "app/fp_probe.S"), "-o", str(exe)],
                       check=True, capture_output=True)
        out = subprocess.run([str(exe)], capture_output=True, text=True)
        print(f"{name}: {out.stdout.strip()} rc={out.returncode}")
        if out.returncode != want_rc:
            raise SystemExit(f"FP-AUDIT SELF-TEST FAIL: {name} rc {out.returncode} != {want_rc}")
    print("FP-AUDIT SELF-TEST PASS")


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--cc", choices=["gcc", "clang", "both"], default="both")
    args = ap.parse_args()
    self_test()
    for cc in (["gcc", "clang"] if args.cc == "both" else [args.cc]):
        elf, app_o = build(cc, f"fp-{cc}")
        audit(elf, app_o)
        mutant, m_app = build(cc, f"fp-mutant-{cc}", ["-DFP_PROBE_MUTATE"])
        a = audit_obj(mutant)
        if a["avx"]:
            raise SystemExit(f"FP-AUDIT FAIL mutant {mutant.name}: AVX present")
        print(f"MUTANT BUILT (isolated negative control): {mutant}")


if __name__ == "__main__":
    main()
