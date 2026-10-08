#!/usr/bin/env python3
"""ArenaOS C runtime prototype - build, audit, host tests, guest evidence.

EXPERIMENTAL. Nothing here is part of the production OS image. See
docs/compat/ for the audit, toolchain study, and roadmap this supports.

Subcommands
  build   compile the runtime + crt_probe with clang and/or gcc (guest ELF)
  audit   check each guest ELF against the kernel loader's rules and the
          no-SSE/x87 execution contract
  abi     cross-check the C syscall numbers against userspace/abi.rs
  host    compile and run the host-only test suite (ASan + UBSan)
  guest   apply the boot-probe patch to a SCRATCH copy of the tree, build the
          kernel image, boot it in QEMU, and extract the probe's serial markers
  all     everything above (guest uses clang's ELF by default)

Evidence labels used in the report:
  HOST-ONLY    host test binary (mmap backend, glibc differential checks)
  GUEST        executed inside ArenaOS under QEMU; serial markers captured
  STATIC       checked from the ELF/disassembly without executing it
"""

import argparse
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

EXP = Path(__file__).resolve().parent
REPO = EXP.parent.parent
BUILD = EXP / "build"

RUNTIME_SRC = ["src/crt0.c", "src/rt.c", "src/alloc.c", "src/string.c", "src/stdio.c", "src/vm.c"]
GUEST_APPS = {"crt-probe": ["app/crt_probe.c"]}

ZIG = os.environ.get("ARENA_ZIG", "/opt/zig-clang/pkg/ziglang/zig")

def common_flags(root):
    return [
    "-std=gnu11", "-O2", "-g0", "-Wall", "-Wextra", "-Werror",
    "-ffreestanding", "-fno-builtin",
    "-fno-stack-protector", "-fcf-protection=none",
    "-fno-pic", "-fno-pie", "-fno-asynchronous-unwind-tables", "-fno-unwind-tables",
    "-fno-exceptions", "-fno-omit-frame-pointer",
    # Execution contract (docs/compat/01-architecture-audit.md): the kernel
    # saves no FPU/SSE state and never sets CR4.OSFXSR, so guest code must
    # not emit x87/MMX/SSE instructions at all.
    "-mgeneral-regs-only", "-msoft-float",
    "-mno-sse", "-mno-sse2", "-mno-mmx", "-mno-80387",
    "-ftls-model=local-exec",
    "-I", str(root / "include"), "-I", str(root / "src"),
    ]
LINK_FLAGS = [
    "-nostdlib", "-static", "-no-pie",
    "-Wl,--no-dynamic-linker", "-Wl,-z,noexecstack", "-Wl,--build-id=none",
    "-Wl,-z,max-page-size=4096",
]

COMPILERS = {
    "clang": [ZIG, "cc", "-target", "x86_64-freestanding-none"],
    "gcc": ["gcc"],
}


def run(cmd, cwd=None, check=True, capture=False):
    if capture:
        p = subprocess.run(cmd, cwd=cwd, capture_output=True, text=True)
    else:
        p = subprocess.run(cmd, cwd=cwd)
    if check and p.returncode != 0:
        if capture:
            sys.stderr.write(p.stdout + p.stderr)
        raise SystemExit(f"command failed ({p.returncode}): {' '.join(map(str, cmd))}")
    return p


def tool_version(cc):
    if cc == "clang":
        p = run([ZIG, "cc", "--version"], capture=True, check=False)
        return (p.stdout.splitlines() or ["unknown"])[0]
    p = run(["gcc", "--version"], capture=True, check=False)
    return (p.stdout.splitlines() or ["unknown"])[0]


def build_guest(cc, root=EXP, out_root=None):
    """Compile every guest app for compiler `cc`. Returns {name: elf_path}."""
    out_root = Path(out_root or BUILD)
    obj_dir = out_root / f"guest-{cc}" / "obj"
    obj_dir.mkdir(parents=True, exist_ok=True)
    result = {}
    runtime_objs = []
    flags = common_flags(root)
    for src in RUNTIME_SRC:
        obj = obj_dir / (Path(src).stem + ".o")
        run(COMPILERS[cc] + flags + ["-c", str(root / src), "-o", str(obj)])
        runtime_objs.append(str(obj))
    for name, srcs in GUEST_APPS.items():
        objs = []
        for src in srcs:
            obj = obj_dir / f"{name}-{Path(src).stem}.o"
            run(COMPILERS[cc] + flags + ["-c", str(root / src), "-o", str(obj)])
            objs.append(str(obj))
        elf = out_root / f"guest-{cc}" / f"{name}-{cc}.elf"
        run(COMPILERS[cc] + flags + LINK_FLAGS +
            ["-Wl,-T," + str(root / "link" / "arena-user.ld")] +
            objs + runtime_objs + ["-o", str(elf)])
        result[name] = elf
    return result


