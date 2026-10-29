#!/usr/bin/env python3
"""Run one V1 gate stage and write a receipt that says what it tested.

    python3 tools/v1_gate_stage.py run --stage guest-gcc --receipts DIR -- \
        python3 tools/v1_guest_proof.py
    python3 tools/v1_gate_stage.py record --stage stability --receipts DIR \
        --exit 0 --note "100/100 boots, EFI 7134acf1"
    python3 tools/v1_gate_stage.py run --stage build --unqualified \
        --note "dev-path build: canonical PIE guard blocked" -- ...

A gate that reports PASS from a stage run against a *different* tree, or
against a boot image that has since been rebuilt, is a false claim. Every
receipt therefore stamps:

  * the git commit and the dirty paths (evidence directories excluded),
  * the digest of `build/arena-boot.efi` and `build/arena-esp.img` as they
    were when the stage ran,
  * the newest tracked source file and its mtime, so the aggregate step can
    prove the stage is not older than the tree it claims to describe.

`record` exists for stages that are executed outside this wrapper (a
stability loop started by hand, a suite run in another shell). It captures
exactly the same identities, so such a stage is usable *only* while the tree
and the artifacts are the ones it recorded.
"""

import hashlib
import json
import os
import subprocess
import sys
import time
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent

# The runs write their own evidence under this prefix; a file the run just
# produced must not make its own receipt look stale.
EVIDENCE_PREFIXES = ("docs/v1/evidence/",)

# Documentation is not an input to any artifact under test: a receipt may
# describe a tree whose prose was edited afterwards (writing up a result
# cannot change what was executed). Everything that *does* feed a build, a
# test or a fixture stays in the staleness rule.
DOC_PREFIXES = ("docs/",)
DOC_SUFFIXES = (".md",)

# The gate's own receipts live here by default; see --receipts.
DEFAULT_RECEIPTS = Path("build/v1-gate/receipts")


def sha256_file(path: Path) -> str:
    try:
        return hashlib.sha256(path.read_bytes()).hexdigest()
    except OSError:
        return "MISSING"


def git(*args: str) -> str:
    try:
        out = subprocess.run(["git", *args], cwd=REPO_ROOT, capture_output=True,
                             text=True, check=False, timeout=60)
        return out.stdout.strip()
    except (OSError, subprocess.SubprocessError):
        return "unavailable"


def dirty_paths() -> list:
    status = git("status", "--porcelain")
    if status in ("", "unavailable"):
        return []
    paths = []
    for line in status.splitlines():
        rel = line[3:].strip()
        if any(rel.startswith(prefix) for prefix in EVIDENCE_PREFIXES):
            continue
        paths.append(rel)
    return paths


def newest_tracked() -> dict:
    """The newest tracked source file, evidence runs excluded.

    This is the same rule the guest harness applies to its boot image: a
    stage that finished before the newest tracked file was last touched did
    not test the current tree.
    """
    tracked = git("ls-files", "-z")
    if tracked in ("", "unavailable"):
        return {"path": "unavailable", "mtime": None}
    newest, newest_path = 0.0, ""
    for rel in tracked.split("\0"):
        if not rel or any(rel.startswith(p) for p in EVIDENCE_PREFIXES):
            continue
        if any(rel.startswith(p) for p in DOC_PREFIXES) or \
                rel.endswith(DOC_SUFFIXES):
            continue
        try:
            mtime = (REPO_ROOT / rel).stat().st_mtime
        except OSError:
            continue
        if mtime > newest:
            newest, newest_path = mtime, rel
    return {"path": newest_path, "mtime": newest or None}


def receipt(stage: str, exit_code: int, notes: list, argv: list,
            started: float, finished: float, qualified: bool = True) -> dict:
    package = REPO_ROOT / "docs/v1/evidence/v2-2026-10-29-r2.tar.gz"
    return {
        "stage": stage,
        "exit": exit_code,
        # False for a stage that could only run through a documented
        # development substitute (see docs/v1/V2-STATUS.md §B1). Such a
        # receipt never supports a qualification claim, whatever its exit
        # code says.
        "qualified": qualified,
        "notes": notes,
        "argv": argv,
        "started_epoch": started,
        "finished_epoch": finished,
        "started_utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime(started)),
        "finished_utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime(finished)),
        "source_commit": git("rev-parse", "HEAD"),
        "dirty_paths": dirty_paths(),
        "newest_tracked": newest_tracked(),
        "efi_sha256": sha256_file(REPO_ROOT / "build" / "arena-boot.efi"),
        "esp_sha256": sha256_file(REPO_ROOT / "build" / "arena-esp.img"),
        "evidence_package_sha256": (sha256_file(package) if package.is_file()
                                    else "MISSING"),
        "host": os.uname().nodename,
        "python": sys.version.split()[0],
    }


def write(out_dir: Path, data: dict) -> Path:
    out_dir.mkdir(parents=True, exist_ok=True)
    path = out_dir / f"{data['stage']}.json"
    path.write_text(json.dumps(data, indent=2) + "\n")
    return path


def parse(argv: list) -> tuple:
    if len(argv) < 2 or argv[1] not in ("run", "record"):
        print(__doc__, file=sys.stderr)
        raise SystemExit(2)
    mode = argv[1]
    stage, receipts, notes, code = None, DEFAULT_RECEIPTS, [], 0
    qualified = True
    rest = argv[2:]
    i = 0
    while i < len(rest):
        arg = rest[i]
        if arg == "--stage":
            stage = rest[i + 1]
            i += 2
        elif arg == "--receipts":
            receipts = Path(rest[i + 1])
            i += 2
        elif arg == "--note":
            notes.append(rest[i + 1])
            i += 2
        elif arg == "--exit":
            code = int(rest[i + 1])
            i += 2
        elif arg == "--unqualified":
            qualified = False
            i += 1
        elif arg == "--":
            i += 1
            break
        else:
            raise SystemExit(f"unknown argument {arg!r}")
    if stage is None:
        raise SystemExit("--stage is required")
    return mode, stage, receipts, notes, code, qualified, rest[i:]


def main(argv: list) -> int:
    mode, stage, receipts, notes, code, qualified, command = parse(argv)
    started = time.time()
    if mode == "run":
        if not command:
            raise SystemExit("run needs a command after --")
        completed = subprocess.run(command, cwd=REPO_ROOT)
        code = completed.returncode
    finished = time.time()
    data = receipt(stage, code, notes, command, started, finished, qualified)
    path = write(receipts, data)
    print(f"[gate-stage] {stage}: exit {code} -> {path}")
    return code


if __name__ == "__main__":
    sys.exit(main(sys.argv))
