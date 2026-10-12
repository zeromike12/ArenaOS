#!/usr/bin/env python3
"""Independently verify hashes and guest assertions in an external trial receipt."""
from __future__ import annotations

import argparse
import hashlib
import json
import re
import sys
from pathlib import Path
from typing import Any


EFI_SHA = "bbbc63df271c3d7d5109789369f72e093b02ef77c77ebe25273b6da4c00d238e"
ESP_SHA = "33dd1576ab597a7d0b3e9a9100bc618e9b251a7a645e77e79d8852a404c40a6a"
XF_SHA = "1f057068b020baefe06f7f3b36bc0beeafcb879bd93d60c5ea66a3cce616c8d3"
XF_SOURCE_SHA = "f59c35ed5d5bbe810a9baad573418b9a64a385e2330b23627ebfc7bffa4a9c3a"
STRICT_SOURCE_COMMIT = "6586a0a18981932ae7048ca267fa425411f8e463"
PROFILES = {"phase15-avx", "phase15-qemu64"}


class ReceiptError(RuntimeError):
    pass


def digest(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            h.update(block)
    return h.hexdigest()


def included_file(record: dict[str, Any], root: Path) -> Path:
    relative = record.get("relative_path")
    path = root / relative if relative else Path(record["path"])
    if not path.is_file():
        raise ReceiptError(f"receipt file is missing: {relative or path}")
    actual = digest(path)
    if path.stat().st_size != record.get("bytes") or actual != record.get("sha256"):
        raise ReceiptError(f"receipt hash/size mismatch: {relative or path}")
    return path


def check_guest_log(log: str) -> None:
    required = (
        "m15:test:thread_isolation_block_yield_preempt: PASS",
        "m15:test:process_isolation_slot_reuse: PASS",
        "m15:test:optional_cpu_feature_reporting: PASS",
        "m15:test:xf_delivery_probe: PASS",
        "m15:test:mf_ud_fault_containment: PASS",
        "minimal #XF probe receipts: fault=Some(113), sibling=Some(2a), peer=Some(2a), sibling_and_peer_survived=true, fault_slot_released=true, resource_baseline_restored=true",
        "minimal unmasked SQRTSS #XF probe delivered vector 19; same-process sibling and independent peer survived;",
        "halting via UEFI ResetSystem(shutdown)",
    )
    for marker in required:
        if marker not in log:
            raise ReceiptError(f"guest log is missing required proof: {marker}")
    if "mmx=true" in log:
        if "m15:test:mmx_state: PASS" not in log:
            raise ReceiptError("guest log omitted MMX state preservation and cleanup")
    elif "m15:test:mmx_state: SKIP (CPUID MMX=0)" not in log:
        raise ReceiptError("guest log omitted an honest MMX capability result")
    if not re.search(
            r"\[arena user fault\] pid=\d+ vector=0x13 \(#XF SIMD Floating Point\)",
            log):
        raise ReceiptError("guest log has no dispatched CPL3 vector-19 exception")
    if not re.search(
            r"feature discovery after FP setup:.*\bfpu=true\b.*\bsse=true\b.*\bsse2=true\b",
            log):
        raise ReceiptError("guest log does not report x87/SSE/SSE2 enabled")
    results = re.findall(r"^(m\d+): RESULT (PASS|FAIL) \((\d+)/(\d+)\)$",
                         log, re.MULTILINE)
    if not results or any(state != "PASS" for _, state, _, _ in results):
        raise ReceiptError("guest log has a missing or failed result line")
    if "PANIC" in log or "RESULT FAIL" in log:
        raise ReceiptError("guest log contains PANIC or RESULT FAIL")


def verify(receipt_path: Path) -> tuple[bool, list[str], list[str]]:
    receipt_path = receipt_path.resolve()
    root = receipt_path.parent
    issues: list[str] = []
    notes: list[str] = []
    receipt = json.loads(receipt_path.read_text())
    if receipt.get("schema") != "arenaos.phase15.external-qualification.v1":
        raise ReceiptError("unknown receipt schema")
    if receipt.get("backend_requested") not in ("kvm", "whpx"):
        raise ReceiptError("receipt has no supported hardware backend")
    if receipt.get("strict_acceptance", {}).get("retry_count") != 0:
        raise ReceiptError("receipt reports a retry; first-run success is required")
    if receipt.get("strict_acceptance", {}).get("attempt_count") != 2:
        raise ReceiptError("receipt must contain exactly the two existing CPU-profile trials")
    if receipt.get("strict_acceptance", {}).get("strict_image_build_source_commit") != STRICT_SOURCE_COMMIT:
        raise ReceiptError("strict image source revision is absent or unexpected")

    if receipt.get("status") != "PASS":
        images = receipt.get("images", {})
        for name, expected in (("arena-boot.efi", EFI_SHA),
                               ("arena-esp.img", ESP_SHA),
                               ("xf_probe", XF_SHA)):
            record = images.get(name)
            if isinstance(record, dict):
                if record.get("sha256") != expected:
                    raise ReceiptError(f"partial receipt has an incorrect image identity: {name}")
                included_file(record, root)
        issues = [f"qualification outcome is {receipt.get('status')}, not PASS"]
        issues.extend(str(issue) for issue in receipt.get("issues", []))
        return False, issues, notes

    images = receipt.get("images", {})
    for name, expected in (("arena-boot.efi", EFI_SHA), ("arena-esp.img", ESP_SHA)):
        record = images.get(name)
        if not isinstance(record, dict) or record.get("sha256") != expected:
            raise ReceiptError(f"pinned strict image identity is absent for {name}")
        included_file(record, root)
    if images.get("esp_embeds_exact_efi") is not True:
        raise ReceiptError("receipt did not verify that the ESP embeds its exact EFI")
    try:
        from pyfatfs.PyFatFS import PyFatFS
    except ImportError as error:
        raise ReceiptError("pyfatfs==1.1.0 is required to verify the ESP") from error
    esp_path = included_file(images["arena-esp.img"], root)
    efi_path = included_file(images["arena-boot.efi"], root)
    fs = PyFatFS(str(esp_path), read_only=True)
    try:
        if fs.getbytes("/EFI/BOOT/BOOTX64.EFI") != efi_path.read_bytes():
            raise ReceiptError("embedded ESP EFI differs from the paired strict EFI")
    finally:
        fs.close()
    xf_record = images.get("xf_probe")
    if not isinstance(xf_record, dict) or xf_record.get("sha256") != XF_SHA:
        raise ReceiptError("compiled strict CPL3 #XF fixture hash is absent or incorrect")
    included_file(xf_record, root)

    source_record = receipt.get("xf_source")
    if not isinstance(source_record, dict) or source_record.get("sha256") != XF_SOURCE_SHA:
        raise ReceiptError("CPL3 fixture source hash is absent")
    source_path = included_file(source_record, root)
    source = source_path.read_text()
    for token in ("CPL3", "ldmxcsr", "stmxcsr", "0x00001F00",
                  "invalid-operation mask bit 7 clear", "0xBF800000",
                  "sqrtss xmm0, xmm0"):
        if token not in source:
            raise ReceiptError(f"source fixture does not prove the strict instruction/state: {token}")
    if re.search(r'"\s*int\s+(?:19|0x13)\b', source, re.IGNORECASE):
        raise ReceiptError("source fixture contains a synthetic vector-19 software interrupt")
    source_assertions = receipt.get("xf_source_assertions", {})
    if not all(value is True for value in source_assertions.values()) or len(source_assertions) != 7:
        raise ReceiptError("recorded source assertion set is incomplete")

    for relative, record in receipt.get("source_review_files", {}).items():
        included_file(record, root)
    if "tools/test_m15_guest.py" not in receipt.get("source_review_files", {}):
        raise ReceiptError("exact strict guest test source was not preserved")
    source_files = receipt.get("source_review_files", {})
    m15_record = source_files.get("kernel/kernel/src/m15.rs")
    idt_record = source_files.get("kernel/kernel/src/arch/x86_64/idt.rs")
    elf_record = source_files.get("kernel/kernel/src/elf.rs")
    if not all(isinstance(item, dict) for item in (m15_record, idt_record, elf_record)):
        raise ReceiptError("kernel fault-dispatch and fixture source was not preserved")
    m15_source = included_file(m15_record, root).read_text()
    idt_source = included_file(idt_record, root).read_text()
    elf_source = included_file(elf_record, root).read_text()
    if "const XF_STATUS: u64 = 0x100 + 19" not in m15_source or \
            "fault_observed != Some(XF_STATUS)" not in m15_source:
        raise ReceiptError("kernel source does not enforce vector-19 exit status")
    if 'EXC_STUB 19, 0' not in idt_source or "cs & 3 == 3" not in idt_source or \
            "exit_on_user_fault(vector)" not in idt_source:
        raise ReceiptError("kernel source does not wire vector 19 to user-fault containment")
    if "FPU_XF_PROBE_IMAGE" not in elf_source:
        raise ReceiptError("kernel source does not embed the pinned strict fixture")

    backend = receipt["backend_requested"]
    qemu = receipt.get("qemu", {})
    if qemu.get("version_exit_status") != 0 or "QEMU emulator version" not in qemu.get("version", ""):
        raise ReceiptError("QEMU version identity is missing or invalid")
    if qemu.get("requested_accelerator_advertised") is not True:
        raise ReceiptError("QEMU did not advertise the requested hardware accelerator")
    backend_args = qemu.get("strict_backend_args", [])
    if backend_args != ["-accel", backend]:
        raise ReceiptError("strict trial did not request the explicit hardware accelerator")
    if qemu.get("accelerator_list_exit_status") not in (0, 1):
        raise ReceiptError("QEMU accelerator-list probe did not complete")
    if re.search(r"\btc[g]\b", " ".join(qemu.get("command_prefix", [])), re.I):
        raise ReceiptError("receipt command prefix includes TCG")
    for label, record in (("QEMU binary", qemu.get("binary")),
                          ("OVMF code", receipt.get("firmware", {}).get("code")),
                          ("OVMF vars", receipt.get("firmware", {}).get("vars_template"))):
        if not isinstance(record, dict) or not re.fullmatch(r"[0-9a-f]{64}", record.get("sha256", "")):
            raise ReceiptError(f"{label} SHA-256 identity is missing")
        dependency = Path(record.get("path", ""))
        if dependency.is_file():
            if dependency.stat().st_size != record.get("bytes") or digest(dependency) != record["sha256"]:
                raise ReceiptError(f"{label} hash differs from its recorded identity")
        else:
            notes.append(f"{label} binary is external to this receipt; recorded SHA-256 retained")
    if not receipt.get("accelerator_probe", {}).get("initialized"):
        raise ReceiptError("hardware accelerator initialization probe did not pass")
    probe = receipt["accelerator_probe"]
    if probe.get("qemu_exit_status") != 0 or probe.get("cpu_count", 0) < 1:
        raise ReceiptError("accelerator initialization did not complete with a live vCPU and clean probe exit")
    if "-accel" not in probe.get("command", []) or backend not in probe.get("command", []):
        raise ReceiptError("accelerator initialization probe did not explicitly select the requested backend")
    probe_log = probe.get("probe_log")
    if not isinstance(probe_log, dict):
        raise ReceiptError("accelerator QMP probe log was not preserved")
    included_file(probe_log, root)
    if backend == "kvm":
        states = [row for row in receipt.get("accelerator_qmp_evidence", [])
                  if row.get("kind") == "query-kvm" and row.get("present") is True
                  and row.get("enabled") is True]
        if len(states) < 2:
            raise ReceiptError("both strict guest boots must report KVM enabled over QMP")
    else:
        states = [row for row in receipt.get("accelerator_qmp_evidence", [])
                  if row.get("kind") == "query-cpus-fast" and row.get("cpu_count", 0) > 0]
        if len(states) < 2:
            raise ReceiptError("both WHPX guest boots must report a live QEMU vCPU")

    trials = receipt.get("trials", [])
    if {trial.get("label") for trial in trials} != PROFILES or len(trials) != 2:
        raise ReceiptError("receipt must contain exactly one trial for each strict CPU profile")
    expected_cpu = {
        "phase15-avx": "max,+nx,+smep,+smap",
        "phase15-qemu64": "qemu64,+nx,+smep,+smap",
    }
    for trial in trials:
        if trial.get("cpu") != expected_cpu[trial["label"]]:
            raise ReceiptError(f"{trial.get('label')} did not use its pinned CPU profile")
        if trial.get("qemu_exit_status") != 0:
            raise ReceiptError(f"{trial.get('label')} actual QEMU exit was not zero")
        if trial.get("strict_harness_checks_passed") is not True:
            raise ReceiptError(f"{trial.get('label')} strict harness checks failed")
        record = trial.get("serial_log")
        if not isinstance(record, dict):
            raise ReceiptError(f"{trial.get('label')} serial log was not preserved")
        log_path = included_file(record, root)
        log = log_path.read_text(errors="replace")
        check_guest_log(log)
        if not all(trial.get("strict_xf_evidence", {}).values()):
            raise ReceiptError(f"{trial.get('label')} structured #XF evidence is incomplete")
        stderr = trial.get("qemu_stderr_log")
        if not isinstance(stderr, dict):
            raise ReceiptError(f"{trial.get('label')} QEMU stderr log was not preserved")
        included_file(stderr, root)
    if not isinstance(receipt.get("runner_log"), dict):
        raise ReceiptError("test runner output log was not preserved")
    included_file(receipt["runner_log"], root)
    if receipt.get("status") != "PASS":
        issues.append(f"qualification outcome is {receipt.get('status')}, not PASS")
    if receipt.get("issues"):
        issues.extend(str(issue) for issue in receipt["issues"])
    return not issues, issues, notes


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--receipt", required=True, type=Path)
    args = parser.parse_args()
    try:
        passed, issues, notes = verify(args.receipt)
    except (OSError, ValueError, KeyError, TypeError, ReceiptError) as error:
        print(f"PHASE 15 EXTERNAL RECEIPT INVALID: {error}", file=sys.stderr)
        return 2
    if passed:
        print("PASS: strict image, source fixture, hardware-backend, guest assertions, logs, and exits verified")
        print("HARDWARE-BACKED-GUEST-TESTED: receipt is complete for both CPU profiles")
        for note in notes:
            print(f"NOTE: {note}")
        return 0
    for issue in issues:
        print(f"FAIL: {issue}", file=sys.stderr)
    print("BLOCKED/FAIL: receipt does not qualify Phase 15", file=sys.stderr)
    return 1


if __name__ == "__main__":
    raise SystemExit(main())
