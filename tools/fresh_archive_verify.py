#!/usr/bin/env python3
"""Fresh-extraction verification of a sealed ArenaOS package archive.

Reproduces the Phase 16 fresh-archive flow (see
docs/phase16/receipts/canonical-corrected-5aacbc8-2026-10-26/fresh-archive/)
as one fail-closed command:

  1. check the archive against its external SHA-256 sidecar;
  2. extract to a FRESH directory (short /tmp path so QMP/vcon socket
     paths stay below the 108-byte limit), never over an old tree;
  3. run the package's own verify_package.py (static), then its
     --guest mode with the explicitly selected QEMU (ARENA_QEMU);
  4. require the extracted EFI/ESP and the archive itself to be
     byte-identical afterwards, and copy the guest serials into the
     evidence directory ($ARENA_TEST_ATTEMPT_DIR/fresh-archive, else
     build/fresh-archive-verify/).

The extraction directory is kept for post-mortem and its path printed;
delete it yourself when done. Nothing inside the sealed archive is
modified, and the verification record lives outside it.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import shutil
import subprocess
import sys
import tarfile
import tempfile
import time
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent


def sha256(path: Path) -> str:
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--archive", required=True, type=Path)
    ap.add_argument("--sidecar", type=Path, default=None,
                    help="default: <archive>.sha256 beside the archive")
    ap.add_argument("--extract-parent", type=Path, default=None,
                    help="parent for the fresh extraction dir (default /tmp)")
    args = ap.parse_args()

    archive = args.archive
    sidecar = args.sidecar or Path(str(archive) + ".sha256")
    if not archive.is_file() or not sidecar.is_file():
        print(f"FAIL: archive or sidecar missing: {archive} / {sidecar}",
              file=sys.stderr)
        return 2
    qemu = os.environ.get("ARENA_QEMU")
    if not qemu:
        print("FAIL: ARENA_QEMU must name the explicit (pinned) emulator "
              "for guest verification", file=sys.stderr)
        return 2

    attempt = os.environ.get("ARENA_TEST_ATTEMPT_DIR")
    evidence = (Path(attempt) / "fresh-archive" if attempt
                else REPO / "build/fresh-archive-verify")
    evidence.mkdir(parents=True, exist_ok=True)
    record: dict = {"archive": str(archive), "started_utc":
                    time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())}

    expected = sidecar.read_text().split()[0].strip()
    before = sha256(archive)
    record["archive_sha256_before"] = before
    if before != expected:
        print(f"FAIL: sidecar mismatch: archive {before} != sidecar {expected}",
              file=sys.stderr)
        return 1
    print(f"[fresh-archive] sidecar OK: {before}")

    parent = args.extract_parent
    extract_root = Path(tempfile.mkdtemp(prefix="arena-fresh-",
                                          dir=str(parent) if parent else None))
    record["extraction_root"] = str(extract_root)
    print(f"[fresh-archive] extracting fresh to {extract_root}")
    with tarfile.open(archive) as tf:
        tf.extractall(extract_root, filter="data")
    roots = [p for p in extract_root.rglob("verify_package.py")
             if (p.parent / "MANIFEST.json").is_file()]
    if len(roots) != 1:
        print(f"FAIL: expected exactly one package root, found {roots}",
              file=sys.stderr)
        return 1
    pkg_root = roots[0].parent
    arts = {n: pkg_root / "artifacts" / n
            for n in ("arena-boot.efi", "arena-esp.img")}
    hashes_before = {n: sha256(p) for n, p in arts.items()}
    record["artifacts_before"] = hashes_before

    def run_step(label: str, cmd: list[str], timeout: int) -> int:
        t0 = time.monotonic()
        with open(evidence / f"{label}.log", "wb") as logf:
            rc = subprocess.run(cmd, cwd=pkg_root, stdout=logf,
                                stderr=subprocess.STDOUT,
                                timeout=timeout).returncode
        record[f"{label}_exit"] = rc
        record[f"{label}_duration_s"] = round(time.monotonic() - t0, 1)
        print(f"[fresh-archive] {label}: exit {rc} "
              f"({record[f'{label}_duration_s']}s) log={evidence / (label + '.log')}")
        return rc

    rc = run_step("static", [sys.executable, "verify_package.py"], 300)
    if rc != 0:
        record["verdict"] = "FAIL (static)"
        (evidence / "run.json").write_text(json.dumps(record, indent=2))
        return 1
    rc = run_step("guest", [sys.executable, "verify_package.py", "--guest",
                            "--qemu", qemu], 600)
    if rc != 0:
        record["verdict"] = "FAIL (guest)"
        (evidence / "run.json").write_text(json.dumps(record, indent=2))
        return 1

    for serial in sorted(pkg_root.rglob("serial-phase15-*.log")):
        shutil.copyfile(serial, evidence / serial.name)
    hashes_after = {n: sha256(p) for n, p in arts.items()}
    after = sha256(archive)
    record.update({"artifacts_after": hashes_after,
                   "archive_sha256_after": after})
    if hashes_after != hashes_before or after != before:
        record["verdict"] = "FAIL (artifact changed during verification)"
        (evidence / "run.json").write_text(json.dumps(record, indent=2))
        print("[fresh-archive] " + record["verdict"], file=sys.stderr)
        return 1
    record["verdict"] = "PASS"
    (evidence / "run.json").write_text(json.dumps(record, indent=2))
    print(f"[fresh-archive] PASS; extracted tree kept at {extract_root}; "
          f"evidence in {evidence}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