# ---------------------------------------------------------------- audit ----

PT_LOAD, PT_INTERP, PT_TLS = 1, 3, 7
PF_X, PF_W, PF_R = 1, 2, 4
USER_HALF_LIMIT = 0x0000_8000_0000_0000
X87_OR_SSE_OPERAND = re.compile(r"%(xmm|ymm|zmm|mm[0-7]|st\b|st\()")
X87_MNEMONIC = re.compile(r"^\s*[0-9a-f]+:\s+(?:[0-9a-f]{2} )+\s*(f[a-z0-9]+)\b")


def audit_elf(path):
    """Structural checks mirroring kernel/kernel/src/elf.rs validate()."""
    data = Path(path).read_bytes()
    failures = []
    if data[:4] != b"\x7fELF" or data[4] != 2 or data[5] != 1:
        failures.append("not ELF64 little-endian")
    e_type = int.from_bytes(data[16:18], "little")
    machine = int.from_bytes(data[18:20], "little")
    if e_type != 2:
        failures.append(f"e_type={e_type} (kernel accepts ET_EXEC=2 only)")
    if machine != 62:
        failures.append(f"e_machine={machine} (want EM_X86_64=62)")
    entry = int.from_bytes(data[24:32], "little")
    phoff = int.from_bytes(data[32:40], "little")
    phnum = int.from_bytes(data[56:58], "little")
    segs = []
    tls = None
    for i in range(phnum):
        ph = data[phoff + i * 56: phoff + (i + 1) * 56]
        p_type = int.from_bytes(ph[0:4], "little")
        p_flags = int.from_bytes(ph[4:8], "little")
        vaddr = int.from_bytes(ph[16:24], "little")
        memsz = int.from_bytes(ph[40:48], "little")
        if p_type == PT_INTERP:
            failures.append("PT_INTERP present (kernel refuses dynamic images)")
        if p_type == PT_TLS:
            tls = {"vaddr": vaddr, "memsz": memsz,
                   "filesz": int.from_bytes(ph[32:40], "little"),
                   "align": int.from_bytes(ph[48:56], "little")}
        if p_type == PT_LOAD:
            if p_flags & PF_W and p_flags & PF_X:
                failures.append("W+X PT_LOAD (ADR-0008 W^X)")
            if vaddr % 4096:
                failures.append(f"PT_LOAD vaddr 0x{vaddr:x} not page aligned")
            if vaddr >= USER_HALF_LIMIT or vaddr + memsz > USER_HALF_LIMIT:
                failures.append("PT_LOAD outside user half")
            segs.append({"vaddr": vaddr, "memsz": memsz, "flags": p_flags})
    if not segs:
        failures.append("no PT_LOAD")
    entry_ok = any(s["flags"] & PF_X and s["vaddr"] <= entry < s["vaddr"] + s["memsz"] for s in segs)
    if not entry_ok:
        failures.append("entry not inside an executable PT_LOAD")
    tls_symbols = None
    if tls:
        # crt0 derives the TLS block from __arena_tls_* (linker script). Those
        # must agree with the PT_TLS the linker actually emitted, or the TP
        # placement is wrong (this exact mismatch was found by the guest run).
        tls_symbols = tls_symbol_values(path)
        want = {"__arena_tls_image": tls["vaddr"], "__arena_tls_filesz": tls["filesz"],
                "__arena_tls_memsz": tls["memsz"], "__arena_tls_align": tls["align"]}
        for name, val in want.items():
            got = tls_symbols.get(name)
            if got != val:
                failures.append(f"{name}=0x{(got or 0):x} but PT_TLS says 0x{val:x}")
        if tls["align"] == 0 or 16 % tls["align"]:
            failures.append(f"PT_TLS align {tls['align']} does not divide 16 (crt0 refuses it)")
    return {"path": str(path), "entry": hex(entry), "segments": [
        {"vaddr": hex(s["vaddr"]), "memsz": s["memsz"],
         "perm": ("R" if s["flags"] & PF_R else "-") + ("W" if s["flags"] & PF_W else "-") +
                 ("X" if s["flags"] & PF_X else "-")} for s in segs],
        "tls": tls and {"vaddr": hex(tls["vaddr"]), "filesz": tls["filesz"],
                        "memsz": tls["memsz"], "align": tls["align"]},
        "tls_symbols": tls_symbols,
        "loader_failures": failures}


def tls_symbol_values(path):
    """Values of the __arena_tls_* ABS/ADDR symbols, via readelf -sW."""
    out = run(["readelf", "-sW", str(path)], capture=True).stdout
    vals = {}
    for line in out.splitlines():
        parts = line.split()
        if parts and parts[-1].startswith("__arena_tls_"):
            vals[parts[-1]] = int(parts[1], 16)
    return vals


