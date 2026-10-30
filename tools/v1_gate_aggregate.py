#!/usr/bin/env python3
"""Requirement-aware V1 gate aggregator.

A green stage is not qualification by itself. The full result is derived
from the requirement matrix below, its required stage receipts, and the
structured guest evidence for requirements 5e (client-policy controls) and 5f
(real kernel-granted 301/300-page integrations). A historical set of stage
PASS labels cannot stand in for a requirement that the current candidate did
not prove.

    python3 tools/v1_gate_aggregate.py --mode guest  # review-candidate subset
    python3 tools/v1_gate_aggregate.py --mode full   # every requirement
"""

import json
import re
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
DEFAULT_RECEIPTS = Path("build/v1-gate/receipts")

# Each acceptance requirement names the evidence stages that can satisfy it.
# The full stage set is derived from this table so adding a requirement without
# adding its evidence stage cannot silently leave --mode full green.
REQUIREMENTS = {
    "1": {
        "label": "Acknowledgment and visible response through production logic",
        "stages": ("guest-gcc", "guest-clang"),
    },
    "2": {
        "label": "Successful checkpoint frames published and verified",
        "stages": ("guest-gcc", "guest-clang", "package", "verify"),
    },
    "3": {
        "label": "Cleanup returns enforceable resources to baseline",
        "stages": ("cycles",),
    },
    "4": {
        "label": "Guest refusal matrix under both compilers",
        "stages": ("refusals-gcc", "refusals-clang"),
    },
    "4b": {
        "label": "Real stale/foreign capability case or M-recorded waiver",
        "stages": ("refusals-gcc", "refusals-clang"),
        "validator": "foreign-capability",
    },
    "5": {
        "label": "Final-artifact runtime stability (100/100 boots)",
        "stages": ("stability",),
    },
    "5b": {
        "label": "Canonical protected build and reproducible artifact identity",
        "stages": ("build",),
    },
    "5c": {
        "label": "ESP embedded EFI identity",
        "stages": ("build",),
    },
    "5d": {
        "label": "Formal 121-suite regression on the candidate",
        "stages": ("regression",),
    },
    "5e": {
        "label": "C layout boundary predicates (exact-grant proof is separate)",
        "stages": ("refusals-gcc", "refusals-clang"),
        "validator": "layout-boundary",
    },
    "5f": {
        "label": "Real-grant C integration: 301 accepted, 300 refused before map/write/window/damage",
        "stages": ("layout-integration-gcc", "layout-integration-clang"),
        "validator": "real-layout-grant",
    },
    "5g": {
        "label": "Phase-15 FP/SIMD/exception regression on the final EFI/ESP",
        "stages": ("phase15",),
    },
    "6": {
        "label": "Gate controls prevent partial or unsupported qualification claims",
        "stages": ("controls", "aggregate-controls"),
    },
    "7": {
        "label": "Review evidence package sealed and fresh-extraction verified",
        "stages": ("package", "verify"),
    },
}

BASE_STAGES = ("build", "env", "controls", "aggregate-controls")
GUEST_STAGES = (
    "guest-gcc", "guest-clang", "refusals-gcc", "refusals-clang",
    "layout-integration-gcc", "layout-integration-clang",
    "cycles", "package", "verify",
)


def _ordered_unique(items):
    return list(dict.fromkeys(items))


FULL_STAGES = _ordered_unique(
    [*BASE_STAGES]
    + [stage for requirement in REQUIREMENTS.values()
       for stage in requirement["stages"]]
)

# quick and guest are deliberately partial. `full` is the ordered union of
# all acceptance requirements above, not a separately maintained second list.
STAGES = {
    "quick": list(BASE_STAGES),
    "guest": [*BASE_STAGES, *GUEST_STAGES],
    "full": FULL_STAGES,
}

STAGE_PURPOSE = {
    "build": "canonical boot image build",
    "env": "environment receipts (emulator, loader, toolchains, guards)",
    "controls": "focused production-logic controls",
    "aggregate-controls": "requirement-matrix and aggregate negative controls",
    "guest-gcc": "V1 guest proof, GCC-built client",
    "guest-clang": "V1 guest proof, Clang-built client",
    "refusals-gcc": "predicate-only refusal matrix, GCC client",
    "refusals-clang": "predicate-only refusal matrix, Clang client",
    "layout-integration-gcc": "real 301/300-page SharedRegion integration, GCC client",
    "layout-integration-clang": "real 301/300-page SharedRegion integration, Clang client",
    "cycles": "ten launch/close lifecycle cycles",
    "package": "evidence manifest + archive",
    "verify": "evidence verifies from a fresh extraction",
    "phase15": "Phase-15 guest validation on the final EFI/ESP",
    "regression": "formal regression suite (tools/run_tests.sh)",
    "stability": "runtime boot-stability loop",
}

