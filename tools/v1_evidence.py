#!/usr/bin/env python3
"""Package and verify a V1 qualification evidence directory.

    python3 tools/v1_evidence.py package docs/v1/evidence/v2-2026-10-29
    python3 tools/v1_evidence.py verify docs/v1/evidence/v2-2026-10-29.tar.gz

`package` writes `<dir>/MANIFEST.json` (every file, its size and SHA-256),
`<dir>/README.md` (what the run is and how to check it), and a
`<dir>.tar.gz` archive that a reader can fetch later.

`verify` extracts the archive to a fresh temporary directory — NOT the
original workspace — and re-checks every digest. It fails if any file is
missing, has a different size or digest, or is present in the archive but
absent from the manifest (an unlisted file is unverifiable evidence).

The point is not the archive format. It is that a reader with only the
tarball can independently confirm that the serial logs, screenshots,
receipts and hashes are the ones the verdict was computed from.
"""

import hashlib
import json
import subprocess
import sys
import tarfile
import tempfile
import time
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent


def sha256_bytes(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def sha256_file(path: Path) -> str:
    return sha256_bytes(path.read_bytes())


def _git(*args: str) -> str:
    try:
        out = subprocess.run(["git", *args], cwd=REPO_ROOT, capture_output=True,
                             text=True, check=False, timeout=30)
        return out.stdout.strip()
    except (OSError, subprocess.SubprocessError):
        return "unavailable"


def collect(root: Path) -> dict:
    files = {}
    for path in sorted(root.rglob("*")):
        if not path.is_file():
            continue
        rel = str(path.relative_to(root))
        if rel in ("MANIFEST.json", "README.md"):
            continue  # generated below, and named in their own manifest
        data = path.read_bytes()
        files[rel] = {"sha256": sha256_bytes(data), "size": len(data)}
    return files


def readme(root: Path, manifest: dict) -> str:
    files = manifest["files"]
    serials = [f for f in files if f.endswith(".log")]
    shots = [f for f in files if f.endswith(".ppm")]
    return f"""# {root.name} — V1 qualification evidence

Produced {manifest["created_utc"]} from commit
`{manifest["source_commit"]}` on branch `{manifest["branch"]}`
(working tree dirty: `{manifest["working_tree_dirty"]}`).

## What is here

| Kind | Count | Notes |
|---|---|---|
| Serial logs | {len(serials)} | The guest's own output under QEMU TCG |
| Screenshots | {len(shots)} | PPM frames captured through the emulator console |
| Manifests/receipts | {len(files) - len(serials) - len(shots)} | JSON verdicts, identities, build receipts |

## How to check it

Extract the archive anywhere and verify every recorded digest from a fresh
directory (nothing is compared against the original workspace):

```sh
tar xzf {root.name}.tar.gz
python3 - <<'PY'
import hashlib, json, pathlib
root = pathlib.Path("{root.name}")
m = json.loads((root / "MANIFEST.json").read_text())
bad = []
for rel, want in m["files"].items():
    p = root / rel
    if not p.is_file():
        bad.append((rel, "missing")); continue
    got = hashlib.sha256(p.read_bytes()).hexdigest()
    if got != want["sha256"] or p.stat().st_size != want["size"]:
        bad.append((rel, f"sha256 {got} != {want['sha256']}"))
print("files:", len(m["files"]), "mismatches:", len(bad))
for b in bad:
    print(" ", b)
raise SystemExit(1 if bad else 0)
PY
```

`tools/v1_evidence.py verify <archive>` performs exactly this check, plus a
scan for files that exist in the archive but not in the manifest.

## How the verdicts were produced

- `guest-gcc/`, `guest-clang/`: `tools/v1_guest_proof.py` — eight
  checkpoints; `evidence.json` is the run's own verdict, `harness-output.log`
  is the transcript, `v1-guest-serial.log` is the guest serial, `shots/` are
  the frames, `qemu-stderr.log` is the emulator's stderr.
- `lifecycle-cycles/`: `tools/v1_cycles.py` — ten launch/close cycles, each
  with its own cursor, instance-specific retirement and exit receipts,
  settlement against the pre-cycle baseline, and visual removal.
- `env/`: environment recovery receipts (emulator identity, loader, TCG
  acceleration, toolchain versions, protected guards).

A missing serial log, a missing harness transcript, an absent identity or a
missing required artifact makes the run FAIL — see
`docs/v1/V2-STATUS.md` §6.
"""


def package(root: Path) -> int:
    root = root.resolve()
    if not root.is_dir():
        print(f"[evidence] {root} is not a directory", file=sys.stderr)
        return 2
    files = collect(root)
    manifest = {
        "created_utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "source_commit": _git("rev-parse", "HEAD"),
        "branch": _git("rev-parse", "--abbrev-ref", "HEAD"),
        "working_tree_dirty": bool(_git("status", "--porcelain")),
        "files": files,
        "total_bytes": sum(f["size"] for f in files.values()),
    }
    (root / "MANIFEST.json").write_text(json.dumps(manifest, indent=2) + "\n")
    (root / "README.md").write_text(readme(root, manifest))
    archive = root.with_suffix(".tar.gz")
    with tarfile.open(archive, "w:gz") as tar:
        tar.add(root, arcname=root.name)
    print(f"[evidence] packaged {len(files)} files, "
          f"{manifest['total_bytes']} bytes -> {archive} "
          f"({archive.stat().st_size} bytes, sha256 {sha256_file(archive)})")
    (archive.with_suffix(".tar.gz.sha256")).write_text(
        f"{sha256_file(archive)}  {archive.name}\n")
    return 0


def verify(archive: Path) -> int:
    archive = archive.resolve()
    if not archive.is_file():
        print(f"[evidence] {archive} not found", file=sys.stderr)
        return 2
    with tempfile.TemporaryDirectory(prefix="v1-evidence-verify-") as tmp:
        with tarfile.open(archive, "r:gz") as tar:
            tar.extractall(tmp, filter="data")
        roots = [p for p in Path(tmp).iterdir() if p.is_dir()]
        if len(roots) != 1:
            print(f"[evidence] expected one root directory, found {len(roots)}")
            return 1
        root = roots[0]
        manifest_path = root / "MANIFEST.json"
        if not manifest_path.is_file():
            print("[evidence] MANIFEST.json missing from archive")
            return 1
        manifest = json.loads(manifest_path.read_text())
        problems = []
        for rel, want in manifest["files"].items():
            path = root / rel
            if not path.is_file():
                problems.append(f"{rel}: missing")
                continue
            data = path.read_bytes()
            if len(data) != want["size"]:
                problems.append(f"{rel}: size {len(data)} != {want['size']}")
            elif sha256_bytes(data) != want["sha256"]:
                problems.append(f"{rel}: sha256 mismatch")
        listed = set(manifest["files"])
        for path in root.rglob("*"):
            if not path.is_file():
                continue
            rel = str(path.relative_to(root))
            if rel in ("MANIFEST.json", "README.md"):
                continue
            if rel not in listed:
                problems.append(f"{rel}: present in archive, absent from manifest")
        print(f"[evidence] extracted to a fresh directory: {root}")
        print(f"[evidence] files checked: {len(manifest['files'])}, "
              f"problems: {len(problems)}")
        for p in problems:
            print(f"[evidence] PROBLEM: {p}")
        return 0 if not problems else 1


def main(argv: list[str]) -> int:
    if len(argv) != 3 or argv[1] not in ("package", "verify"):
        print(__doc__, file=sys.stderr)
        return 2
    target = Path(argv[2])
    return package(target) if argv[1] == "package" else verify(target)


if __name__ == "__main__":
    sys.exit(main(sys.argv))