def disassemble_check(path, objdump="objdump"):
    p = run([objdump, "-d", "--no-show-raw-insn", str(path)], capture=True)
    hits = []
    for line in p.stdout.splitlines():
        if ":\t" not in line:
            continue
        ins = line.split("\t", 1)[1] if "\t" in line else ""
        if X87_OR_SSE_OPERAND.search(ins):
            hits.append(line.strip())
        m = re.match(r"^\s*[0-9a-f]+:\s+([a-z][a-z0-9]*)", line)
        if m and m.group(1).startswith("f") and m.group(1) not in ("fs",):
            hits.append(line.strip())
    return {"instructions": sum(1 for l in p.stdout.splitlines() if ":\t" in l),
            "simd_or_x87": hits[:10], "simd_or_x87_count": len(hits)}


# ----------------------------------------------------------------- abi ----

def abi_check():
    rust = {}
    for line in (REPO / "userspace/abi.rs").read_text().splitlines():
        m = re.match(r"pub const (SYS_[A-Z0-9_]+|VM_PROT_[A-Z]+): u64 = (\d+);", line)
        if m:
            rust[m.group(1)] = int(m.group(2))
    kern = {}
    for line in (REPO / "kernel/kernel/src/arch/x86_64/syscall.rs").read_text().splitlines():
        m = re.match(r"pub const (SYS_[A-Z0-9_]+): u64 = (\d+);", line)
        if m:
            kern[m.group(1)] = int(m.group(2))
    c = {}
    for line in (EXP / "include/arena/abi.h").read_text().splitlines():
        m = re.match(r"#define ARENA_(SYS_[A-Z0-9_]+|VM_PROT_[A-Z]+)\s+(\d+)u?", line)
        if m:
            c[m.group(1)] = int(m.group(2))
    rows, bad = [], []
    for name, val in sorted(c.items()):
        r = rust.get(name)
        k = kern.get(name)
        ok = r == val and (k is None or k == val)
        rows.append({"name": name, "c": val, "userspace_abi_rs": r, "kernel_syscall_rs": k, "match": ok})
        if not ok:
            bad.append(name)
    return {"checked": len(rows), "mismatches": bad, "rows": rows}


# ---------------------------------------------------------------- host ----

def host_tests():
    out = BUILD / "host"
    out.mkdir(parents=True, exist_ok=True)
    exe = out / "test_runtime"
    srcs = ["src/alloc.c", "src/string.c", "src/stdio.c", "src/vm.c", "host/host_main.c",
            "host/test_runtime.c"]
    cmd = ["gcc", "-std=gnu11", "-O1", "-g", "-Wall", "-Wextra", "-Werror",
           "-DARENA_HOSTED", "-fsanitize=address,undefined", "-fno-sanitize-recover=all",
           "-I", str(EXP / "include"), "-I", str(EXP / "src")]
    run(cmd + [str(EXP / s) for s in srcs] + ["-o", str(exe)])
    p = run([str(exe)], capture=True, check=False)
    sys.stdout.write(p.stdout)
    sys.stderr.write(p.stderr)
    return p.returncode == 0, p.stdout


# --------------------------------------------------------------- guest ----

PROBE_FEED = [
    # Wait for the probe's own result line before typing `shutdown` at the
    # shell prompt, so the shutdown cannot race the probe's last group.
    (b"CRT-PROBE RESULT", 1, b""),
    (b"arena>", 1, b"shutdown\r"),
]


def boot_capture(tree, esp, log_path, timeout_s, feed=None):
    """Boot the scratch ESP through the repository's own QEMU harness.

    The kernel runs every milestone suite (M1..M12) before the production
    services and the shell; the probe spawn sits after all of them, so the
    boot must pass the full fixture set. tools/mtest.py run_qemu() attaches
    exactly that set (scratch disk, NIC with host DNS/TCP fixtures, RNG,
    keyboard, console) and feeds `shutdown` at the shell prompt. It is
    imported from the SCRATCH tree so its build directory stays scratch-side.
    Returns (serial_text, reason)."""
    sys.path.insert(0, str(tree / "tools"))
    import mtest  # noqa: E402
    try:
        verdict, serial, _dt = mtest.run_qemu("cprobe", esp, feed=feed)
        reason = f"mtest-verdict-{verdict}"
    except subprocess.TimeoutExpired:
        serial = (tree / "build" / "serial-cprobe.log").read_text(errors="replace") \
            if (tree / "build" / "serial-cprobe.log").exists() else ""
        reason = "timeout"
    log_path.write_text(serial)
    return serial, reason