# Requirement 5e is intentionally stricter than "the refusal stage exited 0".
# These are case-level facts emitted by the current guest matrix. The 300 and
# 301 values are client-policy inputs sent to the exact production predicate;
# they are NOT claimed to be kernel-created SharedRegion grants.
LAYOUT_CASES = {
    "layout-required-pages": {
        "kind": "CLIENT-POLICY", "want": 301, "got": 301,
    },
    "layout-301-accepted": {
        "kind": "CLIENT-POLICY", "want": 0, "got": 0,
    },
    "layout-300-refused": {
        "kind": "CLIENT-POLICY", "want": 7, "got": 7,
    },
    "layout-pages-minimum": {
        "kind": "KERNEL", "want": 301, "minimum_got": 301,
    },
    "layout-pixel-window-fits": {
        "kind": "CLIENT-POLICY", "want": 1, "got": 1,
    },
}


def sha256_file(path: Path) -> str:
    import hashlib
    try:
        return hashlib.sha256(path.read_bytes()).hexdigest()
    except OSError:
        return "MISSING"


def load(receipts: Path, stage: str):
    path = receipts / f"{stage}.json"
    if not path.is_file():
        return None
    try:
        data = json.loads(path.read_text())
        return data if isinstance(data, dict) else None
    except (OSError, ValueError):
        return None


def layout_evidence_problems(receipt: dict, stage: str) -> list[str]:
    """Validate requirement-5e evidence for one compiler cell."""
    label = f"{stage} requirement-5e evidence"
    raw_dir = receipt.get("evidence_dir")
    if not isinstance(raw_dir, str) or not raw_dir.strip():
        return [f"{label}: receipt has no evidence_dir"]

    evidence_dir = Path(raw_dir)
    if not evidence_dir.is_absolute():
        evidence_dir = REPO_ROOT / evidence_dir
    try:
        evidence_dir = evidence_dir.resolve()
        evidence_dir.relative_to(REPO_ROOT.resolve())
    except (OSError, ValueError):
        return [f"{label}: evidence_dir is outside the repository"]

    evidence_path = evidence_dir / "refusals-evidence.json"
    try:
        data = json.loads(evidence_path.read_text())
    except (OSError, ValueError) as exc:
        return [f"{label}: cannot read {evidence_path}: {exc}"]
    if not isinstance(data, dict):
        return [f"{label}: evidence root is not an object"]

    expected_compiler = stage.rsplit("-", 1)[-1]
    errors = []
    if data.get("guest_compiler_under_test") != expected_compiler:
        errors.append(
            f"{label}: evidence compiler is "
            f"{data.get('guest_compiler_under_test')!r}, expected {expected_compiler!r}")
    if data.get("pass") is not True or data.get("rc") != 0:
        errors.append(f"{label}: guest evidence does not report pass/exit 0")

    # The structured matrix is part of the stage, not a free-floating green
    # report. Bind it to the same source commit recorded by the stage receipt
    # and to the APB1 payload whose hash the guest harness recorded. This
    # prevents a historical report or substituted bundle from being reused
    # just because it happens to contain the required case names.
    evidence_commit = data.get("source_commit")
    receipt_commit = receipt.get("source_commit")
    if not isinstance(evidence_commit, str) or evidence_commit != receipt_commit:
        errors.append(
            f"{label}: evidence source commit {evidence_commit!r} does not match "
            f"stage receipt {receipt_commit!r}")
    apb1_path = evidence_dir / "c-gfx-refusals.apb1"
    actual_apb1_sha256 = sha256_file(apb1_path)
    if actual_apb1_sha256 == "MISSING" or \
            data.get("apb1_sha256") != actual_apb1_sha256:
        errors.append(
            f"{label}: evidence APB1 hash {data.get('apb1_sha256')!r} does not "
            f"match {apb1_path.name} ({actual_apb1_sha256})")

    record = data.get("record")
    if not isinstance(record, dict):
        return errors + [f"{label}: missing structured record"]
    if record.get("verdict_ok") is not True:
        errors.append(f"{label}: refusal matrix verdict is not PASS")
    if record.get("unclassified"):
        errors.append(f"{label}: contains unclassified refusal cases")

    cases = record.get("cases")
    classes = record.get("classes")
    if not isinstance(cases, dict) or not isinstance(classes, dict):
        return errors + [f"{label}: cases/classes are missing or malformed"]

    for case_name, contract in LAYOUT_CASES.items():
        case = cases.get(case_name)
        if not isinstance(case, dict):
            errors.append(f"{label}: missing case {case_name}")
            continue
        if case.get("count") != 1:
            errors.append(f"{label}: {case_name} count is {case.get('count')!r}, expected 1")
        if case.get("verdict") != "PASS":
            errors.append(f"{label}: {case_name} verdict is {case.get('verdict')!r}")
        if case.get("want") != contract["want"]:
            errors.append(
                f"{label}: {case_name} want={case.get('want')!r}, "
                f"expected {contract['want']}")
        got = case.get("got")
        if "got" in contract:
            if got != contract["got"]:
                errors.append(
                    f"{label}: {case_name} got={got!r}, expected {contract['got']}")
        elif not isinstance(got, int) or got < contract["minimum_got"]:
            errors.append(
                f"{label}: {case_name} got={got!r}, "
                f"expected at least {contract['minimum_got']} real granted pages")
        classification = classes.get(case_name)
        if not isinstance(classification, dict) or \
                classification.get("kind") != contract["kind"]:
            got_kind = classification.get("kind") if isinstance(classification, dict) else None
            errors.append(
                f"{label}: {case_name} class={got_kind!r}, "
                f"expected {contract['kind']!r}")
    return errors




