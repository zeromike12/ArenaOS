#!/usr/bin/env python3
"""Phase 16 DEVELOPMENT guest runner — NOT QUALIFICATION EVIDENCE.

Runs the unchanged guest suites (guest_c2.py / guest_native.py /
guest_fp.py / guest_stdin.py, including their exact judges, signed APB1
installs, launch flow, and exit-status sequences) on a kernel built by
tools/build_phase16_dev.sh (blocker B1: official rust-std components
unavailable in this sandbox; docs/phase16/DEPENDENCY-REQUEST.md). The
only substitution is `mtest.build` -> the prebuilt development ESP.

Every run emits a self-identifying evidence envelope
(`build/guest-*/evidence-envelope-<suite>.json`, also merged into the
suite summary under key `phase16_evidence`) so the classification
survives copying a JSON file away from its documentation: DEVELOPMENT
status, source commit + dirty paths, EFI/ESP hashes, emulator/firmware/
isolated-runtime identities, compiler and app identities, exact test
command, semantic verdict, process exit, and failure details. The
envelope is emitted on failure paths too. Nothing here becomes
qualification evidence by being recorded; results must be re-run on a
canonical build (B1 resolved) for qualification.

Usage:
  ARENA_QEMU=... ARENA_OVMF_CODE=... ARENA_OVMF_VARS=... ARENA_ZIG=... \
  python3 guest_phase16_dev.py --suite c2|native|fp|stdin \
      [--esp build/arena-esp.img] -- [--cc gcc]
"""
from __future__ import annotations

import argparse
import datetime
import hashlib
import json
import os
import re
import subprocess
import sys
from pathlib import Path

EXP = Path(__file__).resolve().parent
REPO = EXP.parent.parent
sys.path.insert(0, str(EXP))
sys.path.insert(0, str(REPO / "tools"))

import mtest  # noqa: E402

SUMMARY_DIR = {
    "c2": EXP / "build" / "guest-c2",
    "native": EXP / "build" / "guest-native",
    "fp": EXP / "build" / "guest-fp",
    "stdin": EXP / "build" / "guest-stdin",
}


def sha256_file(path: str | None) -> str | None:
    if not path:
        return None
    p = Path(path)
    if not p.is_file():
        return None
    h = hashlib.sha256()
    h.update(p.read_bytes())
    return h.hexdigest()


def run_out(cmd: list[str]) -> str:
    try:
        p = subprocess.run(cmd, capture_output=True, text=True, timeout=20)
        return (p.stdout or p.stderr or "").strip().splitlines()[0] if (p.stdout or p.stderr) else ""
    except Exception as e:  # noqa: BLE001 - identity is best-effort metadata
        return f"<unavailable: {e}>"


def qemu_identity() -> dict:
    env = os.environ.get("ARENA_QEMU", "")
    info: dict = {"ARENA_QEMU": env or "<arena_env default>"}
    if env:
        first = env.split()[0]
        p = Path(first)
        if p.is_file():
            info["file"] = str(p)
            data = p.read_bytes()
            info["sha256"] = hashlib.sha256(data).hexdigest()
            if data.startswith(b"#!"):
                text = data.decode(errors="replace")
                info["wrapper_head"] = text.splitlines()[:8]
                # Hash the real emulator binary a wrapper names, when it
                # names one by absolute path (the pinned patched QEMU).
                for token in re.findall(r"/[^\s\"'\\]+qemu-system-x86_64[^\s\"'\\]*", text):
                    if Path(token).is_file():
                        info["emulator_binary"] = token
                        info["emulator_binary_sha256"] = sha256_file(token)
                        break
    info["version"] = run_out(env.split() + ["--version"]) if env else "default"
    return info


