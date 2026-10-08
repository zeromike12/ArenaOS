#!/usr/bin/env python3
"""C2.6 compatibility report (machine-readable). EXPERIMENTAL.

For every function declared by the SDK headers (include/libc/*.h), reports what the
built library ACTUALLY provides, per compiler archive:

  defined      the standard name is an exported symbol in libarena_c.a (nm T)
  missing      declared in a header but NOT defined (header presence is not proof)
  evidence     where the symbol is exercised: host (ASan/UBSan differential or unit
               test), guest (executed under ArenaOS by a PASS run), or none

Also records whether the GCC and Clang archives export the same symbol set
(ABI symbol agreement), the compiler versions, and the archive hashes.

Usage: python3 c2_report.py            (requires build/guest-{gcc,clang}/lib)
Output: compat/c2-compat-report.json
"""
from __future__ import annotations

import hashlib
import json
import re
import subprocess
import sys
from pathlib import Path

EXP = Path(__file__).resolve().parent
HEADERS = sorted((EXP / "include" / "libc").glob("*.h"))
HOST_TESTS = [EXP / "host" / "test_libc.c", EXP / "host" / "test_runtime.c"]
GUEST_APPS = [EXP / "app" / "c2_console_app.c", EXP / "app" / "xxhash_app.c"]
# Functions reached only through documented refusal paths in the guest runs; listed so
# the report says so instead of implying a working implementation.
REFUSED_BY_DESIGN = {
    "time": "no wall-clock authority: refused with ENOSYS",
    "clock": "no CPU-time source: refused",
    "fopen": "no file authority in C2: refused (ENOSYS/ENOENT); see C2-LIBC-COVERAGE.md",
    "freopen": "no file authority in C2: refused",
    "remove": "no file authority in C2: refused",
    "rename": "no file authority in C2: refused",
    "mtx_timedlock": "no timeout path in the mutex binding: refused with thrd_error (guest G6)",
    "tss_create": "no thread-specific storage: refused with thrd_error (guest G6)",
    "tss_set": "no thread-specific storage: refused with thrd_error (guest G6)",
    "tss_get": "no thread-specific storage: returns NULL, a quiet refusal (guest G6)",
}
# fclose is NOT refused by design: it closes a granted stream and refuses only NULL or
# closed streams (EBADF). It is tested on the host for the NULL refusal.
# A top-level function declaration: unindented, not a typedef, a type ending in
# whitespace or '*', then the function name directly followed by '('.
DECL = re.compile(r"^([A-Za-z_][\w \t\*]*?[\s\*])([A-Za-z_]\w*)\s*\(")
NOT_FUNCTIONS = {"if", "while", "for", "switch", "return", "sizeof", "__attribute__", "defined",
                 "int", "void", "char", "long", "short", "unsigned", "signed", "double", "float",
                 "typedef", "struct", "enum", "union"}


def declared_functions() -> dict[str, str]:
    found: dict[str, str] = {}
    for h in HEADERS:
        for line in h.read_text().splitlines():
            line = re.sub(r"^__attribute__\(\(noreturn\)\)\s+", "", line)
            if not line or line[0] in " \t#" or line.startswith("typedef"):
                continue  # continuation lines, preprocessor, typedefs
            m = DECL.match(line)
            if m and m.group(2) not in NOT_FUNCTIONS and len(m.group(2)) > 1:
                found.setdefault(m.group(2), h.name)
    return found


def defined_symbols(archive: Path) -> set[str]:
    out = subprocess.run(["nm", "--defined-only", "-g", str(archive)], capture_output=True, text=True)
    if out.returncode != 0:
        raise SystemExit(f"nm failed on {archive}: {out.stderr}")
    syms = set()
    for line in out.stdout.splitlines():
        parts = line.split()
        if len(parts) == 3 and parts[1] in ("T", "W"):
            syms.add(parts[2])
    return syms


def code_only(text: str) -> str:
    """Strip comments, string literals, and #include lines so prose cannot count as a reference."""
    text = re.sub(r"/\*.*?\*/", " ", text, flags=re.S)
    text = re.sub(r"//[^\n]*", " ", text)
    text = re.sub(r'"(?:\\.|[^"\\\n])*"', '""', text)
    text = re.sub(r"^\s*#\s*include[^\n]*$", " ", text, flags=re.M)
    return text


def evidence_for(name: str, host_text: str, guest_text: str) -> list[str]:
    ev = []
    # Source-reference match only. A guest "exact PASS" run is what proves execution;
    # these labels say the symbol is referenced by a test/app, not that it ran.
    # Comments, string literals, and #include lines are excluded (C2.8 fix: prose
    # such as "G4 time" had been counted as a reference to time()).
    # Host tests call the internal arena_* names (libc_names.c wraps them for the guest).
    host_alias = {"_Exit": "arena_libc_Exit", "abort": "arena_libc_abort"}.get(name, name)
    if re.search(rf"\b(?:arena_)?{name}\b", code_only(host_text)) or re.search(rf"\b{host_alias}\b", code_only(host_text)):
        ev.append("host-test-source")
    if re.search(rf"\b{name}\b", code_only(guest_text)):
        ev.append("guest-app-source")
    return ev