def guest_run(cc, keep=False, timeout_s=240, baseline=False):
    """Build the boot-probe kernel from a SCRATCH copy of the repository and
    boot it. Nothing outside the scratch tree is modified."""
    scratch = Path(tempfile.mkdtemp(prefix="arena-cprobe-"))
    try:
        def ignore(dirpath, names):
            skip = {".git", "releases", ".cache", "__pycache__"}
            if Path(dirpath) == REPO:
                skip.add("build")
            return [n for n in names if n in skip]
        tree = scratch / "tree"
        shutil.copytree(REPO, tree, ignore=ignore, symlinks=True)
        guest_build = tree / "experiments/c-runtime/build"
        if not baseline:
            # Baseline runs boot the UNPATCHED tree: the control for every
            # probe result (which services start, which serial lines are
            # pre-existing).
            patch = EXP / "patches" / "0001-boot-probe-c-prototype.patch"
            run(["patch", "-p1", "-s", "-d", str(tree), "-i", str(patch)])
            elf = build_guest(cc, root=tree / "experiments/c-runtime",
                              out_root=guest_build)["crt-probe"]
            shutil.copy(elf, guest_build / "crt-probe-embedded.elf")
        env = dict(os.environ)
        bootstrap_env = Path("/opt/rust/prefix/bin")
        if bootstrap_env.exists():
            env["PATH"] = f"{bootstrap_env}:{env.get('PATH', '')}"
        BUILD.mkdir(exist_ok=True)
        build = subprocess.run(
            ["bash", "-lc", "source tools/dev-env/env.sh >/dev/null 2>&1; tools/build.sh --image"],
            cwd=tree, env=env, capture_output=True, text=True)
        (BUILD / f"guest-build-{cc}.log").write_text(build.stdout + build.stderr)
        if build.returncode != 0:
            return {"built": False, "serial": "", "reason": "kernel-build-failed"}
        esp = tree / "build" / "arena-esp.img"
        label = "baseline" if baseline else cc
        text, reason = boot_capture(tree, esp, BUILD / f"guest-serial-{label}.log", timeout_s,
                                    feed=None if baseline else PROBE_FEED)
        return {"built": True, "serial": text, "reason": reason}
    finally:
        if not keep:
            shutil.rmtree(scratch, ignore_errors=True)


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("command", choices=["build", "audit", "abi", "host", "guest", "all"])
    ap.add_argument("--cc", action="append", choices=list(COMPILERS), help="compilers (default: both)")
    ap.add_argument("--keep-scratch", action="store_true")
    ap.add_argument("--baseline", action="store_true",
                    help="guest: boot the UNPATCHED tree as the control")
    args = ap.parse_args()
    ccs = args.cc or list(COMPILERS)
    BUILD.mkdir(exist_ok=True)
    summary = {"tools": {}, "results": {}}
    for cc in ccs:
        summary["tools"][cc] = tool_version(cc)

    if args.command in ("build", "audit", "all"):  # guest ELFs
        elfs = {}
        for cc in ccs:
            elfs[cc] = build_guest(cc)
        summary["guest_elfs"] = {cc: {k: str(v) for k, v in d.items()} for cc, d in elfs.items()}
        if args.command in ("audit", "all"):
            audits = {}
            for cc, d in elfs.items():
                for name, elf in d.items():
                    a = audit_elf(elf)
                    a["disasm"] = disassemble_check(elf)
                    audits[f"{name}-{cc}"] = a
            summary["results"]["static_audit"] = audits
            bad = {k: v for k, v in audits.items()
                   if v["loader_failures"] or v["disasm"]["simd_or_x87_count"]}
            summary["results"]["static_audit_pass"] = not bad
            print(json.dumps(audits, indent=2))
    if args.command in ("abi", "all"):
        a = abi_check()
        summary["results"]["abi"] = {"checked": a["checked"], "mismatches": a["mismatches"]}
        print(f"abi: checked {a['checked']} constants, mismatches={a['mismatches']}")
    if args.command in ("host", "all"):
        ok, out = host_tests()
        summary["results"]["host_pass"] = ok
    if args.command in ("guest", "all"):
        cc = args.cc[0] if args.cc else "clang"
        g = guest_run(cc, keep=args.keep_scratch, baseline=args.baseline)
        sr = g["serial"]
        markers = [l for l in sr.splitlines() if "crt-probe" in l or "CRT-PROBE" in l]
        result_line = next((l for l in markers if "CRT-PROBE RESULT" in l), None)
        summary["results"]["guest"] = {
            "compiler": cc,
            "baseline_unpatched": args.baseline,
            "kernel_built": g["built"],
            "stop_reason": g["reason"],
            "markers": markers,
            "result_line": result_line,
            "guest_executed": result_line is not None,
        }
        print("\n".join(markers) if markers else "(no probe markers on serial)")
        print("stop reason:", g["reason"], "| guest executed:", result_line is not None)
    (BUILD / "summary.json").write_text(json.dumps(summary, indent=2, default=str))
    print(f"summary: {BUILD / 'summary.json'}")


if __name__ == "__main__":
    main()
