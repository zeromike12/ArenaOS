#!/usr/bin/env python3
"""Requirement-aware V1 gate aggregator.

A green stage is not qualification by itself. The full result is derived
from the requirement matrix below, its required stage receipts, and the
structured guest evidence for requirement 5e. A historical set of stage PASS
labels cannot stand in for a requirement that the current candidate did not
prove.

    python3 tools/v1_gate_aggregate.py --mode guest  # review-candidate subset
    python3 tools/v1_gate_aggregate.py --mode full   # every requirement
"""

import json
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
        "label": "Guest production C layout policy: 301 accepted, 300 refused",
        "stages": ("refusals-gcc", "refusals-clang"),
        "validator": "layout-boundary",
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
    "refusals-gcc": "guest refusal matrix, GCC client",
    "refusals-clang": "guest refusal matrix, Clang client",
    "cycles": "ten launch/close lifecycle cycles",
    "package": "evidence manifest + archive",
    "verify": "evidence verifies from a fresh extraction",
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
            if requirement.get("validator") == "layout-boundary":
                evidence_errors = []
                for stage in stages:
                    evidence_errors.extend(layout_evidence_problems(receipts[stage], stage))
                if evidence_errors:
                    status = "FAIL"
                    detail = "; ".join(evidence_errors)
                    errors.extend(evidence_errors)
                else:
                    status = "PASS"
                    detail = "both compiler cells prove the production 301/300 policy and real grant"
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
        if item["status"] in ("FAIL", "BLOCKED"):
            print(f"             detail: {item['detail']}")

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
        failed_requirements = [r for r in requirements if r["status"] != "PASS"]
        if failed_requirements:
            print("\nV1 GATE (full): FAIL — one or more acceptance requirements are not proven")
            return 1
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
