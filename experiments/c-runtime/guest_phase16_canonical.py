#!/usr/bin/env python3
"""Phase 16 CANONICAL guest runner (B1 resolved; not acceptance).

Runs the unchanged guest suites (guest_c2.py / guest_native.py /
guest_fp.py / guest_stdin.py / guest_notification.py, including their
exact judges, signed APB1 installs, launch flow, and exit-status
sequences) without substituting `mtest.build`: every suite builds its
image through the canonical `tools/build.sh` path. The protected Phase
14 fixture is therefore rebuilt and hash-guarded on every build.

Every run emits a self-identifying evidence envelope
(`build/guest-*/evidence-envelope-<suite>.json`, also merged into the
suite summary under key `phase16_evidence`) so classification survives
copying a JSON file away from its documentation: B1 status and official
component hashes, source commit + dirty paths, EFI/ESP hashes,
protected Phase 14 identities, emulator/firmware/isolated-runtime
identities, compiler and app identities, exact test command, semantic
verdict, process exit, and failure details.

A clean source tree is labelled a qualification candidate for M/C
review. A dirty tree is explicitly downgraded in the envelope. Nothing
here declares Phase 16 accepted.

Usage:
  ARENA_QEMU=... ARENA_OVMF_CODE=... ARENA_OVMF_VARS=... ARENA_ZIG=... \
  python3 guest_phase16_canonical.py --suite c2|native|fp|stdin|notification \
      -- [--cc gcc]
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

SUMMARY_DIR = {
    "c2": EXP / "build" / "guest-c2",
    "native": EXP / "build" / "guest-native",
    "fp": EXP / "build" / "guest-fp",
    "stdin": EXP / "build" / "guest-stdin",
    "notification": EXP / "build" / "guest-notification",
}

OFFICIAL_COMPONENTS = {
    "rust-std-1.97.0-x86_64-unknown-none.tar.xz":
        "5b6702d1ce758d4d01fc3861085bbf1ee19eb9d7f7dc69e066c299ee9606168a",
    "rust-std-1.97.0-x86_64-unknown-uefi.tar.xz":
        "2c48c8b659d65ec74e916e44e5ecac8e77f4c1abce51d01d661f133233641ad0",
}
EXPECTED_PIE = "d7b0a8cf9c735c3898a867d824563f06b0d949df80fa1a9c96f9180a395fea2e"
EXPECTED_APB1 = "04add54f09042353fc42511978c82f8b8efb87f7d5f0657f9116f456276b3dfa"


def sha256_file(path: str | Path | None) -> str | None:
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


def git_status() -> list[str]:
    p = subprocess.run(["git", "-C", str(REPO), "status", "--porcelain"],
                       capture_output=True, text=True)
    return [ln for ln in p.stdout.splitlines() if ln.strip()]


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
                for token in re.findall(r"/[^\s\"'\\]+qemu-system-x86_64[^\s\"'\\]*", text):
                    if Path(token).is_file():
                        info["emulator_binary"] = token
                        info["emulator_binary_sha256"] = sha256_file(token)
                        break
    info["version"] = run_out(env.split() + ["--version"]) if env else "default"
    return info


def installed_component_identity() -> dict:
    sysroot = run_out(["rustc", "--print", "sysroot"])
    out: dict = {"sysroot": sysroot, "official_archive_sha256": OFFICIAL_COMPONENTS}
    if sysroot and not sysroot.startswith("<"):
        for target in ("x86_64-unknown-none", "x86_64-unknown-uefi"):
            libdir = Path(sysroot) / "lib" / "rustlib" / target / "lib"
            cores = sorted(libdir.glob("libcore-*.rlib"))
            out[target] = {
                "libcore": str(cores[0]) if cores else None,
                "libcore_sha256": sha256_file(cores[0]) if cores else None,
            }
    return out


def envelope(args) -> dict:
    esp = REPO / "build/arena-esp.img"
    efi = REPO / "build/arena-boot.efi"
    glibc_dir = os.environ.get("ARENA_GLIBC_DIR") or "/nonexistent"
    dirty = git_status()
    pie = sha256_file(REPO / "userspace/phase14-pie/fixture.elf")
    apb1 = sha256_file(REPO / "userspace/phase14-pie/phase14-pie.apb1")
    return {
        "classification": ("CANONICAL — QUALIFICATION CANDIDATE (not acceptance)"
                           if not dirty else
                           "CANONICAL TOOLCHAIN, DIRTY SOURCE — NOT FINAL QUALIFICATION"),
        "qualification_candidate": not dirty,
        "accepted": False,
        "blockers": [],
        "b1": {
            "status": "resolved",
            "source": "docs/phase16/dependencies/rust-1.97.0/README.md",
            "components": installed_component_identity(),
            "protected_phase14": {
                "pie_sha256": pie,
                "pie_expected": EXPECTED_PIE,
                "pie_matches": pie == EXPECTED_PIE,
                "signed_apb1_sha256": apb1,
                "signed_apb1_expected": EXPECTED_APB1,
                "signed_apb1_matches": apb1 == EXPECTED_APB1,
            },
        },
        "canonical_build": True,
        "build_script": "tools/build.sh via each suite's unchanged mtest.build",
        "suite": args.suite,
        "test_command": " ".join(sys.argv),
        "utc": datetime.datetime.now(datetime.timezone.utc).isoformat(),
        "source_commit": run_out(["git", "-C", str(REPO), "rev-parse", "HEAD"]),
        "dirty_paths": dirty,
        "images": {
            "esp": str(esp), "esp_sha256": sha256_file(esp),
            "efi": str(efi), "efi_sha256": sha256_file(efi),
            "protected_phase14_fixture_sha256": pie,
            "protected_phase14_signed_apb1_sha256": apb1,
        },
        "emulator": qemu_identity(),
        "firmware": {
            "OVMF_CODE": os.environ.get("ARENA_OVMF_CODE"),
            "OVMF_CODE_sha256": sha256_file(os.environ.get("ARENA_OVMF_CODE")),
            "OVMF_VARS": os.environ.get("ARENA_OVMF_VARS"),
            "OVMF_VARS_sha256": sha256_file(os.environ.get("ARENA_OVMF_VARS")),
        },
        "isolated_runtime": {
            "note": "Pinned patched QEMU requires GLIBC>=2.38; wrapper content "
                    "is recorded under emulator.wrapper_head. Setup: "
                    "tools/dev-env/build-isolated-glibc.sh and "
                    "docs/phase16/BACKEND-RECOVERY.md. Host glibc unchanged.",
            "ARENA_GLIBC_DIR": os.environ.get("ARENA_GLIBC_DIR"),
            "glibc_libc_sha256": sha256_file(glibc_dir + "/lib/libc.so.6"),
            "glibc_ld_sha256": sha256_file(glibc_dir + "/lib/ld-linux-x86-64.so.2"),
        },
        "compilers": {
            "gcc": run_out(["gcc", "--version"]),
            "zig": run_out([os.environ.get("ARENA_ZIG", "zig"), "version"]),
            "rustc": run_out(["rustc", "--version"]),
            "cargo": run_out(["cargo", "--version"]),
        },
    }


def refresh_after_run(env: dict) -> dict:
    """Refresh hashes that canonical mtest.build may have just produced."""
    esp = REPO / "build/arena-esp.img"
    efi = REPO / "build/arena-boot.efi"
    pie = sha256_file(REPO / "userspace/phase14-pie/fixture.elf")
    apb1 = sha256_file(REPO / "userspace/phase14-pie/phase14-pie.apb1")
    env["images"].update({
        "esp_sha256": sha256_file(esp),
        "efi_sha256": sha256_file(efi),
        "protected_phase14_fixture_sha256": pie,
        "protected_phase14_signed_apb1_sha256": apb1,
    })
    env["b1"]["protected_phase14"].update({
        "pie_sha256": pie,
        "pie_matches": pie == EXPECTED_PIE,
        "signed_apb1_sha256": apb1,
        "signed_apb1_matches": apb1 == EXPECTED_APB1,
    })
    env["dirty_paths_after_run"] = git_status()
    return env


def emit_envelope(suite: str, env: dict, rc: int, failure: str | None = None) -> Path | None:
    d = SUMMARY_DIR[suite]
    d.mkdir(parents=True, exist_ok=True)
    env = refresh_after_run(dict(env))
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
        if "mutation_fixture" in summary:
            env["mutation_fixture"] = summary["mutation_fixture"]
        summary["phase16_evidence"] = env
        summary["evidence_class"] = (
            "GUEST-CANONICAL" if env["qualification_candidate"]
            else "GUEST-CANONICAL-TOOLCHAIN-DIRTY")
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
    ap.add_argument("--suite", choices=["c2", "native", "fp", "stdin", "notification"],
                    default="c2")
    ap.add_argument("rest", nargs=argparse.REMAINDER)
    args = ap.parse_args()
    print(f"PHASE16 CANONICAL RUNNER: suite={args.suite} — B1 resolved; not acceptance")
    env = envelope(args)
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
        elif args.suite == "notification":
            import guest_notification
            rc = guest_notification.main()
        else:
            import guest_native
            rc = guest_native.main()
    except SystemExit as e:
        rc = int(e.code or 0) if isinstance(e.code, int) else 1
        failure = f"SystemExit: {e.code!r}"
        env_path = emit_envelope(args.suite, env, rc, failure)
        if env_path:
            print(f"PHASE16 EVIDENCE ENVELOPE (CANONICAL): {env_path}")
        raise
    except BaseException as e:  # noqa: BLE001 - record, then propagate
        env_path = emit_envelope(args.suite, env, 1, f"{type(e).__name__}: {e}")
        if env_path:
            print(f"PHASE16 EVIDENCE ENVELOPE (CANONICAL): {env_path}")
        raise
    env_path = emit_envelope(args.suite, env, rc, failure)
    if env_path:
        print(f"PHASE16 EVIDENCE ENVELOPE (CANONICAL): {env_path}")
    return rc


if __name__ == "__main__":
    raise SystemExit(main())
