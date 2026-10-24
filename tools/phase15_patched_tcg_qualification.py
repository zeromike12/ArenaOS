#!/usr/bin/env python3
"""Run or verify Phase 15 using the pinned locally modified QEMU TCG build.

This runner is separate from tools/phase15_external, whose hardware-backed
meaning is unchanged. Every invocation pins the emulator, strict EFI/ESP, and
OVMF hashes in docs/phase15/patched-tcg-qualification.json.
"""

from __future__ import annotations

import argparse
import hashlib
from importlib.metadata import PackageNotFoundError, version
import json
import os
import re
import shlex
import shutil
import subprocess
import sys
from datetime import datetime, timezone
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
MANIFEST_DEFAULT = ROOT / "docs/phase15/patched-tcg-qualification.json"
PROFILE_LABELS = {"phase15-avx": "max,+nx,+smep,+smap",
                  "phase15-qemu64": "qemu64,+nx,+smep,+smap"}
TIERS = {"T2": 10, "T3": 25, "T4": 100}


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def read_manifest(path: Path) -> dict:
    try:
        value = json.loads(path.read_text())
    except (OSError, json.JSONDecodeError) as error:
        raise SystemExit(f"invalid backend manifest {path}: {error}") from error
    if value.get("backend_id") != "arenaos-qemu-11.1.2-local-patched-tcg-xm":
        raise SystemExit("backend manifest identity is not the approved modified-TCG backend")
    return value


def create_new_dir(path: Path) -> None:
    if path.exists() and any(path.iterdir()):
        raise SystemExit(f"refusing to overwrite nonempty receipt directory: {path}")
    path.mkdir(parents=True, exist_ok=True)


def validate_inputs(args: argparse.Namespace, manifest: dict) -> dict:
    paths = {
        "qemu": args.qemu_binary.resolve(),
        "lib_dir": args.lib_dir.resolve(),
        "rom_dir": args.rom_dir.resolve(),
        "efi": args.efi.resolve(),
        "esp": args.esp.resolve(),
        "ovmf_code": args.ovmf_code.resolve(),
        "ovmf_vars": args.ovmf_vars.resolve(),
    }
    for name, path in paths.items():
        if name == "lib_dir" or name == "rom_dir":
            if not path.is_dir():
                raise SystemExit(f"missing {name} directory: {path}")
        elif not path.is_file():
            raise SystemExit(f"missing {name} file: {path}")

    expected = {
        "qemu": manifest["emulator"]["binary_sha256"],
        "efi": manifest["strict_arenaos_image"]["efi_sha256"],
        "esp": manifest["strict_arenaos_image"]["esp_sha256"],
        "ovmf_code": manifest["firmware"]["code_sha256"],
        "ovmf_vars": manifest["firmware"]["vars_template_sha256"],
    }
    actual = {name: sha256(paths[name]) for name in expected}
    for name, digest in expected.items():
        if actual[name] != digest:
            raise SystemExit(f"{name} SHA-256 mismatch: expected {digest}, got {actual[name]}")

    runtime_hashes = {}
    for library in manifest["runtime"]["packaged_runtime_libraries"]:
        library_path = paths["lib_dir"] / library["name"]
        if not library_path.is_file():
            raise SystemExit(f"missing pinned runtime library: {library_path}")
        digest = sha256(library_path)
        if digest != library["sha256"]:
            raise SystemExit(f"runtime library SHA-256 mismatch for {library_path}: "
                             f"expected {library['sha256']}, got {digest}")
        runtime_hashes[library["name"]] = digest

    # Inspect the actual FAT ESP and reject a matching outer hash that does
    # not embed the exact paired EFI.
    try:
        from pyfatfs.PyFatFS import PyFatFS
    except ImportError as error:
        raise SystemExit("pyfatfs is required; install the pinned official PyPI dependency") from error
    try:
        pyfatfs_version = version("pyfatfs")
    except PackageNotFoundError as error:
        raise SystemExit("pyfatfs package metadata is missing; install the pinned requirements") from error
    if pyfatfs_version != "1.1.0":
        raise SystemExit(f"pyfatfs 1.1.0 is required, found {pyfatfs_version}")
    fs = PyFatFS(str(paths["esp"]), read_only=True)
    try:
        if fs.getbytes("/EFI/BOOT/BOOTX64.EFI") != paths["efi"].read_bytes():
            raise SystemExit("strict ESP does not embed the pinned strict EFI")
    finally:
        fs.close()
    return {"paths": {name: str(path) for name, path in paths.items()},
            "sha256": actual, "runtime_library_sha256": runtime_hashes}