def foreign_capability_evidence_problems(receipt: dict, stage: str) -> tuple[list[str], bool]:
    """Validate the synthetic foreign-identity classification and M waiver.

    `foreign-identity` currently feeds mutated identity bytes to the Desktop
    validator. That is not a real stale/foreign capability test, so the
    requirement stays OPEN unless the same run's evidence root contains an
    explicit, scoped M-WAIVER.json. No waiver is synthesized by this tool.
    """
    label = f"{stage} stale/foreign capability evidence"
    raw_dir = receipt.get("evidence_dir")
    if not isinstance(raw_dir, str) or not raw_dir.strip():
        return [f"{label}: receipt has no evidence_dir"], False
    evidence_dir = Path(raw_dir)
    if not evidence_dir.is_absolute():
        evidence_dir = REPO_ROOT / evidence_dir
    try:
        evidence_dir = evidence_dir.resolve()
        evidence_dir.relative_to(REPO_ROOT.resolve())
    except (OSError, ValueError):
        return [f"{label}: evidence_dir is outside the repository"], False
    path = evidence_dir / "refusals-evidence.json"
    try:
        data = json.loads(path.read_text())
    except (OSError, ValueError) as exc:
        return [f"{label}: cannot read refusal evidence: {exc}"], False
    record = data.get("record") if isinstance(data, dict) else None
    if not isinstance(record, dict):
        return [f"{label}: missing structured refusal record"], False
    if data.get("pass") is not True or record.get("verdict_ok") is not True:
        return [f"{label}: refusal matrix did not pass"], False
    classes = record.get("classes")
    cases = record.get("cases")
    classification = classes.get("foreign-identity") if isinstance(classes, dict) else None
    case = cases.get("foreign-identity") if isinstance(cases, dict) else None
    if not isinstance(classification, dict) \
            or classification.get("kind") != "IDENTITY" \
            or "production identity validator" not in str(classification.get("description", "")) \
            or "object id one above" not in str(classification.get("description", "")):
        return [f"{label}: foreign-identity is not honestly classified as synthetic IDENTITY"], False
    if not isinstance(case, dict) or case.get("count") != 1 \
            or case.get("verdict") != "PASS" or case.get("want") != 0 \
            or case.get("got") != 0:
        return [f"{label}: synthetic foreign-identity control is missing or failed"], False

    waiver_path = evidence_dir.parent / "M-WAIVER.json"
    if not waiver_path.exists():
        return [], False
    try:
        waiver = json.loads(waiver_path.read_text())
    except (OSError, ValueError) as exc:
        return [f"{label}: M-WAIVER.json is unreadable: {exc}"], False
    if not isinstance(waiver, dict):
        return [f"{label}: M-WAIVER.json is not an object"], False
    expected = {
        "kind": "m_recorded_v1_coverage_waiver",
        "requirement": "4b",
        "decision": "waive",
        "decision_by": "M",
        "scope": "no real stale/foreign capability is presented to the kernel",
        "source_commit": receipt.get("source_commit"),
        "efi_sha256": receipt.get("efi_sha256"),
    }
    mismatches = [f"{key}={waiver.get(key)!r}, expected {value!r}"
                  for key, value in expected.items() if waiver.get(key) != value]
    for key in ("recorded_utc", "rationale"):
        if not isinstance(waiver.get(key), str) or not waiver[key].strip():
            mismatches.append(f"{key} is missing")
    if mismatches:
        return [f"{label}: invalid M-WAIVER.json: " + "; ".join(mismatches)], False
    return [], True

def _integration_file(evidence_dir: Path, raw: object) -> Path | None:
    """Resolve a recorded artifact only if it remains inside evidence_dir."""
    if not isinstance(raw, str) or not raw:
        return None
    candidate = Path(raw)
    choices = [candidate] if candidate.is_absolute() else [
        evidence_dir / candidate,
        REPO_ROOT / candidate,
    ]
    for choice in choices:
        try:
            resolved = choice.resolve()
            resolved.relative_to(evidence_dir.resolve())
        except (OSError, ValueError):
            continue
        if resolved.is_file():
            return resolved
    return None


