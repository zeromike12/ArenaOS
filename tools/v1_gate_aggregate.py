#!/usr/bin/env python3
"""Decide whether the gate that just ran actually qualifies this candidate.

    python3 tools/v1_gate_aggregate.py --mode guest    # partial by design
    python3 tools/v1_gate_aggregate.py --mode full     # qualification

A stage PASS on its own is not a qualification. This step is what turns a
pile of green logs into a defensible claim, and it refuses the claim when:

  * a required stage has no receipt (it never ran),
  * a required stage's receipt says it failed,
  * a receipt was produced from a different commit than HEAD,
  * a receipt describes a different boot image than the one on disk now
    (the tree was rebuilt after the stage ran, so the stage tested an
    artifact this candidate no longer ships),
  * a receipt is older than the newest tracked source file — that is a
    stale receipt from an earlier tree, which must never be allowed to
    satisfy a new candidate.

Only `--mode full` (every stage, including the formal regression suite and
the runtime-stability loop) can print `V1 QUALIFICATION: PASS`. Any other
mode is reported as a partial gate and says exactly what is missing.
"""

import json
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
DEFAULT_RECEIPTS = Path("build/v1-gate/receipts")

# `quick` exists so `v1_gate.sh`'s development mode has an aggregate too;
# it can only ever report a partial result. `guest` is the review-candidate
# gate: every V1-specific execution. `full` adds the two project-wide gates
# (121-suite regression, boot loop).
STAGES = {
    "quick": ["build", "env", "controls"],
    "guest": [
        "build", "env", "controls", "guest-gcc", "guest-clang",
        "refusals-gcc", "refusals-clang", "cycles", "package", "verify",
    ],
    "full": [
        "build", "env", "controls", "guest-gcc", "guest-clang",
        "refusals-gcc", "refusals-clang", "cycles", "package", "verify",
        "regression", "stability",
    ],
}

STAGE_PURPOSE = {
    "build": "boot image built from this tree",
    "env": "environment receipts (emulator, loader, toolchains, guards)",
    "controls": "focused controls through production logic",
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
        return json.loads(path.read_text())
    except (OSError, ValueError):
        return None


def main(argv: list) -> int:
    mode = "guest"
    receipts = DEFAULT_RECEIPTS
    i = 1
    while i < len(argv):
        if argv[i] == "--mode":
            mode = argv[i + 1]
            i += 2
        elif argv[i] == "--receipts":
            receipts = Path(argv[i + 1])
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
    problems, rows, unqualified = [], [], []

    for stage in required:
        receipt = load(receipts, stage)
        if receipt is None:
            problems.append(f"{stage}: no receipt (stage did not run)")
            rows.append((stage, "MISSING", "-"))
            continue
        if receipt.get("exit") != 0:
            problems.append(f"{stage}: stage failed (exit {receipt.get('exit')})")
        # Commit id alone would be the wrong test in both directions: a
        # documentation commit moves HEAD without changing a byte that ran,
        # and an uncommitted edit changes the bytes with HEAD fixed. Compare
        # the digest of the tracked code instead, and keep the commit in the
        # message as provenance.
        if receipt.get("code_fingerprint") != current_code:
            problems.append(
                f"{stage}: ran against code {str(receipt.get('code_fingerprint'))[:12]}"
                f" (commit {str(receipt.get('source_commit'))[:12]}), candidate is"
                f" {current_code[:12]} (commit {current_commit[:12]})")
        if receipt.get("efi_sha256") != current_efi:
            problems.append(
                f"{stage}: tested EFI {str(receipt.get('efi_sha256'))[:12]}, "
                f"candidate ships {current_efi[:12]}")
        if receipt.get("qualified", True) is False:
            unqualified.append((stage, receipt.get("notes") or []))
        newest = (receipt.get("newest_tracked") or {}).get("mtime")
        finished = receipt.get("finished_epoch")
        if newest and finished and finished < newest:
            problems.append(
                f"{stage}: receipt ({receipt.get('finished_utc')}) is older "
                f"than the newest tracked source "
                f"({(receipt.get('newest_tracked') or {}).get('path')}) — "
                "stale receipt cannot qualify a new candidate")
        rows.append((stage, "PASS" if receipt.get("exit") == 0 else "FAIL",
                     receipt.get("finished_utc", "-")))

    print("stage            result   finished")
    for name, result, when in rows:
        print(f"{name:<16} {result:<8} {when}")

    missing_all = [s for s in STAGES["full"] if s not in required]
    if problems:
        print(f"\nV1 GATE ({mode}): FAIL")
        for problem in problems:
            print(f"  - {problem}")
        return 1

    if unqualified:
        print("\nV1 QUALIFICATION: BLOCKED — the stages below ran, but only "
              "through a documented substitute, so they cannot support a "
              "qualification claim:")
        for stage, notes in unqualified:
            print(f"  - {stage}: {'; '.join(notes) if notes else 'unqualified'}")
        print("  See docs/v1/V2-STATUS.md §4 (blocker B1).")
        if mode != "full":
            print(f"  This {mode}-mode run was also partial; not run: "
                  + ", ".join(s for s in STAGES["full"] if s not in required))
        return 3

    if mode == "full":
        print("\nV1 QUALIFICATION: PASS — every required stage ran against "
              f"this tree and this boot image ({current_efi[:12]}).")
        return 0

    print(f"\nV1 GATE ({mode}): PASS — PARTIAL. This is not a V1 qualification "
          "result.")
    print("  Not run (required for qualification): "
          + ", ".join(f"{s} ({STAGE_PURPOSE[s]})" for s in missing_all))
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
