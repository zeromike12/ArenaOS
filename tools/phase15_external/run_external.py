#!/usr/bin/env python3
"""Run the unchanged strict M15 guest test on an explicit hardware backend."""
from __future__ import annotations

import argparse
import contextlib
import hashlib
import importlib
import json
import os
import platform
import queue
import re
import shlex
import shutil
import socket
import subprocess
import sys
import threading
import time
from datetime import datetime, timezone
from pathlib import Path
from typing import Any


ROOT = Path(__file__).resolve().parents[2]
EFI_SHA = "bbbc63df271c3d7d5109789369f72e093b02ef77c77ebe25273b6da4c00d238e"
ESP_SHA = "631cc6452ecbacf58b2418e54b9c509bb717cc2211580ebef63f9aa4c13d866f"
XF_SHA = "1f057068b020baefe06f7f3b36bc0beeafcb879bd93d60c5ea66a3cce616c8d3"
XF_SOURCE_SHA = "f59c35ed5d5bbe810a9baad573418b9a64a385e2330b23627ebfc7bffa4a9c3a"
OVMF_CODE_SHA = "7e34e3b83224968bb0714c0ebc88f75c5c37b4cc892c902e511ccc7a7c107f94"
OVMF_VARS_SHA = "5d2ac383371b408398accee7ec27c8c09ea5b74a0de0ceea6513388b15be5d1e"
CPU_RUNS = (
    ("phase15-avx", "max,+nx,+smep,+smap", True),
    ("phase15-qemu64", "qemu64,+nx,+smep,+smap", False),
)