def layout_integration_evidence_problems(receipt: dict, stage: str) -> list[str]:
    """Validate real SharedRegion grants, ordinary C control flow, and cleanup."""
    label = f"{stage} requirement-5f real-grant evidence"
    raw_dir = receipt.get("evidence_dir")
    if not isinstance(raw_dir, str) or not raw_dir.strip():
        return [f"{label}: receipt has no evidence_dir"]
    evidence_dir = Path(raw_dir)
    if not evidence_dir.is_absolute():
        evidence_dir = REPO_ROOT / evidence_dir
    try:
        evidence_dir = evidence_dir.resolve()
        evidence_dir.relative_to(REPO_ROOT.resolve())
    except (OSError, ValueError):
        return [f"{label}: evidence_dir is outside the repository"]

    path = evidence_dir / "integration-evidence.json"
    try:
        data = json.loads(path.read_text())
    except (OSError, ValueError) as exc:
        return [f"{label}: cannot read integration-evidence.json: {exc}"]
    if not isinstance(data, dict):
        return [f"{label}: evidence root is not an object"]

    errors: list[str] = []
    as_dict = lambda value: value if isinstance(value, dict) else {}
    expected_compiler = stage.rsplit("-", 1)[-1]
    if data.get("kind") != "real_shared_region_layout_integration":
        errors.append(f"{label}: evidence kind is not real SharedRegion integration")
    if data.get("guest_compiler_under_test") != expected_compiler:
        errors.append(
            f"{label}: evidence compiler is {data.get('guest_compiler_under_test')!r}, "
            f"expected {expected_compiler!r}")
    if data.get("pass") is not True or data.get("rc") != 0:
        errors.append(f"{label}: integration evidence does not report pass/exit 0")
    if data.get("source_commit") != receipt.get("source_commit"):
        errors.append(
            f"{label}: evidence source commit {data.get('source_commit')!r} does not "
            f"match stage receipt {receipt.get('source_commit')!r}")

    identities = data.get("identities")
    if not isinstance(identities, dict):
        return errors + [f"{label}: candidate identities are missing"]
    efi = as_dict(identities.get("boot_efi"))
    esp = as_dict(identities.get("esp"))
    extracted = as_dict(identities.get("extracted_efi"))
    if efi.get("sha256") != receipt.get("efi_sha256"):
        errors.append(f"{label}: boot EFI identity does not match the stage receipt")
    if esp.get("sha256") != receipt.get("esp_sha256"):
        errors.append(f"{label}: ESP identity does not match the stage receipt")
    if not data.get("identities", {}).get("efi_embedded_matches_build") \
            or extracted.get("sha256") != efi.get("sha256"):
        errors.append(f"{label}: extracted EFI is not byte-identical to the built EFI")
    extracted_path = _integration_file(evidence_dir, extracted.get("path"))
    if extracted_path is None or sha256_file(extracted_path) != extracted.get("sha256"):
        errors.append(f"{label}: extracted EFI artifact is missing or has a bad digest")

    protected = (
        ("protected_phase14_pie",
         "d7b0a8cf9c735c3898a867d824563f06b0d949df80fa1a9c96f9180a395fea2e"),
        ("protected_signed_apb1",
         "04add54f09042353fc42511978c82f8b8efb87f7d5f0657f9116f456276b3dfa"),
    )
    for key, expected_hash in protected:
        actual = as_dict(identities.get(key)).get("sha256")
        if actual != expected_hash:
            errors.append(f"{label}: protected artifact {key} changed ({actual!r})")

    qemu = as_dict(identities.get("qemu"))
    backend_hashes = {
        "wrapper_sha256": "92ec1c12cc174d549dfdbedf921984784252d66630bdeca32d248085b42c76a1",
        "elf_sha256": "e0ea2b1f5ae2a5aa65a455eb232e46e1743b5c327c48ca678630c214c69f26ff",
        "loader_sha256": "63704534fba7799d309e9e16992ea7d11e709906bd44b72060c007461b970458",
    }
    if qemu.get("identity_ok") is not True:
        errors.append(f"{label}: QEMU backend identity is incomplete")
    for key, expected_hash in backend_hashes.items():
        if qemu.get(key) != expected_hash:
            errors.append(f"{label}: QEMU {key} is {qemu.get(key)!r}, expected {expected_hash}")
    runtime_libs = qemu.get("runtime_libs")
    if not isinstance(runtime_libs, list) or not any(
            isinstance(item, dict)
            and str(item.get("path", "")).endswith("/libc.so.6")
            and item.get("sha256") ==
            "eef95f26903e6f9ac6b8cb92df9ba5306ff01b4058ddf7f9ad6aaa986bb7f35f"
            for item in runtime_libs):
        errors.append(f"{label}: isolated QEMU libc identity is missing or changed")

    compiler_ids = as_dict(data.get("compiler_identities"))
    compiler_key = "gcc" if expected_compiler == "gcc" else "clang_via_zig"
    compiler_line = compiler_ids.get(compiler_key)
    if not isinstance(compiler_line, str) or "MISSING" in compiler_line:
        errors.append(f"{label}: {compiler_key} compiler identity is missing")
    rustc_line = compiler_ids.get("rustc")
    if not isinstance(rustc_line, str) or "rustc 1.97.0" not in rustc_line:
        errors.append(f"{label}: private Rust compiler identity is missing or unexpected")

    contract = as_dict(data.get("fixture_contract"))
    if contract.get("required_pages") != 301 \
            or contract.get("pixel_offset_bytes") != 4096 \
            or contract.get("pixel_bytes") != 640 * 480 * 4:
        errors.append(f"{label}: fixture page/layout contract is inconsistent")
    if contract.get("flags") != 25 or contract.get("requested_capabilities") != 0:
        errors.append(f"{label}: signed fixture changed the existing flags/permission requests")
    if contract.get("new_kernel_syscalls") != [] \
            or contract.get("added_child_capabilities") != []:
        errors.append(f"{label}: fixture claims a new syscall or child capability")

    artifacts = data.get("artifacts")
    fixtures = data.get("fixtures")
    if not isinstance(artifacts, dict) or not isinstance(fixtures, dict):
        return errors + [f"{label}: per-page artifact or run records are missing"]

    def check_recorded_file(entry: object, name: str) -> None:
        if not isinstance(entry, dict):
            errors.append(f"{label}: {name} identity is missing")
            return
        resolved = _integration_file(evidence_dir, entry.get("path"))
        if resolved is None:
            errors.append(f"{label}: {name} file is missing or outside evidence_dir")
        elif sha256_file(resolved) != entry.get("sha256"):
            errors.append(f"{label}: {name} digest does not match its file")

    for pages, expected_exit in ((301, 0), (300, 7)):
        key = str(pages)
        artifact = artifacts.get(key)
        fixture = fixtures.get(key)
        if not isinstance(artifact, dict) or not isinstance(fixture, dict):
            errors.append(f"{label}: missing {pages}-page artifact/run record")
            continue
        if artifact.get("application_id") != f"com.arena.layout{pages}" \
                or fixture.get("application_id") != f"com.arena.layout{pages}":
            errors.append(f"{label}: {pages}-page app identity is not the fixed fixture ID")
        if artifact.get("manifest_flags") != 25 \
                or artifact.get("requested_capabilities") != 0 \
                or fixture.get("manifest_flags") != 25 \
                or fixture.get("requested_capabilities") != 0:
            errors.append(f"{label}: {pages}-page manifest flags or permission requests changed")
        if fixture.get("expected_pages") != pages \
                or fixture.get("allocated_pages") != pages \
                or fixture.get("client_reported_pages") != pages \
                or fixture.get("retired_pages") != pages:
            errors.append(f"{label}: {pages}-page case is not bound to an exact real grant")
        if fixture.get("client_exit_status") != expected_exit:
            errors.append(f"{label}: {pages}-page child exit status is not {expected_exit}")
        if fixture.get("install_signed_version") != 1 \
                or not isinstance(fixture.get("install_receipt"), str) \
                or "APB1 installed; signed version=1" not in fixture.get("install_receipt", ""):
            errors.append(f"{label}: {pages}-page case lacks the protected signed-install receipt")
        if fixture.get("precreation_suppressed") is not True \
                or fixture.get("settled") is not True \
                or fixture.get("no_window_after_cleanup") is not True \
                or fixture.get("resource_drift") != {}:
            errors.append(f"{label}: {pages}-page case did not prove precreation suppression and cleanup")

        authority = as_dict(fixture.get("authority_contract"))
        if authority.get("added_application_capabilities") != [] \
                or authority.get("child_flags") != 25 \
                or authority.get("identify_required_on_success") is not True:
            errors.append(f"{label}: {pages}-page case changed the signed launch/capability contract")

        check_recorded_file(artifact.get("elf"), f"{pages}-page ELF")
        check_recorded_file(artifact.get("apb1"), f"{pages}-page APB1")
        apb1_entry = as_dict(artifact.get("apb1"))
        elf_entry = as_dict(artifact.get("elf"))
        if apb1_entry.get("sha256") != fixture.get("apb1_sha256"):
            errors.append(f"{label}: {pages}-page APB1 digest differs between artifact and run")
        if elf_entry.get("sha256") != fixture.get("elf_sha256"):
            errors.append(f"{label}: {pages}-page ELF digest differs between artifact and run")

        format_boot = as_dict(fixture.get("format_boot"))
        if format_boot.get("rc") != 0:
            errors.append(f"{label}: {pages}-page AFS2 format boot did not exit 0")
        format_path = _integration_file(evidence_dir, format_boot.get("serial_path"))
        if format_path is None or sha256_file(format_path) != format_boot.get("serial_sha256"):
            errors.append(f"{label}: {pages}-page format serial is missing or changed")
        elif "filesd: AFS2 mounted" not in format_path.read_text(errors="replace"):
            errors.append(f"{label}: {pages}-page format boot did not mount AFS2")

        serial_entry = as_dict(fixture.get("serial"))
        serial_path = _integration_file(evidence_dir, serial_entry.get("path"))
        if serial_path is None or sha256_file(serial_path) != serial_entry.get("sha256"):
            errors.append(f"{label}: {pages}-page guest serial is missing or changed")
            continue
        serial = serial_path.read_text(errors="replace")
        if as_dict(fixture.get("qemu")).get("rc") != 0:
            errors.append(f"{label}: {pages}-page QEMU run did not exit 0")
        allocation = re.findall(
            rf"\[desktop\] layout fixture SharedRegion allocated "
            rf"app=com\.arena\.layout{pages} pages=(\d+) surface=(\d+)", serial)
        if len(allocation) != 1 or int(allocation[0][0]) != pages:
            errors.append(f"{label}: serial does not prove exactly one real {pages}-page grant")
            surface = None
        else:
            surface = int(allocation[0][1])
        if not re.search(
                rf"\[desktop\] layout fixture console precreation suppressed "
                rf"app=com\.arena\.layout{pages}", serial):
            errors.append(f"{label}: serial lacks the no-precreated-window receipt")
        if not re.search(rf"\[gfx\] surface pages={pages} required=301", serial):
            errors.append(f"{label}: C client did not query exactly {pages} pages")
        if not re.search(
                rf"\[desktop\] layout fixture pixels surface={surface} "
                rf"pages={pages} words=(\d+) nonzero=(\d+)", serial):
            errors.append(f"{label}: Desktop did not report pixel backing at retirement")
        pixel_match = re.search(
            rf"\[desktop\] layout fixture pixels surface={surface} "
            rf"pages={pages} words=(\d+) nonzero=(\d+)", serial)
        wanted_words = 307200 if pages == 301 else 306176
        wanted_nonzero = 307200 if pages == 301 else 0
        if pixel_match is None or (int(pixel_match.group(1)), int(pixel_match.group(2))) != \
                (wanted_words, wanted_nonzero):
            errors.append(f"{label}: {pages}-page pixel scan is not the expected value")

        if surface is not None:
            if not re.search(
                    rf"\[desktop\] child Process-cap exit surface={surface} "
                    rf"status={expected_exit}", serial):
                errors.append(f"{label}: child exit receipt does not match surface/status")
            if not re.search(
                    rf"\[desktop\] application retired: kind=6 surface={surface}", serial):
                errors.append(f"{label}: exact surface was not retired")
            if re.search(
                    rf"\[desktop\] ordinary window placed handle=\d+ surface={surface} ",
                    serial) is None and pages == 301:
                errors.append(f"{label}: successful 301-page run has no placed window")
            if pages == 300 and re.search(
                    rf"\[desktop\] ordinary window placed handle=\d+ surface={surface} ",
                    serial):
                errors.append(f"{label}: refused 300-page run unexpectedly created a window")

        if pages == 301:
            ordered = (
                "[gfx] surface pages=301 required=301",
                "[gfx] size-check accepted; map request next",
                "[gfx] shared-map syscall",
                "[gfx] surface mapped",
                "[gfx] identify ok surface=",
                "[gfx] create-window request after Identify",
                "[gfx] window created handle=",
                "[gfx] first frame pixels written=307200",
                "[gfx] first damage published",
            )
            lines = serial.splitlines()
            positions = []
            for marker in ordered:
                needs_surface = "surface=" in marker
                hit = next((i for i, line in enumerate(lines)
                            if marker in line
                            and (not needs_surface or surface is None
                                 or f"surface={surface}" in line)), None)
                positions.append(hit)
            if any(position is None for position in positions) \
                    or positions != sorted(positions):
                errors.append(f"{label}: 301 C size/map/Identify/window/write/damage order is not proven")
            if fixture.get("identify_surface_object_id") != surface \
                    or fixture.get("window_created") is not True \
                    or fixture.get("rendered_visual_assertion") is not True \
                    or fixture.get("pixel_nonzero_words") != 307200:
                errors.append(f"{label}: 301 success, Identify, render, or damage assertions are missing")
            if re.search(r"\[gfx\] exit events=\d+ focused=-?\d+ code=0 "
                         rf"surface={surface}", serial) is None:
                errors.append(f"{label}: 301 C client did not report clean exit")
        else:
            query = serial.find("[gfx] surface pages=300 required=301")
            refused = serial.find("[gfx] size-check refused before map/window/damage; exit=7")
            if query < 0 or refused < 0 or query >= refused:
                errors.append(f"{label}: 300 C size refusal does not follow the actual page query")
            forbidden = (
                "[gfx] size-check accepted; map request next",
                "[gfx] shared-map syscall",
                "[gfx] surface mapped",
                "[gfx] map failed",
                "[gfx] identify ok",
                "[gfx] create-window request after Identify",
                "[gfx] window created",
                "[gfx] first frame pixels written",
                "[gfx] first damage published",
            )
            if any(marker in serial for marker in forbidden):
                errors.append(f"{label}: 300 C refusal crossed map/write/window/damage")
            if fixture.get("identify_surface_object_id") is not None \
                    or fixture.get("window_created") is not False \
                    or fixture.get("no_map_write_window_damage_before_refusal") is not True \
                    or fixture.get("pixel_nonzero_words") != 0:
                errors.append(f"{label}: 300 refusal/zero-write assertions are incomplete")

        required_frames = ("prelaunch", "launcher-selection", "after-cleanup")
        if pages == 301:
            required_frames += ("rendered",)
        frames = fixture.get("frames")
        if not isinstance(frames, dict):
            errors.append(f"{label}: {pages}-page screenshot map is missing")
            continue
        for frame_name in required_frames:
            frame = frames.get(frame_name)
            if not isinstance(frame, dict):
                errors.append(f"{label}: {pages}-page screenshot {frame_name} is missing")
                continue
            png_path = _integration_file(evidence_dir, frame.get("png"))
            if png_path is None or sha256_file(png_path) != frame.get("png_sha256"):
                errors.append(f"{label}: {pages}-page screenshot {frame_name} is missing or changed")
            if frame.get("width") != 800 or frame.get("height") != 600 \
                    or not isinstance(frame.get("raster_sha256"), str) \
                    or len(frame.get("raster_sha256", "")) != 64:
                errors.append(f"{label}: {pages}-page screenshot {frame_name} has invalid frame identity")

    return errors