def qemu_prefix(paths: dict) -> list[str]:
    binary = paths["qemu"]
    libs = paths["lib_dir"]
    roms = paths["rom_dir"]
    return ["env", f"LD_LIBRARY_PATH={libs}", str(binary),
            "-accel", "tcg", "-L", str(roms)]


def run_preflight(args: argparse.Namespace, manifest: dict,
                  output_dir: Path | None = None) -> tuple[dict, dict]:
    identity = validate_inputs(args, manifest)
    env = os.environ.copy()
    env["LD_LIBRARY_PATH"] = str(args.lib_dir.resolve())
    qemu = str(args.qemu_binary.resolve())
    roms = str(args.rom_dir.resolve())
    version = subprocess.run([qemu, "--version"], env=env, text=True,
                             stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
    accel = subprocess.run([qemu, "-accel", "help"], env=env, text=True,
                           stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
    advertised = [line.strip() for line in accel.stdout.splitlines()[1:] if line.strip()]
    base = {
        "backend_id": manifest["backend_id"],
        "label": "locally modified QEMU TCG",
        "accelerator_requested": "-accel tcg",
        "accelerator_initialized": False,
        "hardware_acceleration": False,
        "qemu_version": version.stdout.strip(),
        "accelerators_advertised": advertised,
        **identity,
    }
    if output_dir is not None:
        output_dir.mkdir(parents=True, exist_ok=True)
        (output_dir / "preflight-version.log").write_text(version.stdout)
        (output_dir / "preflight-accelerators.log").write_text(accel.stdout)
    if version.returncode != 0 or "QEMU emulator version 11.1.2" not in version.stdout:
        if output_dir is not None:
            (output_dir / "preflight.json").write_text(json.dumps(base, indent=2, sort_keys=True) + "\n")
        raise SystemExit(f"unexpected QEMU version output: {version.stdout.strip()}")
    expected_accelerators = manifest["emulator"]["supported_accelerators"]
    if accel.returncode != 0 or advertised != expected_accelerators:
        if output_dir is not None:
            (output_dir / "preflight.json").write_text(json.dumps(base, indent=2, sort_keys=True) + "\n")
        raise SystemExit(f"TCG is not advertised by the pinned binary: {accel.stdout.strip()}")

    # This QMP smoke initializes a q35 machine with explicit TCG and then
    # exits cleanly. It detects runtime library, ROM, and accelerator-init
    # failures before any ArenaOS guest trial starts.
    smoke_cmd = [qemu, "-accel", "tcg", "-L", roms, "-machine", "q35",
                 "-m", "64M", "-cpu", "qemu64,+nx,+smep,+smap", "-nodefaults",
                 "-display", "none", "-serial", "none", "-monitor", "none",
                 "-S", "-qmp", "stdio"]
    smoke = subprocess.run(smoke_cmd, env=env,
                           input='{"execute":"qmp_capabilities"}\n{"execute":"quit"}\n',
                           text=True, stdout=subprocess.PIPE,
                           stderr=subprocess.STDOUT, timeout=20)
    smoke_pass = smoke.returncode == 0 and '"reason": "host-qmp-quit"' in smoke.stdout
    result = base | {
        "accelerator_initialized": smoke_pass,
        "smoke_command": shlex.join(smoke_cmd),
        "smoke_exit": smoke.returncode,
        "smoke_output": smoke.stdout,
    }
    if output_dir is not None:
        (output_dir / "preflight.json").write_text(json.dumps(result, indent=2, sort_keys=True) + "\n")
        (output_dir / "preflight-qemu-smoke.log").write_text(smoke.stdout)
    if not smoke_pass:
        raise SystemExit(f"explicit TCG initialization smoke failed (exit {smoke.returncode}):\n{smoke.stdout}")
    return result, identity


def runner_env(args: argparse.Namespace) -> tuple[dict[str, str], list[str]]:
    env = os.environ.copy()
    prefix = qemu_prefix({"qemu": args.qemu_binary.resolve(),
                          "lib_dir": args.lib_dir.resolve(),
                          "rom_dir": args.rom_dir.resolve()})
    env["ARENA_QEMU"] = shlex.join(prefix)
    env["ARENA_OVMF_CODE"] = str(args.ovmf_code.resolve())
    env["ARENA_OVMF_VARS"] = str(args.ovmf_vars.resolve())
    env["ARENA_PHASE15_EFI"] = str(args.efi.resolve())
    env["ARENA_PHASE15_ESP"] = str(args.esp.resolve())
    env["PHASE15_PYTHON"] = sys.executable
    env["PYTHONPATH"] = os.pathsep.join(
        item for item in (str(ROOT / "build/onboarding/python"), env.get("PYTHONPATH", "")) if item
    )
    return env, prefix


def metadata(args: argparse.Namespace, manifest: dict, identity: dict,
             command: list[str], run_kind: str) -> dict:
    try:
        source_commit = subprocess.run(["git", "rev-parse", "HEAD"], cwd=ROOT,
                                      check=True, text=True, stdout=subprocess.PIPE).stdout.strip()
    except (OSError, subprocess.CalledProcessError):
        source_commit = None
    return {
        "schema_version": 1,
        "run_kind": run_kind,
        "created_utc": datetime.now(timezone.utc).isoformat(),
        "source_commit": source_commit,
        "python_executable": sys.executable,
        "python_version": sys.version.split()[0],
        "backend_id": manifest["backend_id"],
        "backend_label": "locally modified QEMU TCG; not upstream-QEMU or hardware-backed",
        "emulator": manifest["emulator"]["binary_sha256"],
        "qualification_script_sha256": {
            "test_m15_guest.py": sha256(ROOT / "tools/test_m15_guest.py"),
            "mtest.py": sha256(ROOT / "tools/mtest.py"),
            "stability_loop.sh": sha256(ROOT / "tools/stability_loop.sh"),
            "phase15_boot_receipt.py": sha256(ROOT / "tools/phase15_patched_tcg_boot_receipt.py"),
        },
        "command": shlex.join(command),
        "accelerator": "tcg (explicit -accel tcg)",
        **identity,
    }


def save_boot_receipt(args: argparse.Namespace, out: Path, label: str,
                      serial_src: Path, stderr_src: Path, qemu_rc: int | None,
                      semantic_pass: bool, manifest_path: Path) -> dict:
    serial_dest = out / f"serial-{label}.log"
    stderr_dest = out / f"qemu-stderr-{label}.log"
    if serial_src.is_file():
        shutil.copyfile(serial_src, serial_dest)
    else:
        serial_dest.write_text("")
    if stderr_src.is_file():
        shutil.copyfile(stderr_src, stderr_dest)
    else:
        stderr_dest.write_text("")
    rc = qemu_rc if qemu_rc is not None else 255
    cpu = PROFILE_LABELS[label]
    command = [sys.executable, str(ROOT / "tools/phase15_patched_tcg_boot_receipt.py"),
               "--manifest", str(manifest_path), "--serial", str(serial_dest),
               "--qemu-stderr", str(stderr_dest), "--output", str(out / f"{label}.json"),
               "--boot", "1", "--cpu-profile", cpu, "--qemu-exit", str(rc),
               "--stability-verdict", "PASS" if semantic_pass else "FAIL",
               "--reason", "strict direct trial" if semantic_pass else "strict guest harness failed"]
    completed = subprocess.run(command, cwd=ROOT, text=True, stdout=subprocess.PIPE,
                               stderr=subprocess.STDOUT)
    if completed.stdout:
        (out / f"{label}-receipt-check.log").write_text(completed.stdout)
    return json.loads((out / f"{label}.json").read_text())


def run_strict(args: argparse.Namespace, manifest: dict, manifest_path: Path) -> int:
    out = args.receipts.resolve()
    create_new_dir(out)
    preflight, identity = run_preflight(args, manifest, out)
    env, prefix = runner_env(args)
    command = [sys.executable, str(ROOT / "tools/test_m15_guest.py")]
    run_meta = metadata(args, manifest, identity, prefix + command, "strict-two-profile")
    (out / "command.txt").write_text(
        f"PHASE15_PYTHON={sys.executable}\n"
        "ARENA_QEMU=" + shlex.join(prefix) + "\n"
        f"ARENA_OVMF_CODE={env['ARENA_OVMF_CODE']}\n"
        f"ARENA_OVMF_VARS={env['ARENA_OVMF_VARS']}\n"
        f"ARENA_PHASE15_EFI={env['ARENA_PHASE15_EFI']}\n"
        f"ARENA_PHASE15_ESP={env['ARENA_PHASE15_ESP']}\n"
        + shlex.join(command) + "\n")
    completed = subprocess.run(command, cwd=ROOT, env=env, text=True,
                               stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
    (out / "strict-run.log").write_text(completed.stdout)
    parsed: dict[str, int | None] = {}
    for label in PROFILE_LABELS:
        match = re.search(rf"\[{re.escape(label)}\] (?:PASS|FAIL): QEMU process exited cleanly \(rc=(-?\d+)\)",
                          completed.stdout)
        parsed[label] = int(match.group(1)) if match else None
    harness_pass = completed.returncode == 0 and "Phase-15 native guest qualification: PASS" in completed.stdout
    receipts = []
    for label in PROFILE_LABELS:
        profile_pass = (
            completed.returncode == 0
            and f"[{label}] PASS: guest semantic result and harness checks passed (verdict=0)" in completed.stdout
            and f"[{label}] PASS: all Phase-15 guest cases passed" in completed.stdout
            and f"[{label}] PASS: kernel reached declared clean shutdown" in completed.stdout
            and f"[{label}] PASS: every reported historical and Phase-15 result passed" in completed.stdout
        )
        receipts.append(save_boot_receipt(
            args, out, label, ROOT / "build" / f"serial-{label}.log",
            ROOT / "build" / f"qemu-stderr-{label}.log", parsed[label], profile_pass,
            manifest_path))
    passed = harness_pass and all(item["qualification_pass"] for item in receipts)
    run_meta.update({
        "preflight_pass": bool(preflight["accelerator_initialized"]),
        "harness_process_exit": completed.returncode,
        "strict_guest_verdict": "PASS" if passed else "FAIL",
        "qemu_process_exits": parsed,
        "profile_receipts": [{
                                  "checks": item["checks"],
                                  "profile": item["cpu_profile"],
                                  "qualification_pass": item["qualification_pass"],
                                  "qemu_exit_status": item["qemu_exit_status"],
                                  "semantic_guest_result": item["semantic_guest_result"],
                                  "serial_log": item["serial_log"],
                                  "qemu_stderr_log": item["qemu_stderr_log"]}
                             for item in receipts],
        "run_log_sha256": sha256(out / "strict-run.log"),
        "qualification_pass": passed,
    })
    (out / "run.json").write_text(json.dumps(run_meta, indent=2, sort_keys=True) + "\n")
    print(completed.stdout, end="")
    print(f"receipt directory: {out}")
    return 0 if passed else 1


def run_stability_tier(args: argparse.Namespace, manifest: dict,
                       manifest_path: Path, tier: str, out: Path) -> tuple[int, dict]:
    count = TIERS[tier]
    create_new_dir(out)
    preflight, identity = run_preflight(args, manifest, out)
    env, prefix = runner_env(args)
    env.update({
        "PHASE15_STRICT_GATE": "1",
        "PHASE15_PYTHON": sys.executable,
        "PHASE15_STRICT_EFI": str(args.efi.resolve()),
        "PHASE15_STRICT_ESP": str(args.esp.resolve()),
        "PHASE15_RECEIPTS_DIR": str(out),
        "PHASE15_QEMU_BINARY": str(args.qemu_binary.resolve()),
        "PHASE15_QEMU_SHA256": manifest["emulator"]["binary_sha256"],
    })
    command = ["bash", str(ROOT / "tools/stability_loop.sh"), str(count)]
    run_meta = metadata(args, manifest, identity, prefix + command, f"strict-{tier.lower()}")
    run_meta.update({"tier": tier, "required_boots": count,
                     "cpu_profile": "qemu64,+nx,+smep,+smap",
                     "preflight_pass": bool(preflight["accelerator_initialized"])})
    (out / "command.txt").write_text(
        f"PHASE15_PYTHON={sys.executable}\n"
        "ARENA_QEMU=" + shlex.join(prefix) + "\n" +
        f"ARENA_OVMF_CODE={env['ARENA_OVMF_CODE']}\n" +
        f"ARENA_OVMF_VARS={env['ARENA_OVMF_VARS']}\n" +
        f"PHASE15_STRICT_EFI={env['PHASE15_STRICT_EFI']}\n" +
        f"PHASE15_STRICT_ESP={env['PHASE15_STRICT_ESP']}\n" +
        shlex.join(command) + "\n")
    completed = subprocess.run(command, cwd=ROOT, env=env, text=True,
                               stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
    (out / "stability-run.log").write_text(completed.stdout)
    receipts = sorted((out / "boots").glob("boot-*.json")) if (out / "boots").is_dir() else []
    per_boot = [json.loads(path.read_text()) for path in receipts]
    pass_count = sum(bool(item.get("qualification_pass")) for item in per_boot)
    passed = (completed.returncode == 0 and pass_count == count and len(per_boot) == count
              and "STABILITY: PASS" in completed.stdout)
    run_meta.update({
        "harness_process_exit": completed.returncode,
        "attempted_boots": len(per_boot),
        "passed_boots": pass_count,
        "failed_boots": len(per_boot) - pass_count,
        "timeouts": sum(item.get("qemu_exit_status") == 124 for item in per_boot),
        "retries": 0,
        "stability_verdict": "PASS" if passed else "FAIL",
        "stability_log_sha256": sha256(out / "stability-run.log"),
        "per_boot_receipts": [path.relative_to(out).as_posix() for path in receipts],
        "qualification_pass": passed,
    })
    (out / "run.json").write_text(json.dumps(run_meta, indent=2, sort_keys=True) + "\n")
    print(completed.stdout, end="")
    print(f"strict {tier} receipts: {out} ({pass_count}/{count})")
    return (0 if passed else 1), run_meta


def verify_receipts(path: Path, manifest: dict) -> dict:
    run_path = path / "run.json"
    if not run_path.is_file():
        raise SystemExit(f"missing machine-readable run receipt: {run_path}")
    run = json.loads(run_path.read_text())
    if run.get("backend_id") != manifest["backend_id"]:
        raise SystemExit("receipt identifies a different emulator backend")
    if run.get("emulator") != manifest["emulator"]["binary_sha256"]:
        raise SystemExit("receipt emulator hash does not match the approved patched QEMU")
    if run.get("sha256", {}).get("efi") != manifest["strict_arenaos_image"]["efi_sha256"]:
        raise SystemExit("receipt EFI hash does not match the strict EFI")
    if run.get("sha256", {}).get("esp") != manifest["strict_arenaos_image"]["esp_sha256"]:
        raise SystemExit("receipt ESP hash does not match the strict ESP")
    if run.get("accelerator") != "tcg (explicit -accel tcg)":
        raise SystemExit("receipt does not prove explicit TCG selection")
    expected_scripts = {
        "test_m15_guest.py": sha256(ROOT / "tools/test_m15_guest.py"),
        "mtest.py": sha256(ROOT / "tools/mtest.py"),
        "stability_loop.sh": sha256(ROOT / "tools/stability_loop.sh"),
        "phase15_boot_receipt.py": sha256(ROOT / "tools/phase15_patched_tcg_boot_receipt.py"),
    }
    if run.get("qualification_script_sha256") != expected_scripts:
        raise SystemExit("receipt qualification-script hashes do not match the checked source")
    if run.get("run_kind") == "strict-two-profile":
        if not run.get("preflight_pass") or run.get("strict_guest_verdict") != "PASS":
            raise SystemExit("strict run is missing a passing explicit-TCG preflight or guest verdict")
        profiles = {entry.get("profile"): entry for entry in run.get("profile_receipts", [])}
        if set(profiles) != set(PROFILE_LABELS.values()):
            raise SystemExit("strict receipt is missing one or both required CPU profiles")
        if not run.get("qualification_pass") or any(not p.get("qualification_pass") for p in profiles.values()):
            raise SystemExit("strict guest evidence does not pass both CPU profiles")
        for label in PROFILE_LABELS:
            entry = profiles[PROFILE_LABELS[label]]
            if entry.get("qemu_exit_status") != 0:
                raise SystemExit(f"strict QEMU process did not exit zero for {label}")
            if entry.get("semantic_guest_result") != "m15: RESULT PASS (6/6)":
                raise SystemExit(f"strict semantic guest result did not pass for {label}")
            if not entry.get("checks") or not all(entry["checks"].values()):
                raise SystemExit(f"strict receipt has a failed containment or guest check for {label}")
            for log_key in ("serial_log", "qemu_stderr_log"):
                log = entry.get(log_key, {})
                file = path / log.get("path", "")
                if not file.is_file() or sha256(file) != log.get("sha256"):
                    raise SystemExit(f"missing or changed raw strict evidence: {file}")
        strict_log = path / "strict-run.log"
        if (not strict_log.is_file()
                or sha256(strict_log) != run.get("run_log_sha256")):
            raise SystemExit(f"missing or changed strict runner log: {strict_log}")
    elif run.get("run_kind", "").startswith("strict-t"):
        if not run.get("qualification_pass"):
            raise SystemExit("stability gate receipt is not PASS")
        tier = run.get("tier")
        if tier not in TIERS or run.get("required_boots") != TIERS[tier]:
            raise SystemExit("stability tier boot count does not match the official T2/T3/T4 requirement")
        if (run.get("cpu_profile") != "qemu64,+nx,+smep,+smap"
                or run.get("attempted_boots") != TIERS[tier]
                or run.get("passed_boots") != TIERS[tier]
                or run.get("failed_boots") != 0
                or run.get("timeouts") != 0
                or run.get("retries") != 0):
            raise SystemExit("stability receipt has an incomplete profile, failed attempt, timeout, or retry")
        receipts = [path / name for name in run.get("per_boot_receipts", [])]
        if len(receipts) != run.get("required_boots"):
            raise SystemExit("stability receipt count does not match the required tier")
        for receipt_path in receipts:
            if not receipt_path.is_file():
                raise SystemExit(f"missing per-boot receipt: {receipt_path}")
            item = json.loads(receipt_path.read_text())
            if not item.get("qualification_pass"):
                raise SystemExit(f"failed strict per-boot receipt: {receipt_path}")
            if (item.get("backend_id") != manifest["backend_id"]
                    or item.get("emulator") != manifest["emulator"]["binary_sha256"]
                    or item.get("accelerator") != "tcg (explicit -accel tcg)"
                    or item.get("cpu_profile") != run["cpu_profile"]
                    or item.get("efi_sha256") != manifest["strict_arenaos_image"]["efi_sha256"]
                    or item.get("esp_sha256") != manifest["strict_arenaos_image"]["esp_sha256"]
                    or item.get("qemu_exit_status") != 0
                    or item.get("semantic_guest_result") != "m15: RESULT PASS (6/6)"
                    or item.get("fault_status") != "0x113"
                    or not item.get("vector_19_instruction_generated")
                    or not item.get("checks") or not all(item["checks"].values())):
                raise SystemExit(f"strict per-boot receipt has mismatched identity or missing Phase-15 proof: {receipt_path}")
            serial = path / "boots" / item["serial_log"]["path"]
            stderr = path / "boots" / item["qemu_stderr_log"]["path"]
            if not serial.is_file() or sha256(serial) != item["serial_log"]["sha256"]:
                raise SystemExit(f"missing or changed serial evidence: {serial}")
            if not stderr.is_file() or sha256(stderr) != item["qemu_stderr_log"]["sha256"]:
                raise SystemExit(f"missing or changed QEMU stderr evidence: {stderr}")
        stability_log = path / "stability-run.log"
        if (not stability_log.is_file()
                or sha256(stability_log) != run.get("stability_log_sha256")):
            raise SystemExit(f"missing or changed strict stability log: {stability_log}")
    else:
        raise SystemExit(f"unsupported receipt kind: {run.get('run_kind')}")
    return run


def add_inputs(parser: argparse.ArgumentParser) -> None:
    parser.add_argument("--manifest", type=Path, default=MANIFEST_DEFAULT)
    parser.add_argument("--qemu-binary", type=Path, required=True)
    parser.add_argument("--lib-dir", type=Path, required=True,
                        help="directory containing the pinned QEMU runtime .so files")
    parser.add_argument("--rom-dir", type=Path, required=True,
                        help="official QEMU pc-bios ROM directory")
    parser.add_argument("--efi", type=Path, required=True)
    parser.add_argument("--esp", type=Path, required=True)
    parser.add_argument("--ovmf-code", type=Path, required=True)
    parser.add_argument("--ovmf-vars", type=Path, required=True)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="action", required=True)
    for action in ("preflight", "strict"):
        child = sub.add_parser(action)
        add_inputs(child)
        if action == "strict":
            child.add_argument("--receipts", type=Path, required=True)
    stability = sub.add_parser("stability")
    add_inputs(stability)
    stability.add_argument("tier", choices=(*TIERS, "all"))
    stability.add_argument("--receipts", type=Path, required=True)
    verify = sub.add_parser("verify")
    verify.add_argument("receipt_directory", type=Path)
    verify.add_argument("--manifest", type=Path, default=MANIFEST_DEFAULT)
    args = parser.parse_args()
    manifest = read_manifest(args.manifest)

    if args.action == "verify":
        result = verify_receipts(args.receipt_directory.resolve(), manifest)
        print(f"verified {result['run_kind']} evidence; qualification_pass={result['qualification_pass']}")
        return 0
    if args.action == "preflight":
        result, _ = run_preflight(args, manifest)
        print(json.dumps(result, indent=2, sort_keys=True))
        return 0
    if args.action == "strict":
        return run_strict(args, manifest, args.manifest.resolve())

    root = args.receipts.resolve()
    create_new_dir(root)
    tiers = list(TIERS) if args.tier == "all" else [args.tier]
    results = []
    overall = 0
    for tier in tiers:
        result, receipt = run_stability_tier(
            args, manifest, args.manifest.resolve(), tier, root / tier)
        overall |= result
        results.append({"tier": tier, "verdict": receipt["stability_verdict"],
                        "attempted": receipt["attempted_boots"],
                        "passed": receipt["passed_boots"],
                        "directory": tier})
    summary = {"backend_id": manifest["backend_id"],
               "qualification_pass": overall == 0, "tiers": results}
    (root / "run.json").write_text(json.dumps(summary, indent=2, sort_keys=True) + "\n")
    print(json.dumps(summary, indent=2, sort_keys=True))
    return overall


if __name__ == "__main__":
    raise SystemExit(main())