def sha256(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            h.update(block)
    return h.hexdigest()


def file_record(path: Path) -> dict[str, Any]:
    return {"path": str(path), "bytes": path.stat().st_size,
            "sha256": sha256(path)}


def output_file_record(path: Path, out: Path) -> dict[str, Any]:
    record = file_record(path)
    record["relative_path"] = path.resolve().relative_to(out.resolve()).as_posix()
    return record


def default_artifact(filename: str, build_name: str) -> Path:
    candidates = (ROOT / "artifacts" / filename, ROOT / "build" / build_name)
    return next((p for p in candidates if p.is_file()), candidates[1])


def default_qemu() -> str:
    return (os.environ.get("QEMU_SYSTEM_X86_64")
            or os.environ.get("ARENA_QEMU")
            or "qemu-system-x86_64")


def qemu_prefix(command: str) -> list[str]:
    direct = Path(command).expanduser()
    if direct.is_file():
        return [str(direct.resolve())]
    parts = shlex.split(command)
    if not parts:
        raise ValueError("QEMU command is empty")
    if "-accel" in parts:
        raise ValueError("do not put -accel in --qemu; the runner selects it explicitly")
    return parts


def qemu_executable(prefix: list[str]) -> Path | None:
    for token in reversed(prefix):
        candidate = Path(token)
        if candidate.is_file():
            return candidate.resolve()
        found = shutil.which(token)
        if found:
            return Path(found).resolve()
    return None


def output_directory(path: str | None) -> Path:
    if path:
        result = Path(path).expanduser().resolve()
    else:
        stamp = datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%S%fZ")
        result = ROOT / "build" / f"phase15-external-{stamp}"
    result.mkdir(parents=True, exist_ok=False)
    (result / "artifacts").mkdir()
    return result


def source_identity(root: Path) -> dict[str, Any]:
    record: dict[str, Any] = {"root": str(root), "commit": None,
                              "branch": None, "working_tree_clean": None}
    if (root / ".git").exists():
        def git(*args: str) -> str | None:
            result = subprocess.run(["git", "-C", str(root), *args],
                                    text=True, capture_output=True)
            return result.stdout.strip() if result.returncode == 0 else None
        record["commit"] = git("rev-parse", "HEAD")
        record["branch"] = git("branch", "--show-current")
        status = git("status", "--porcelain")
        record["working_tree_clean"] = status == "" if status is not None else None
    else:
        kit = root / "KIT-MANIFEST.json"
        if kit.is_file():
            try:
                manifest = json.loads(kit.read_text())
                record["commit"] = manifest.get("source_commit")
                record["branch"] = manifest.get("source_branch")
                record["working_tree_clean"] = manifest.get("source_clean")
                record["kit_manifest_sha256"] = sha256(kit)
            except (OSError, json.JSONDecodeError):
                pass
    return record


def verify_kit_manifest(root: Path) -> tuple[dict[str, Any] | None, list[str]]:
    manifest_path = root / "KIT-MANIFEST.json"
    if not manifest_path.is_file():
        return None, []
    errors: list[str] = []
    try:
        manifest = json.loads(manifest_path.read_text())
        if manifest.get("format") != "arenaos.phase15.external-kit.v2":
            errors.append("external kit manifest has an unknown format")
        identities = manifest.get("strict_artifact_identities", {})
        expected = {"efi_sha256": EFI_SHA, "esp_sha256": ESP_SHA,
                    "xf_probe_sha256": XF_SHA}
        if identities != expected:
            errors.append("external kit strict artifact identities do not match the accepted pins")
        for relative, record in manifest.get("files", {}).items():
            path = root / relative
            if not path.resolve().is_relative_to(root.resolve()):
                errors.append(f"kit manifest path escapes the kit root: {relative}")
            elif not path.is_file():
                errors.append(f"kit manifest file is missing: {relative}")
            elif path.stat().st_size != record.get("bytes") or sha256(path) != record.get("sha256"):
                errors.append(f"kit manifest hash mismatch: {relative}")
    except (OSError, json.JSONDecodeError, AttributeError) as error:
        errors.append(f"could not read kit manifest: {error}")
        manifest = None
    return manifest, errors


def copy_source_review(root: Path, out: Path) -> dict[str, Any]:
    entries = (
        ("tools/test_m15_guest.py", (root / "tools/test_m15_guest.py",)),
        ("tools/phase15_external/run_external.py", (root / "tools/phase15_external/run_external.py",)),
        ("kernel/kernel/src/m15.rs", (
            root / "kernel/kernel/src/m15.rs", root / "source-review/kernel/kernel/src/m15.rs")),
        ("kernel/kernel/src/arch/x86_64/idt.rs", (
            root / "kernel/kernel/src/arch/x86_64/idt.rs",
            root / "source-review/kernel/kernel/src/arch/x86_64/idt.rs")),
        ("kernel/kernel/src/elf.rs", (
            root / "kernel/kernel/src/elf.rs", root / "source-review/kernel/kernel/src/elf.rs")),
        ("userspace/phase15-fpu/src/xf_probe.rs", (
            root / "userspace/phase15-fpu/src/xf_probe.rs",
            root / "source-review/userspace/phase15-fpu/src/xf_probe.rs")),
    )
    records: dict[str, Any] = {}
    for relative, candidates in entries:
        source = next((candidate for candidate in candidates if candidate.is_file()), None)
        if source is None:
            continue
        dest = out / "source-review" / relative
        dest.parent.mkdir(parents=True, exist_ok=True)
        if source.resolve() != dest.resolve():
            shutil.copyfile(source, dest)
        records[relative] = output_file_record(dest, out)
    manifest = root / "KIT-MANIFEST.json"
    if manifest.is_file():
        dest = out / "source-review/KIT-MANIFEST.json"
        shutil.copyfile(manifest, dest)
        records["KIT-MANIFEST.json"] = output_file_record(dest, out)
    return records


def host_identity() -> dict[str, Any]:
    result: dict[str, Any] = {
        "platform": platform.platform(),
        "system": platform.system(),
        "release": platform.release(),
        "machine": platform.machine(),
        "processor": platform.processor(),
        "logical_cpu_count": os.cpu_count(),
    }
    if Path("/proc/cpuinfo").is_file():
        for line in Path("/proc/cpuinfo").read_text(errors="replace").splitlines():
            if line.startswith("flags\t:") or line.startswith("Features\t:"):
                flags = set(line.split(":", 1)[1].split())
                result["host_cpu_flags"] = sorted(flags)
                result["host_has_fpu_sse_sse2"] = all(
                    flag in flags for flag in ("fpu", "sse", "sse2"))
                result["host_has_avx"] = "avx" in flags
                result["host_virtualization_flags"] = sorted(
                    flags.intersection({"vmx", "svm"}))
                break
    return result


def qemu_capture(prefix: list[str], args: list[str], timeout: float = 20.0) -> tuple[int, str]:
    result = subprocess.run(prefix + args, text=True, capture_output=True,
                            errors="replace", timeout=timeout)
    return result.returncode, (result.stdout or "") + (result.stderr or "")


def _qmp_line(lines: queue.Queue[str | None], proc: subprocess.Popen[str],
              timeout: float, transcript: list[str]) -> dict[str, Any]:
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if proc.poll() is not None and lines.empty():
            break
        try:
            line = lines.get(timeout=max(0.01, min(0.2, deadline - time.monotonic())))
        except queue.Empty:
            continue
        if line is None:
            break
        transcript.append(line)
        try:
            message = json.loads(line)
        except json.JSONDecodeError:
            continue
        if "return" in message or "error" in message or "QMP" in message:
            return message
    raise RuntimeError("QEMU accelerator probe did not answer QMP; output="
                       + "".join(transcript)[-2000:])


def probe_accelerator(prefix: list[str], backend: str) -> dict[str, Any]:
    """Initialize the requested accelerator and query its live QEMU state."""
    cmd = prefix + [
        "-accel", backend, "-machine", "q35", "-cpu", "max", "-m", "128M",
        "-nodefaults", "-display", "none", "-serial", "none", "-monitor", "none",
        "-S", "-qmp", "stdio", "-no-reboot",
    ]
    proc = subprocess.Popen(cmd, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                           stderr=subprocess.STDOUT, text=True, encoding="utf-8",
                           errors="replace", bufsize=1)
    lines: queue.Queue[str | None] = queue.Queue()
    transcript: list[str] = []

    def reader() -> None:
        assert proc.stdout is not None
        for line in proc.stdout:
            lines.put(line)
        lines.put(None)

    thread = threading.Thread(target=reader, daemon=True)
    thread.start()

    def command(name: str) -> dict[str, Any]:
        assert proc.stdin is not None
        proc.stdin.write(json.dumps({"execute": name}) + "\n")
        proc.stdin.flush()
        while True:
            reply = _qmp_line(lines, proc, 15.0, transcript)
            if "error" in reply:
                raise RuntimeError(f"QMP {name} failed: {reply['error']}")
            if "return" in reply:
                return reply["return"]

    evidence: dict[str, Any] = {"command": cmd, "qemu_exit_status": None,
                                "qmp_greeting": False, "cpu_count": 0}
    try:
        greeting = _qmp_line(lines, proc, 20.0, transcript)
        if "QMP" not in greeting:
            raise RuntimeError("QEMU did not provide a QMP greeting")
        evidence["qmp_greeting"] = True
        assert proc.stdin is not None
        proc.stdin.write(json.dumps({"execute": "qmp_capabilities"}) + "\n")
        proc.stdin.flush()
        while True:
            reply = _qmp_line(lines, proc, 15.0, transcript)
            if "error" in reply:
                raise RuntimeError(f"QMP capabilities failed: {reply['error']}")
            if "return" in reply:
                break
        cpus = command("query-cpus-fast")
        evidence["cpu_count"] = len(cpus) if isinstance(cpus, list) else 0
        if evidence["cpu_count"] < 1:
            raise RuntimeError("accelerator initialized without a queryable vCPU")
        if backend == "kvm":
            kvm = command("query-kvm")
            evidence["query_kvm"] = kvm
            if kvm.get("present") is not True or kvm.get("enabled") is not True:
                raise RuntimeError(f"QMP reports KVM inactive: {kvm}")
        else:
            evidence["explicit_accelerator"] = backend
        command("quit")
        evidence["qemu_exit_status"] = proc.wait(timeout=10)
        if evidence["qemu_exit_status"] != 0:
            raise RuntimeError(f"accelerator probe QEMU exited {evidence['qemu_exit_status']}")
        evidence["probe_output"] = "".join(transcript)
        evidence["initialized"] = True
        return evidence
    except Exception as error:
        if proc.poll() is None:
            proc.kill()
        try:
            evidence["qemu_exit_status"] = proc.wait(timeout=5)
        except subprocess.TimeoutExpired:
            evidence["qemu_exit_status"] = None
        evidence["probe_output"] = "".join(transcript)
        evidence["initialized"] = False
        raise AcceleratorProbeError(str(error), evidence) from error


def verify_embedded_efi(esp: Path, efi: Path) -> bool:
    from pyfatfs.PyFatFS import PyFatFS

    fs = PyFatFS(str(esp), read_only=True)
    try:
        return fs.getbytes("/EFI/BOOT/BOOTX64.EFI") == efi.read_bytes()
    finally:
        fs.close()


def choose_firmware(args: argparse.Namespace) -> tuple[Path, Path]:
    code = Path(args.ovmf_code).expanduser().resolve() if args.ovmf_code else None
    vars_path = Path(args.ovmf_vars).expanduser().resolve() if args.ovmf_vars else None
    if code is None or vars_path is None:
        bundled_code = ROOT / "artifacts/OVMF_CODE_4M.fd"
        bundled_vars = ROOT / "artifacts/OVMF_VARS_4M.fd"
        if bundled_code.is_file() and bundled_vars.is_file():
            code = code or bundled_code
            vars_path = vars_path or bundled_vars
        elif os.name == "nt":
            raise FileNotFoundError(
                "bundled OVMF is missing; Windows requires --ovmf-code and --ovmf-vars")
    if code is None or vars_path is None:
        sys.path.insert(0, str(ROOT / "tools"))
        import arena_env

        try:
            code = code or arena_env.ovmf_code()
            vars_path = vars_path or arena_env.ovmf_vars_template()
        except SystemExit as error:
            raise FileNotFoundError(str(error)) from error
    if not code.is_file() or not vars_path.is_file():
        raise FileNotFoundError(f"OVMF code/vars files must exist: {code}, {vars_path}")
    return code, vars_path


class AcceleratorProbeError(RuntimeError):
    def __init__(self, message: str, evidence: dict[str, Any]):
        super().__init__(message)
        self.evidence = evidence


def classify_trial(serial: str, qemu_exit: int | None,
                   semantic_exit: int | None,
                   harness_passed: bool) -> dict[str, Any]:
    arena_booted = "[arena INFO  boot] ArenaOS boot stage" in serial
    firmware_started = arena_booted or "BdsDxe: starting Boot" in serial
    fault_receipt = re.search(r"minimal #XF probe receipts: fault=Some\(([^)]+)\), "
                              r"sibling=Some\(([^)]+)\), peer=Some\(([^)]+)\), "
                              r"sibling_and_peer_survived=(true|false), "
                              r"fault_slot_released=(true|false), "
                              r"resource_baseline_restored=(true|false)", serial)
    vector19 = re.search(
        r"\[arena user fault\] pid=\d+ vector=0x13 \(#XF SIMD Floating Point\)",
        serial) is not None
    expected_fault = bool(fault_receipt and fault_receipt.group(1).lower() == "113")
    both_survived = bool(fault_receipt and fault_receipt.group(2).lower() == "2a"
                         and fault_receipt.group(3).lower() == "2a"
                         and fault_receipt.group(4) == "true")
    sibling_survived = both_survived
    peer_survived = both_survived
    slot_released = bool(fault_receipt and fault_receipt.group(5) == "true")
    resources_restored = bool(fault_receipt and fault_receipt.group(6) == "true")
    xf_attempted = "m15:test:xf_delivery_probe:" in serial
    historical_complete = all(marker in serial for marker in (
        "m3: RESULT PASS (13/13)", "m4: RESULT PASS (9/9)"))
    clean_shutdown = "halting via UEFI ResetSystem(shutdown)" in serial
    kernel_continued = "m15:test:mf_ud_fault_containment: PASS" in serial
    containment_ok = all((expected_fault, sibling_survived, peer_survived,
                          slot_released, resources_restored))
    if qemu_exit not in (0, None) and not serial:
        boundary = "qemu_process_or_firmware_startup"
    elif not firmware_started:
        boundary = "firmware_boot_not_observed"
    elif not arena_booted:
        boundary = "firmware_did_not_transfer_to_arenaos"
    elif not xf_attempted:
        boundary = "arenaos_did_not_reach_xf_probe"
    elif not vector19:
        boundary = "vector_19_delivery"
    elif not containment_ok:
        boundary = "exception_containment"
    elif not (historical_complete and kernel_continued and clean_shutdown
              and harness_passed and qemu_exit == 0 and semantic_exit == 0):
        boundary = "other_strict_guest_assertion"
    else:
        boundary = "complete"
    return {
        "qemu_process_exit": "PASS" if qemu_exit == 0 else "FAIL",
        "semantic_guest_exit": "PASS" if semantic_exit == 0 else "FAIL",
        "firmware_boot": "PASS" if firmware_started else "NOT_OBSERVED",
        "arenaos_boot": "PASS" if arena_booted else "NOT_OBSERVED",
        "historical_guest_suites": ("PASS" if historical_complete else
                                    "FAIL" if arena_booted else "NOT_OBSERVED"),
        "xf_probe_attempted": xf_attempted,
        "vector_19_delivery": ("PASS" if vector19 else
                               "FAIL" if xf_attempted else "NOT_REACHED"),
        "faulting_thread_terminated_with_0x113": expected_fault,
        "same_process_sibling_survived": sibling_survived,
        "independent_process_survived": peer_survived,
        "scheduler_slot_reclaimed": slot_released,
        "resource_baseline_restored": resources_restored,
        "containment": ("PASS" if all((expected_fault, sibling_survived,
                                        peer_survived, slot_released,
                                        resources_restored)) else
                        "FAIL" if xf_attempted else "NOT_REACHED"),
        "kernel_continued_operation": ("PASS" if kernel_continued else
                                       "FAIL" if xf_attempted else "NOT_OBSERVED"),
        "clean_shutdown": "PASS" if clean_shutdown else
                         "FAIL" if arena_booted else "NOT_OBSERVED",
        "failure_boundary": boundary,
        "strict_harness": "PASS" if harness_passed else "FAIL",
        "trial": ("PASS" if qemu_exit == 0 and semantic_exit == 0
                  and harness_passed and vector19 and expected_fault
                  and sibling_survived and peer_survived and slot_released
                  and resources_restored and historical_complete
                  and kernel_continued and clean_shutdown
                  else "FAIL"),
    }


def strict_xf_evidence(serial: str) -> dict[str, bool]:
    vector = re.search(
        r"\[arena user fault\] pid=\d+ vector=0x13 \(#XF SIMD Floating Point\)",
        serial)
    receipt = re.search(
        r"minimal #XF probe receipts: fault=Some\(113\), sibling=Some\(2a\), "
        r"peer=Some\(2a\), sibling_and_peer_survived=true, "
        r"fault_slot_released=true, resource_baseline_restored=true", serial)
    cpu = re.search(
        r"feature discovery after FP setup:.*\bfpu=true\b.*\bsse=true\b.*\bsse2=true\b",
        serial)
    results = re.findall(r"^(m\d+): RESULT (PASS|FAIL) \((\d+)/(\d+)\)$",
                         serial, re.MULTILINE)
    return {
        "ring3_vector_19_logged": vector is not None,
        "fault_exit_status_0x113": receipt is not None,
        "sibling_and_independent_peer_survived": receipt is not None,
        "fault_scheduler_slot_released": receipt is not None,
        "resource_baselines_restored": receipt is not None,
        "guest_reports_fpu_sse_sse2": cpu is not None,
        "strict_xf_guest_pass_marker": serial.count("m15:test:xf_delivery_probe: PASS") == 1,
        "strict_m15_pass_result": bool(re.search(r"^m15: RESULT PASS \(\d+/\d+\)$",
                                                  serial, re.MULTILINE)),
        "reported_results_all_pass": bool(results) and all(
            result == "PASS" for _, result, _, _ in results),
        "clean_uefi_shutdown": "halting via UEFI ResetSystem(shutdown)" in serial,
        "no_panic_or_failed_result": "PANIC" not in serial and "RESULT FAIL" not in serial,
    }


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--backend", required=True, choices=("kvm", "whpx"))
    parser.add_argument("--qemu", default=default_qemu(),
                        help="QEMU executable or shell-quoted command prefix")
    parser.add_argument("--efi", default=str(default_artifact(
        "arena-boot.efi", "phase15-strict-efi-6586a0a.efi")))
    parser.add_argument("--esp", default=str(default_artifact(
        "arena-esp.img", "phase15-strict-esp-6586a0a.img")))
    parser.add_argument("--xf-probe", default=None)
    parser.add_argument("--ovmf-code", default=None)
    parser.add_argument("--ovmf-vars", default=None)
    parser.add_argument("--output-dir", default=None)
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    out = output_directory(args.output_dir)
    status = "BLOCKED"
    reasons: list[str] = []
    receipt: dict[str, Any] = {
        "schema": "arenaos.phase15.external-qualification.v1",
        "created_utc": datetime.now(timezone.utc).isoformat(),
        "status": status,
        "backend_requested": args.backend,
        "source": source_identity(ROOT),
        "host": host_identity(),
        "strict_acceptance": {
            "efi_expected_sha256": EFI_SHA,
            "esp_expected_sha256": ESP_SHA,
            "xf_probe_expected_sha256": XF_SHA,
            "ovmf_code_expected_sha256": OVMF_CODE_SHA,
            "ovmf_vars_expected_sha256": OVMF_VARS_SHA,
            "strict_image_build_source_commit": "6586a0a18981932ae7048ca267fa425411f8e463",
            "required_cpu_profiles": [
                {"label": label, "cpu": cpu, "avx_expected": avx}
                for label, cpu, avx in CPU_RUNS
            ],
            "attempt_count": len(CPU_RUNS),
            "retry_count": 0,
        },
        "images": {}, "qemu": {}, "firmware": {},
        "accelerator_probe": {"initialized": False},
        "trials": [], "issues": reasons,
        "qualification_claim": "external strict guest trial only; no T2/T3/T4 or release qualification",
    }
    kit_manifest, kit_errors = verify_kit_manifest(ROOT)
    if kit_manifest is not None:
        receipt["kit_manifest"] = {
            "format": kit_manifest.get("format"),
            "source_commit": kit_manifest.get("source_commit"),
            "file_count": len(kit_manifest.get("files", {})),
            "sha256": sha256(ROOT / "KIT-MANIFEST.json"),
            "verified": not kit_errors,
        }
    if kit_errors:
        reasons.extend(kit_errors)
        status = "FAIL"
    receipt["source_review_files"] = copy_source_review(ROOT, out)
    if "tools/test_m15_guest.py" not in receipt["source_review_files"]:
        reasons.append("existing strict Phase 15 guest test source is missing")
        status = "FAIL"

    efi = Path(args.efi).expanduser().resolve()
    esp = Path(args.esp).expanduser().resolve()
    if not efi.is_file() or not esp.is_file():
        reasons.append("strict EFI/ESP input file is missing")
        status = "FAIL"
    else:
        for source, name in ((efi, "arena-boot.efi"), (esp, "arena-esp.img")):
            dest = out / "artifacts" / name
            shutil.copyfile(source, dest)
            receipt["images"][name] = output_file_record(dest, out)
        receipt["images"]["efi_expected_sha256"] = EFI_SHA
        receipt["images"]["esp_expected_sha256"] = ESP_SHA
        if sha256(efi) != EFI_SHA or sha256(esp) != ESP_SHA:
            reasons.append("strict image SHA-256 does not match the pinned acceptance identity")
            status = "FAIL"
        else:
            try:
                receipt["images"]["esp_embeds_exact_efi"] = verify_embedded_efi(esp, efi)
                if not receipt["images"]["esp_embeds_exact_efi"]:
                    reasons.append("ESP does not contain its paired exact EFI")
                    status = "FAIL"
            except Exception as error:  # recorded as a failed artifact preflight
                reasons.append(f"could not verify the ESP EFI entry: {error}")
                status = ("BLOCKED" if isinstance(error, ImportError)
                          and status != "FAIL" else "FAIL")

    xf_probe = (Path(args.xf_probe).expanduser().resolve() if args.xf_probe
                else ROOT / "userspace/phase15-fpu/target/x86_64-unknown-none/release/arena-phase15-xf-probe")
    if not xf_probe.is_file():
        packaged = ROOT / "artifacts/arena-phase15-xf-probe"
        xf_probe = packaged if packaged.is_file() else xf_probe
    if xf_probe.is_file():
        shutil.copyfile(xf_probe, out / "artifacts" / "arena-phase15-xf-probe")
        receipt["images"]["xf_probe"] = output_file_record(
            out / "artifacts/arena-phase15-xf-probe", out)
        if sha256(xf_probe) != XF_SHA:
            reasons.append("compiled CPL3 #XF fixture does not match its pinned SHA-256")
            status = "FAIL"
    else:
        reasons.append("compiled CPL3 #XF fixture is missing")
        status = "FAIL"

    source_candidates = (
        ROOT / "userspace/phase15-fpu/src/xf_probe.rs",
        ROOT / "source-review/userspace/phase15-fpu/src/xf_probe.rs",
    )
    xf_source = next((p for p in source_candidates if p.is_file()), None)
    if xf_source:
        source_text = xf_source.read_text()
        source_copy = out / "source-review/userspace/phase15-fpu/src/xf_probe.rs"
        if not source_copy.is_file():
            source_copy.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(xf_source, source_copy)
        receipt["xf_source"] = output_file_record(source_copy, out)
        receipt["xf_source"]["expected_sha256"] = XF_SOURCE_SHA
        required_source_tokens = (
            "CPL3", "ldmxcsr", "stmxcsr", "0x00001F00",
            "invalid-operation mask bit 7 clear", "0xBF800000", "sqrtss xmm0, xmm0")
        receipt["xf_source_assertions"] = {
            token: token in source_text for token in required_source_tokens
        }
        if sha256(xf_source) != XF_SOURCE_SHA:
            reasons.append("CPL3 #XF source does not match its pinned source revision")
            status = "FAIL"
        if not all(receipt["xf_source_assertions"].values()):
            reasons.append("CPL3 source fixture is missing the exact unmasked invalid SQRTSS sequence")
            status = "FAIL"
    else:
        reasons.append("source for the pinned CPL3 #XF fixture is missing from the kit")
        status = "FAIL"

    if args.backend == "kvm" and not Path("/dev/kvm").exists():
        reasons.append("/dev/kvm is absent; no KVM guest was started")
        status = "BLOCKED" if status != "FAIL" else status
    if args.backend == "kvm" and platform.system() != "Linux":
        reasons.append("the KVM runner requires a Linux host")
        status = "BLOCKED" if status != "FAIL" else status
    if args.backend == "whpx" and platform.system() != "Windows":
        reasons.append("the WHPX runner requires a Windows host")
        status = "BLOCKED" if status != "FAIL" else status
    if receipt["host"].get("machine", "").lower() not in ("x86_64", "amd64"):
        reasons.append("hardware qualification requires an x86-64 host CPU")
        status = "BLOCKED" if status != "FAIL" else status
    if args.backend == "kvm" and receipt["host"].get("host_has_fpu_sse_sse2") is False:
        reasons.append("host CPU flags do not include x87/SSE/SSE2")
        status = "BLOCKED" if status != "FAIL" else status
    if args.backend == "kvm" and receipt["host"].get("host_has_avx") is False:
        reasons.append("host CPU does not expose AVX required by the existing max-profile acceptance check")
        status = "BLOCKED" if status != "FAIL" else status

    prefix: list[str] = []
    try:
        prefix = qemu_prefix(args.qemu)
    except Exception as error:
        reasons.append(f"invalid QEMU command prefix: {error}")
        status = "BLOCKED" if status != "FAIL" else status
    qemu_path = qemu_executable(prefix) if prefix else None
    receipt["qemu"]["command_prefix"] = prefix
    receipt["qemu"]["strict_backend_args"] = ["-accel", args.backend]
    if qemu_path is None:
        reasons.append("QEMU executable could not be resolved")
        status = "BLOCKED" if status != "FAIL" else status
    else:
        prefix = qemu_prefix(args.qemu)
        receipt["qemu"]["binary"] = file_record(qemu_path)
        try:
            rc, version = qemu_capture(prefix, ["--version"])
            receipt["qemu"]["version_exit_status"] = rc
            receipt["qemu"]["version"] = version.strip()
            if rc != 0 or "QEMU emulator version" not in version:
                reasons.append("selected executable did not identify as a QEMU system emulator")
                status = "BLOCKED" if status != "FAIL" else status
            rc, accelerators = qemu_capture(prefix, ["-accel", "help"])
            receipt["qemu"]["accelerator_list_exit_status"] = rc
            receipt["qemu"]["accelerators"] = accelerators.strip()
            receipt["qemu"]["requested_accelerator_advertised"] = bool(
                re.search(rf"\b{re.escape(args.backend)}\b", accelerators, re.I))
            if not receipt["qemu"]["requested_accelerator_advertised"]:
                reasons.append(f"QEMU does not advertise the requested {args.backend} accelerator")
                status = "BLOCKED" if status != "FAIL" else status
        except Exception as error:
            reasons.append(f"QEMU identity/preflight command failed: {error}")
            status = "BLOCKED" if status != "FAIL" else status

    try:
        code, vars_path = choose_firmware(args)
        code_copy = out / "artifacts/OVMF_CODE_4M.fd"
        vars_copy = out / "artifacts/OVMF_VARS_4M.fd"
        shutil.copyfile(code, code_copy)
        shutil.copyfile(vars_path, vars_copy)
        receipt["firmware"] = {
            "code": output_file_record(code_copy, out),
            "vars_template": output_file_record(vars_copy, out),
            "bundled_code_identity_matches": sha256(code) == OVMF_CODE_SHA,
            "bundled_vars_identity_matches": sha256(vars_path) == OVMF_VARS_SHA,
        }
        firmware_source = ROOT / "FIRMWARE-SOURCE.json"
        if firmware_source.is_file():
            shutil.copyfile(firmware_source, out / "artifacts/FIRMWARE-SOURCE.json")
            receipt["firmware"]["source_record"] = output_file_record(
                out / "artifacts/FIRMWARE-SOURCE.json", out)
    except Exception as error:
        reasons.append(f"OVMF firmware preflight failed: {error}")
        status = "BLOCKED" if status != "FAIL" else status
        code = vars_path = None

    # Start no guest if any identity check or backend prerequisite failed.
    if status not in ("FAIL", "BLOCKED"):
        status = "BLOCKED"
    prerequisites_ok = (
        status == "BLOCKED" and not reasons
        and qemu_path is not None and code is not None and vars_path is not None
        and receipt.get("images", {}).get("esp_embeds_exact_efi") is True
        and receipt.get("images", {}).get("xf_probe", {}).get("sha256") == XF_SHA
    )
    if prerequisites_ok:
        try:
            receipt["accelerator_probe"] = probe_accelerator(prefix, args.backend)
            status = "FAIL"  # a boot trial must promote this only after every assertion passes
        except AcceleratorProbeError as error:
            receipt["accelerator_probe"] = error.evidence
            reasons.append(
                f"requested accelerator failed its QMP initialization probe: {error}")
            status = "BLOCKED"
        except Exception as error:
            receipt["accelerator_probe"] = {"initialized": False,
                                             "error": str(error)}
            reasons.append(f"requested accelerator failed its QMP initialization probe: {error}")
            status = "BLOCKED"

    if status == "FAIL" and receipt.get("accelerator_probe", {}).get("initialized"):
        actual_exits: dict[str, int | None] = {}
        qmp_states: list[dict[str, Any]] = []
        profile_checks: dict[str, bool] = {}
        logs_dir = out / "logs"
        logs_dir.mkdir(exist_ok=True)
        runner_log = out / "runner.log"
        try:
            code, vars_path = choose_firmware(args)
            staged_efi = out / "artifacts/arena-boot.efi"
            staged_esp = out / "artifacts/arena-esp.img"
            os.environ["ARENA_QEMU"] = shlex.join(prefix + ["-accel", args.backend])
            os.environ["ARENA_OVMF_CODE"] = str(code)
            os.environ["ARENA_OVMF_VARS"] = str(vars_path)
            os.environ["ARENA_PHASE15_EFI"] = str(staged_efi)
            os.environ["ARENA_PHASE15_ESP"] = str(staged_esp)
            if os.name == "nt":
                os.environ["ARENA_PHASE15_SOCKET_MODE"] = "tcp"

            sys.path.insert(0, str(ROOT / "tools"))
            mtest = importlib.import_module("mtest")
            qmp = importlib.import_module("qmp")
            test = importlib.import_module("test_m15_guest")
            original_semantic = mtest.semantic_exit_rc
            semantic_exits: dict[str, int | None] = {}

            def capture_exit(rc: int | None, serial: str, label: str) -> int | None:
                actual_exits[label] = rc
                semantic_rc = original_semantic(rc, serial, label)
                semantic_exits[label] = semantic_rc
                return semantic_rc

            mtest.semantic_exit_rc = capture_exit
            original_qmp = qmp.Qmp

            class EvidenceQmp(original_qmp):
                def __init__(self, path: str, connect_timeout_s: float = 10.0):
                    super().__init__(path, connect_timeout_s)
                    if args.backend == "kvm":
                        state = self.command("query-kvm")
                        qmp_states.append({"kind": "query-kvm", **state})
                        if state.get("present") is not True or state.get("enabled") is not True:
                            raise RuntimeError(f"KVM not active in trial QMP: {state}")
                    else:
                        cpus = self.command("query-cpus-fast")
                        qmp_states.append({"kind": "query-cpus-fast", "cpu_count": len(cpus)})

            qmp.Qmp = EvidenceQmp
            original_check = test.check_serial

            def record_check(label: str, serial: str, rc: int | None,
                             avx_expected: bool) -> bool:
                passed = original_check(label, serial, rc, avx_expected)
                profile_checks[label] = passed
                return passed

            test.check_serial = record_check
            start = time.monotonic()
            for label, _, _ in CPU_RUNS:
                for suffix in ("serial", "qemu-stderr"):
                    stale = ROOT / "build" / f"{suffix}-{label}.log"
                    stale.unlink(missing_ok=True)
            real_popen = subprocess.Popen
            process_index = 0

            class TrialPopen(real_popen):
                def __init__(self, command: Any, *a: Any, **kw: Any):
                    nonlocal process_index
                    self.trial_label: str | None = None
                    command_text = " ".join(map(str, command)) if isinstance(command, (list, tuple)) else str(command)
                    if "qemu-system-x86_64" in command_text.lower():
                        if process_index < len(CPU_RUNS):
                            self.trial_label = CPU_RUNS[process_index][0]
                        process_index += 1
                    super().__init__(command, *a, **kw)

                def wait(self, *a: Any, **kw: Any) -> int:
                    result = super().wait(*a, **kw)
                    if self.trial_label:
                        actual_exits[self.trial_label] = result
                    return result

            try:
                subprocess.Popen = TrialPopen
                with runner_log.open("w", encoding="utf-8") as stream, \
                        contextlib.redirect_stdout(stream), contextlib.redirect_stderr(stream):
                    test_rc = test.main()
            finally:
                subprocess.Popen = real_popen
            receipt["test_runner_exit_status"] = test_rc
            receipt["test_runner_elapsed_seconds"] = round(time.monotonic() - start, 3)
            receipt["accelerator_qmp_evidence"] = qmp_states
            for label, cpu, avx in CPU_RUNS:
                serial_src = ROOT / "build" / f"serial-{label}.log"
                stderr_src = ROOT / "build" / f"qemu-stderr-{label}.log"
                item: dict[str, Any] = {
                    "label": label, "cpu": cpu, "avx_expected": avx,
                    "qemu_exit_status": actual_exits.get(label),
                    "semantic_guest_exit_status": semantic_exits.get(label),
                    "strict_harness_checks_passed": profile_checks.get(label, False),
                }
                if serial_src.is_file():
                    serial_copy = logs_dir / f"serial-{label}.log"
                    shutil.copyfile(serial_src, serial_copy)
                    serial = serial_copy.read_text(errors="replace")
                    item["serial_log"] = output_file_record(serial_copy, out)
                    item["strict_xf_evidence"] = strict_xf_evidence(serial)
                    item["stage_outcomes"] = classify_trial(
                        serial, actual_exits.get(label), semantic_exits.get(label),
                        profile_checks.get(label, False))
                else:
                    item["strict_xf_evidence"] = {}
                if stderr_src.is_file():
                    stderr_copy = logs_dir / f"qemu-stderr-{label}.log"
                    shutil.copyfile(stderr_src, stderr_copy)
                    item["qemu_stderr_log"] = output_file_record(stderr_copy, out)
                receipt["trials"].append(item)
            shutil.copyfile(runner_log, logs_dir / "runner.log")
            receipt["runner_log"] = output_file_record(logs_dir / "runner.log", out)
            trials = receipt["trials"]
            all_xf = all(
                all(item.get("strict_xf_evidence", {}).values())
                and item.get("strict_harness_checks_passed") is True
                and item.get("qemu_exit_status") == 0
                and item.get("semantic_guest_exit_status") == 0
                and item.get("stage_outcomes", {}).get("trial") == "PASS"
                for item in trials
            )
            kvm_records_ok = (args.backend != "kvm" or
                              len([x for x in qmp_states
                                   if x.get("kind") == "query-kvm"
                                   and x.get("present") is True
                                   and x.get("enabled") is True]) >= len(CPU_RUNS))
            if test_rc == 0 and len(trials) == len(CPU_RUNS) and all_xf and kvm_records_ok:
                status = "PASS"
            else:
                status = "FAIL"
                reasons.append("one or more strict guest, QEMU exit, accelerator, or containment assertions failed")
        except Exception as error:
            reasons.append(f"strict guest test could not complete: {type(error).__name__}: {error}")
            status = "FAIL"
            logs_dir.mkdir(exist_ok=True)
            for label, _, _ in CPU_RUNS:
                serial_src = ROOT / "build" / f"serial-{label}.log"
                stderr_src = ROOT / "build" / f"qemu-stderr-{label}.log"
                item: dict[str, Any] = {
                    "label": label,
                    "qemu_exit_status": actual_exits.get(label),
                    "semantic_guest_exit_status": semantic_exits.get(label),
                    "strict_harness_checks_passed": profile_checks.get(label, False),
                    "strict_xf_evidence": {},
                }
                if serial_src.is_file():
                    serial_copy = logs_dir / f"serial-{label}.log"
                    shutil.copyfile(serial_src, serial_copy)
                    serial = serial_copy.read_text(errors="replace")
                    item["serial_log"] = output_file_record(serial_copy, out)
                    item["strict_xf_evidence"] = strict_xf_evidence(serial)
                    item["stage_outcomes"] = classify_trial(
                        serial, actual_exits.get(label), semantic_exits.get(label),
                        profile_checks.get(label, False))
                if stderr_src.is_file():
                    stderr_copy = logs_dir / f"qemu-stderr-{label}.log"
                    shutil.copyfile(stderr_src, stderr_copy)
                    item["qemu_stderr_log"] = output_file_record(stderr_copy, out)
                receipt["trials"].append(item)
            if runner_log.is_file():
                shutil.copyfile(runner_log, logs_dir / "runner.log")
                receipt["runner_log"] = output_file_record(logs_dir / "runner.log", out)

    receipt["status"] = status
    receipt["issues"] = reasons
    probe_output = receipt.get("accelerator_probe", {}).get("probe_output")
    if probe_output is not None:
        (out / "logs").mkdir(exist_ok=True)
        probe_log = out / "logs/accelerator-probe.log"
        probe_log.write_text(probe_output)
        receipt["accelerator_probe"]["probe_log"] = output_file_record(probe_log, out)
    receipt_path = out / "receipt.json"
    receipt_path.write_text(json.dumps(receipt, indent=2, sort_keys=True) + "\n")
    print(f"PHASE15_EXTERNAL_STATUS={status}")
    print(f"RECEIPT_PATH={receipt_path}")
    for issue in reasons:
        print(f"ISSUE: {issue}")
    if status == "PASS":
        print("HARDWARE-BACKED-GUEST-TESTED: strict Phase 15 guest passed both CPU profiles")
        return 0
    print(f"{status}: no Phase 15 hardware-backed qualification claim")
    return 3 if status == "BLOCKED" else 1


if __name__ == "__main__":
    raise SystemExit(main())