def evaluate_requirements(mode: str, receipts: dict[str, dict | None],
                          invalid_stages: set[str],
                          unqualified_stages: set[str]) -> tuple[list[dict], list[str]]:
    """Return requirement statuses and hard errors for requirements in mode."""
    included = set(STAGES[mode])
    results, errors = [], []
    for req_id, requirement in REQUIREMENTS.items():
        stages = requirement["stages"]
        omitted = [stage for stage in stages if stage not in included]
        if omitted:
            status = "PARTIAL" if any(stage in included for stage in stages) else "NOT RUN"
            detail = "mode omits " + ", ".join(omitted)
            results.append({"id": req_id, "label": requirement["label"],
                            "status": status, "detail": detail})
            continue

        missing = [stage for stage in stages if receipts.get(stage) is None]
        bad = [stage for stage in stages if stage in invalid_stages]
        unqualified = [stage for stage in stages if stage in unqualified_stages]
        if missing or bad:
            pieces = []
            if missing:
                pieces.append("missing " + ", ".join(missing))
            if bad:
                pieces.append("invalid " + ", ".join(bad))
            status, detail = "FAIL", "; ".join(pieces)
            errors.append(f"requirement {req_id}: {detail}")
        elif unqualified:
            status = "BLOCKED"
            detail = "unqualified stage(s): " + ", ".join(unqualified)
        else:
            detail = "all mapped stage receipts are current and qualified"
            validator = requirement.get("validator")
            if validator in ("layout-boundary", "real-layout-grant"):
                evidence_errors = []
                check = (layout_evidence_problems if validator == "layout-boundary"
                         else layout_integration_evidence_problems)
                for stage in stages:
                    evidence_errors.extend(check(receipts[stage], stage))
                if evidence_errors:
                    status = "FAIL"
                    detail = "; ".join(evidence_errors)
                    errors.extend(evidence_errors)
                else:
                    status = "PASS"
                    if validator == "layout-boundary":
                        detail = "301/300 C predicates are separate from the matrix's 471-page grant; no exact boundary grant"
                    else:
                        detail = "both compilers prove real 301-page grant and pre-map 300-page refusal"
            elif validator == "foreign-capability":
                evidence_errors = []
                waiver_results = []
                for stage in stages:
                    stage_errors, waived = foreign_capability_evidence_problems(
                        receipts[stage], stage)
                    evidence_errors.extend(stage_errors)
                    waiver_results.append(waived)
                if evidence_errors:
                    status = "FAIL"
                    detail = "; ".join(evidence_errors)
                    errors.extend(evidence_errors)
                elif all(waiver_results):
                    status = "PASS"
                    detail = "M recorded a waiver scoped to the unchanged foreign/stale capability gap"
                else:
                    status = "OPEN"
                    detail = ("foreign-identity remains an IDENTITY validator test over mutated bytes; "
                              "no real stale/foreign capability or M-WAIVER.json is recorded")
            else:
                status = "PASS"
        results.append({"id": req_id, "label": requirement["label"],
                        "status": status, "detail": detail})
    return results, errors