def envelope(args, esp: Path) -> dict:
    efi = esp.parent / "arena-boot.efi"
    glibc_dir = os.environ.get("ARENA_GLIBC_DIR") or "/nonexistent"
    return {
        "classification": "DEVELOPMENT — NOT QUALIFICATION EVIDENCE",
        "qualified": False,
        "blockers": ["B1: official rust-std components unavailable "
                     "(docs/phase16/DEPENDENCY-REQUEST.md)"],
        "canonical_build": False,
        "build_script": "tools/build_phase16_dev.sh (committed protected "
                        "Phase 14 fixture verified, not rebuilt)",
        "suite": args.suite,
        "test_command": " ".join(sys.argv),
        "utc": datetime.datetime.now(datetime.timezone.utc).isoformat(),
        "source_commit": run_out(["git", "-C", str(REPO), "rev-parse", "HEAD"]),
        "dirty_paths": [ln for ln in subprocess.run(
            ["git", "-C", str(REPO), "status", "--porcelain"],
            capture_output=True, text=True).stdout.splitlines() if ln.strip()],
        "images": {
            "esp": str(esp), "esp_sha256": sha256_file(str(esp)),
            "efi": str(efi), "efi_sha256": sha256_file(str(efi)),
            "protected_phase14_fixture_sha256": sha256_file(
                str(REPO / "userspace/phase14-pie/fixture.elf")),
        },
        "emulator": qemu_identity(),
        "firmware": {
            "OVMF_CODE": os.environ.get("ARENA_OVMF_CODE"),
            "OVMF_CODE_sha256": sha256_file(os.environ.get("ARENA_OVMF_CODE")),
            "OVMF_VARS": os.environ.get("ARENA_OVMF_VARS"),
            "OVMF_VARS_sha256": sha256_file(os.environ.get("ARENA_OVMF_VARS")),
        },
        "isolated_runtime": {
            "note": "Pinned patched QEMU requires GLIBC>=2.38; when a wrapper "
                    "is used its content (incl. isolated glibc path) is "
                    "recorded under emulator.wrapper_head. Setup: "
                    "tools/dev-env/build-isolated-glibc.sh and "
                    "docs/phase16/BACKEND-RECOVERY.md.",
            "ARENA_GLIBC_DIR": os.environ.get("ARENA_GLIBC_DIR"),
            "glibc_libc_sha256": sha256_file(glibc_dir + "/lib/libc.so.6"),
            "glibc_ld_sha256": sha256_file(glibc_dir + "/lib/ld-linux-x86-64.so.2"),
        },
        "compilers": {
            "gcc": run_out(["gcc", "--version"]),
            "zig": run_out([os.environ.get("ARENA_ZIG", "zig"), "version"]),
            "rustc": run_out(["rustc", "--version"]),
        },
    }


def emit_envelope(suite: str, env: dict, rc: int, failure: str | None = None) -> Path | None:
    d = SUMMARY_DIR[suite]
    d.mkdir(parents=True, exist_ok=True)
    env = dict(env)
    env["process_exit"] = rc
    if failure:
        env["failure"] = failure
    cands = sorted(d.glob("summary-*.json"), key=lambda p: p.stat().st_mtime)
    summary = None
    if cands:
        try:
            summary = json.loads(cands[-1].read_text())
        except Exception:  # noqa: BLE001
            summary = None
    if summary is not None:
        env["semantic_verdict"] = {
            "summary": str(cands[-1]),
            "runs": summary.get("runs"),
            "passed": summary.get("passed"),
            "failed": (summary.get("runs") or 0) - (summary.get("passed") or 0),
            "failures": [f for r in summary.get("results", [])
                         for f in r.get("failures", [])],
        }
        if "identity" in summary:
            env["app_identity"] = summary["identity"]
        summary["phase16_evidence"] = env
        summary["evidence_class"] = "GUEST-DEVELOPMENT"
        cands[-1].write_text(json.dumps(summary, indent=2, default=str))
    env_path = d / f"evidence-envelope-{suite}.json"
    try:
        env_path.write_text(json.dumps(env, indent=2, default=str))
        return env_path
    except Exception as e:  # noqa: BLE001
        print(f"warning: could not write evidence envelope: {e}", file=sys.stderr)
        return None


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--suite", choices=["c2", "native", "fp", "stdin"], default="c2")
    ap.add_argument("--esp", default=str(REPO / "build/arena-esp.img"))
    ap.add_argument("rest", nargs=argparse.REMAINDER)
    args = ap.parse_args()
    esp = Path(args.esp)
    if not esp.is_file():
        print(f"error: development ESP missing: {esp} (run tools/build_phase16_dev.sh --image)",
              file=sys.stderr)
        return 2
    print(f"PHASE16 DEV RUNNER: suite={args.suite} esp={esp} — NOT QUALIFICATION EVIDENCE")
    env = envelope(args, esp)
    mtest.build = lambda label, desktop=False: esp  # noqa: ARG005 - dev-only substitution
    rest = args.rest[1:] if args.rest[:1] == ["--"] else args.rest
    sys.argv = [sys.argv[0]] + rest
    rc = 1
    failure = None
    try:
        if args.suite == "c2":
            import guest_c2
            rc = guest_c2.main()
        elif args.suite == "fp":
            import guest_fp
            rc = guest_fp.main()
        elif args.suite == "stdin":
            import guest_stdin
            rc = guest_stdin.main()
        else:
            import guest_native
            rc = guest_native.main()
    except SystemExit as e:
        rc = int(e.code or 0) if isinstance(e.code, int) else 1
        failure = f"SystemExit: {e.code!r}"
        env_path = emit_envelope(args.suite, env, rc, failure)
        if env_path:
            print(f"PHASE16 EVIDENCE ENVELOPE (DEVELOPMENT): {env_path}")
        raise
    except BaseException as e:  # noqa: BLE001 - record, then propagate
        env_path = emit_envelope(args.suite, env, 1, f"{type(e).__name__}: {e}")
        if env_path:
            print(f"PHASE16 EVIDENCE ENVELOPE (DEVELOPMENT): {env_path}")
        raise
    env_path = emit_envelope(args.suite, env, rc, failure)
    if env_path:
        print(f"PHASE16 EVIDENCE ENVELOPE (DEVELOPMENT): {env_path}")
    return rc


if __name__ == "__main__":
    raise SystemExit(main())