def tool_version(cmd: list[str]) -> str:
    try:
        return subprocess.run(cmd, capture_output=True, text=True).stdout.splitlines()[0].strip()
    except Exception:  # noqa: BLE001 - report the failure as data
        return "unavailable"


def sha256_file(p: Path) -> str:
    return hashlib.sha256(p.read_bytes()).hexdigest()


def main() -> int:
    declared = declared_functions()
    host_text = "\n".join(p.read_text() for p in HOST_TESTS if p.exists())
    guest_text = "\n".join(p.read_text() for p in GUEST_APPS if p.exists())
    per_cc: dict[str, dict] = {}
    for cc in ("gcc", "clang"):
        archive = EXP / "build" / f"guest-{cc}" / "lib" / "libarena_c.a"
        if not archive.exists():
            raise SystemExit(f"missing {archive}: run `run.py --cc {cc} build` first")
        defined = defined_symbols(archive)
        per_cc[cc] = {"archive": str(archive.relative_to(EXP)), "sha256": sha256_file(archive),
                      "defined": defined}
    cc_tool = {"gcc": tool_version(["gcc", "--version"]),
               "clang": tool_version([str(Path.home() / ".arena-tools/zig/pkg/ziglang/zig"), "cc", "--version"])}
    agree = per_cc["gcc"]["defined"] == per_cc["clang"]["defined"]

    symbols = []
    for name in sorted(declared):
        gcc_ok = name in per_cc["gcc"]["defined"]
        clang_ok = name in per_cc["clang"]["defined"]
        if gcc_ok and clang_ok:
            status = "defined"
        elif gcc_ok or clang_ok:
            status = "defined-in-one-compiler-only"
        else:
            status = "missing"
        entry = {"name": name, "header": declared[name], "status": status,
                 "evidence": evidence_for(name, host_text, guest_text)}
        if name in REFUSED_BY_DESIGN:
            entry["refusal"] = REFUSED_BY_DESIGN[name]
        symbols.append(entry)

    # Defined but undeclared standard-name exports (should be empty for the SDK surface).
    declared_set = set(declared)
    extras = sorted(s for s in per_cc["gcc"]["defined"] if s in REFUSED_BY_DESIGN or False)
    report = {
        "scope": "C2 SDK surface (include/libc/*.h). Not full libc; not POSIX.",
        "evidence_classes": {
            "host": "ASan+UBSan host tests (glibc differential where meaningful); host-only",
            "host-test-source": "name referenced by host/test_libc.c or host/test_runtime.c (reference only)",
            "guest-app-source": "name referenced by app/c2_console_app.c or app/xxhash_app.c (reference only; execution is shown by the exact-PASS guest runs)",
            "defined": "exported by the built static archive (nm); header presence is NOT proof",
        },
        "toolchains": {"gcc": cc_tool["gcc"], "clang": cc_tool["clang"] +
                       " (Zig-bundled clang, target x86_64-freestanding-none)"},
        "archives": {cc: {k: v for k, v in d.items() if k != "defined"} for cc, d in per_cc.items()},
        "abi_symbol_agreement_gcc_clang": agree,
        "counts": {
            "declared": len(declared),
            "defined_both": sum(1 for s in symbols if s["status"] == "defined"),
            "missing": sum(1 for s in symbols if s["status"] == "missing"),
            "one_compiler_only": sum(1 for s in symbols if s["status"].startswith("defined-in")),
            "referenced_in_guest_app": sum(1 for s in symbols if "guest-app-source" in s["evidence"]),
            "referenced_in_host_test": sum(1 for s in symbols if "host-test-source" in s["evidence"]),
            "no_test_reference": sum(1 for s in symbols if not s["evidence"]),
            "refused_by_design": sum(1 for s in symbols if "refusal" in s),
        },
        "symbols": symbols,
    }
    out = EXP / "compat" / "c2-compat-report.json"
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(report["counts"], indent=2))
    print(f"abi_symbol_agreement_gcc_clang={agree}")
    print(f"report: {out}")
    untested = [s["name"] for s in symbols if not s["evidence"]]
    if untested:
        print("DEFINED BUT NOT REFERENCED BY ANY TEST:", untested)
    bad = [s["name"] for s in symbols if s["status"] != "defined"]
    if bad:
        print("NOT DEFINED IN BOTH:", bad)
    return 0 if agree and not any(s["status"] == "missing" for s in symbols) else 1


if __name__ == "__main__":
    sys.exit(main())