def main(argv: list) -> int:
    mode = "guest"
    receipts_dir = DEFAULT_RECEIPTS
    i = 1
    while i < len(argv):
        if argv[i] == "--mode":
            if i + 1 >= len(argv):
                print("--mode requires a value", file=sys.stderr)
                return 2
            mode = argv[i + 1]
            i += 2
        elif argv[i] == "--receipts":
            if i + 1 >= len(argv):
                print("--receipts requires a directory", file=sys.stderr)
                return 2
            receipts_dir = Path(argv[i + 1])
            i += 2
        else:
            print(f"unknown argument {argv[i]!r}", file=sys.stderr)
            return 2
    if mode not in STAGES:
        print(f"unknown mode {mode!r}; expected one of {sorted(STAGES)}",
              file=sys.stderr)
        return 2

    required = STAGES[mode]
    current_commit = _git("rev-parse", "HEAD")
    current_code = code_fingerprint()
    current_efi = sha256_file(REPO_ROOT / "build" / "arena-boot.efi")
    problems, stage_errors = [], set()
    receipts = {}
    unqualified_stages = set()

    for stage in required:
        receipt = load(receipts_dir, stage)
        receipts[stage] = receipt
        if receipt is None:
            problems.append(f"{stage}: no valid receipt (stage did not run or JSON is invalid)")
            stage_errors.add(stage)
            continue
        if receipt.get("stage") != stage:
            problems.append(f"{stage}: receipt identifies stage {receipt.get('stage')!r}")
            stage_errors.add(stage)
        if receipt.get("exit") != 0:
            problems.append(f"{stage}: stage failed (exit {receipt.get('exit')})")
            stage_errors.add(stage)
        # Commit id alone is the wrong test in both directions: a documentation
        # commit moves HEAD without changing executed bytes, while an
        # uncommitted edit changes bytes without touching HEAD.
        if receipt.get("code_fingerprint") != current_code:
            problems.append(
                f"{stage}: ran against code {str(receipt.get('code_fingerprint'))[:12]}"
                f" (commit {str(receipt.get('source_commit'))[:12]}), candidate is"
                f" {current_code[:12]} (commit {current_commit[:12]})")
            stage_errors.add(stage)
        if receipt.get("efi_sha256") != current_efi:
            problems.append(
                f"{stage}: tested EFI {str(receipt.get('efi_sha256'))[:12]}, "
                f"candidate ships {current_efi[:12]}")
            stage_errors.add(stage)
        if receipt.get("qualified", True) is False:
            unqualified_stages.add(stage)
        newest = (receipt.get("newest_tracked") or {}).get("mtime")
        finished = receipt.get("finished_epoch")
        if newest is not None and finished is not None and finished < newest:
            problems.append(
                f"{stage}: receipt ({receipt.get('finished_utc')}) is older "
                f"than newest tracked source "
                f"({(receipt.get('newest_tracked') or {}).get('path')}); stale receipt")
            stage_errors.add(stage)

    requirements, requirement_errors = evaluate_requirements(
        mode, receipts, stage_errors, unqualified_stages)
    problems.extend(requirement_errors)

    print("stage                 result   finished")
    for name in required:
        receipt = receipts.get(name)
        result = "MISSING" if receipt is None else (
            "PASS" if receipt.get("exit") == 0 and name not in stage_errors else "FAIL")
        finished = receipt.get("finished_utc", "-") if receipt else "-"
        print(f"{name:<21} {result:<8} {finished}")

    print("\nrequirement  result     coverage")
    for item in requirements:
        print(f"{item['id']:<12} {item['status']:<10} {item['label']}")
        if item["status"] in ("FAIL", "BLOCKED", "OPEN"):
            print(f"             detail: {item['detail']}")

    foreign_status = next((item for item in requirements if item["id"] == "4b"), None)
    if foreign_status and foreign_status["status"] == "PASS":
        print("\nCOVERAGE LIMITATION: foreign-identity is still an IDENTITY-class "
              "validator test over mutated bytes, not a real stale/foreign "
              "capability launch; an M-recorded waiver is present.")
    else:
        print("\nOPEN LIMITATION: foreign-identity is an IDENTITY-class validator test "
              "over mutated identity bytes, not a real stale/foreign capability "
              "launch. No M-recorded waiver is present in this gate evidence.")

    missing_all = [stage for stage in STAGES["full"] if stage not in required]
    if problems:
        print(f"\nV1 GATE ({mode}): FAIL")
        for problem in problems:
            print(f"  - {problem}")
        return 1

    if unqualified_stages:
        print("\nV1 QUALIFICATION: BLOCKED — one or more required stages ran "
              "through an unqualified substitute:")
        for stage in sorted(unqualified_stages):
            receipt = receipts.get(stage) or {}
            notes = receipt.get("notes") or []
            print(f"  - {stage}: {'; '.join(notes) if notes else 'unqualified'}")
        return 3

    if mode == "full":
        failed_requirements = [r for r in requirements if r["status"] == "FAIL"]
        open_requirements = [r for r in requirements if r["status"] != "PASS"]
        if failed_requirements:
            print("\nV1 GATE (full): FAIL — one or more acceptance requirements are not proven")
            return 1
        if open_requirements:
            print("\nV1 QUALIFICATION: BLOCKED — required coverage remains OPEN or needs "
                  "M's recorded waiver:")
            for item in open_requirements:
                print(f"  - requirement {item['id']}: {item['detail']}")
            return 3
        print("\nV1 QUALIFICATION: PASS — every mapped requirement and required stage "
              f"ran against this tree and boot image ({current_efi[:12]}).")
        return 0

    print(f"\nV1 GATE ({mode}): PASS — PARTIAL. This is not a V1 qualification "
          "result.")
    print("  Not run (required for full qualification): "
          + ", ".join(f"{stage} ({STAGE_PURPOSE[stage]})" for stage in missing_all))
    return 0


def code_fingerprint() -> str:
    """Same rule the stage writer used; see tools/v1_gate_stage.py."""
    import hashlib
    tracked = _git("ls-files", "-z")
    if tracked in ("", "unavailable"):
        return "unavailable"
    digest = hashlib.sha256()
    for rel in sorted(x for x in tracked.split("\0") if x):
        if rel.startswith("docs/v1/evidence/"):
            continue
        if rel.startswith("docs/") or rel.endswith(".md"):
            continue
        digest.update(rel.encode())
        digest.update(b"\0")
        try:
            digest.update(hashlib.sha256((REPO_ROOT / rel).read_bytes()).digest())
        except OSError:
            digest.update(b"absent")
        digest.update(b"\0")
    return digest.hexdigest()


def _git(*args: str) -> str:
    import subprocess
    try:
        out = subprocess.run(["git", *args], cwd=REPO_ROOT, capture_output=True,
                             text=True, check=False, timeout=60)
        return out.stdout.strip()
    except (OSError, subprocess.SubprocessError):
        return "unavailable"


if __name__ == "__main__":
    sys.exit(main(sys.argv))
