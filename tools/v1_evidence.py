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
import shutil
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


# Independent restatement of the checkpoint images a guest run must publish.
#
# It is deliberately duplicated here instead of imported from the harness: a
# producer that silently stops publishing a checkpoint must not be able to
# relax the packaging rule that notices. The names cover the four input
# checkpoints (keyboard, mouse, per-instance isolation focus) and the final
# demonstration, which is exactly what M's round-2 requirement #2 asks for.
REQUIRED_GUEST_FRAMES = (
    "shots/cp01-launch-ready-desktop.png",
    "shots/cp03-first-native-c-window.png",
    "shots/cp04-keyboard-response.png",
    "shots/cp05-mouse-response.png",
    "shots/cp06a-two-instances.png",
    "shots/cp06a-focused.png",
    "shots/cp06b-focused.png",
    "shots/cp07-desktop-restored.png",
    "shots/cp08-final-demonstration.png",
)


def png_raster(data: bytes) -> tuple:
    """Decode a PNG written by the harness (8-bit RGB, filter 0) to pixels.

    Implemented here rather than shared with the producer so packaging checks
    the bytes that are actually published.
    """
    import struct
    import zlib

    if not data.startswith(b"\x89PNG\r\n\x1a\n"):
        raise ValueError("not a PNG")
    pos, width, height, depth, colour, idat = 8, None, None, None, None, b""
    while pos < len(data):
        (length,) = struct.unpack(">I", data[pos:pos + 4])
        tag = data[pos + 4:pos + 8]
        payload = data[pos + 8:pos + 8 + length]
        (crc,) = struct.unpack(">I", data[pos + 8 + length:pos + 12 + length])
        if zlib.crc32(tag + payload) & 0xFFFFFFFF != crc:
            raise ValueError(f"PNG chunk {tag!r} CRC mismatch")
        if tag == b"IHDR":
            width, height, depth, colour = struct.unpack(">IIBB", payload[:10])
        elif tag == b"IDAT":
            idat += payload
        pos += 12 + length
    if depth != 8 or colour != 2:
        raise ValueError("unexpected PNG depth/colour type")
    raw = zlib.decompress(idat)
    stride = width * 3
    out = bytearray()
    for y in range(height):
        i = y * (stride + 1)
        if raw[i] != 0:
            raise ValueError(f"unexpected PNG filter {raw[i]} on row {y}")
        out += raw[i + 1:i + 1 + stride]
    return width, height, bytes(out)


def check_frames(root: Path) -> list:
    """Require every checkpoint image, and that it still shows its frame.

    A run that records a verdict but publishes no image for a checkpoint has
    not shown the frame the assertion was made on. Each published PNG must
    decode back to the raster digest the harness recorded at capture time, so
    a re-encoded, resized or substituted image is caught here rather than
    merely checksummed.
    """
    problems = []
    for evidence_path in sorted(root.rglob("evidence.json")):
        run_dir = evidence_path.parent
        rel_dir = str(run_dir.relative_to(root)) or "."
        try:
            payload = json.loads(evidence_path.read_text())
        except (OSError, ValueError) as exc:
            problems.append(f"{rel_dir}: evidence.json unreadable: {exc}")
            continue
        frames = payload.get("frames")
        if not isinstance(frames, dict) or not frames:
            problems.append(f"{rel_dir}: evidence.json records no checkpoint "
                            "frames")
            continue
        required = payload.get("required_frames") or list(REQUIRED_GUEST_FRAMES)
        missing = [n for n in REQUIRED_GUEST_FRAMES if n not in frames]
        if missing:
            problems.append(f"{rel_dir}: evidence.json omits checkpoint "
                            f"frames: {', '.join(missing)}")
        for name in required:
            entry = frames.get(name)
            if entry is None:
                problems.append(f"{rel_dir}: no digest entry for {name}")
                continue
            path = run_dir / name
            if not path.is_file() or path.stat().st_size == 0:
                problems.append(f"{rel_dir}: required checkpoint image "
                                f"{name} is missing or empty")
                continue
            data = path.read_bytes()
            if sha256_bytes(data) != entry.get("published_png_sha256"):
                problems.append(f"{rel_dir}: {name} does not match its "
                                "recorded published digest")
                continue
            try:
                w, h, raster = png_raster(data)
            except (ValueError, OSError) as exc:
                problems.append(f"{rel_dir}: {name} cannot be decoded: {exc}")
                continue
            if sha256_bytes(raster) != entry.get("raster_sha256"):
                problems.append(f"{rel_dir}: {name} decodes to a different "
                                "raster than the tested frame")
            if (w, h) != (entry.get("width"), entry.get("height")):
                problems.append(f"{rel_dir}: {name} is {w}x{h}, recorded "
                                f"{entry.get('width')}x{entry.get('height')}")
    return problems


def collect(root: Path) -> tuple:
    files = {}
    excluded = 0
    for path in sorted(root.rglob("*")):
        if not path.is_file():
            continue
        rel = str(path.relative_to(root))
        if rel in ("MANIFEST.json", "README.md"):
            continue  # generated below, and named in their own manifest
        data = path.read_bytes()
        if rel.endswith(".ppm"):
            # A PPM screendump is a fixed 15-byte header followed by exactly
            # the raster of the published PNG, so it adds no evidence a reader
            # cannot already rebuild from the PNG plus the recorded digests --
            # it only makes the archive several megabytes larger per frame.
            excluded += 1
            continue
        files[rel] = {"sha256": sha256_bytes(data), "size": len(data)}
    return files, excluded


VERIFY_SNIPPET = """import hashlib, json, pathlib
root = pathlib.Path("%(root)s")
m = json.loads((root / "MANIFEST.json").read_text())
bad = []
for rel, want in m["files"].items():
    p = root / rel
    if not p.is_file():
        bad.append((rel, "missing"))
        continue
    got = hashlib.sha256(p.read_bytes()).hexdigest()
    if got != want["sha256"] or p.stat().st_size != want["size"]:
        bad.append((rel, got + " != " + want["sha256"]))
print("files:", len(m["files"]), "mismatches:", len(bad))
for b in bad:
    print(" ", b)
raise SystemExit(1 if bad else 0)
"""


def readme(root: Path, manifest: dict) -> str:
    files = manifest["files"]
    serials = [f for f in files if f.endswith(".log")]
    shots = [f for f in files if f.endswith((".ppm", ".png"))]
    snippet = VERIFY_SNIPPET % {"root": root.name}
    excluded = manifest.get("excluded_from_package", {}).get("count", 0)
    return f"""# {root.name} — V1 evidence package

Produced {manifest["created_utc"]} from commit
`{manifest["source_commit"]}` on branch `{manifest["branch"]}`
(working tree dirty: `{manifest["working_tree_dirty"]}`).

## What is here

This package records evidence and classifications; it does not declare V1
accepted or qualified. Read the current aggregate and status note before using
any result.

| Kind | Count | Notes |
|---|---|---|
| Serial logs | {len(serials)} | The guest's own output under QEMU TCG |
| Screenshots | {len(shots)} | The exact frames each checkpoint assertion was made on (PNG, losslessly re-encoded from the QEMU screendump raster) |
| Manifests and receipts | {len(files) - len(serials) - len(shots)} | JSON verdicts, identities, build receipts |

## How to check it

Extract the archive anywhere and verify every recorded digest from a fresh
directory (nothing is compared against the original workspace):

```sh
tar xzf {root.name}.tar.gz
cd {root.name} && python3 - <<'PY'
{snippet}PY
```

`tools/v1_evidence.py verify <archive>` performs exactly this check, plus a
scan for files that exist in the archive but not in the manifest.

`package` refuses to run unless every guest run in this directory publishes
all nine checkpoint images, each PNG still decoding to the raster digest the
harness recorded when that checkpoint passed.

PPM working copies are not shipped ({excluded} of them): a PPM is a fixed
header plus the identical raster, so it is reproducible from the published
PNG, and each tested PPM's own SHA-256 is recorded in its run's
`evidence.json`.

## How the verdicts were produced

- `guest-gcc/`, `guest-clang/`: `tools/v1_guest_proof.py` — eight
  checkpoints; `evidence.json` is the run's own verdict, `harness-output.log`
  is the transcript, `v1-guest-serial.log` is the guest serial, `shots/` are
  the frames, `qemu-stderr.log` is the emulator's stderr.
- `phase15/`: `tools/test_m15_guest.py` — both final-ESP CPU profiles, with
  serial logs and QEMU stderr tied to the exact EFI/ESP identities in the gate
  receipts.
- `layout-integration-gcc/`, `layout-integration-clang/`:
  `tools/v1_layout_integration.py` — each cell records real Desktop grants of
  301 and 300 pages, the ordinary C client's map/Identify/window/write/damage
  order or pre-map refusal, retirement pixel scans, cleanup settlement, and
  QEMU screenshots tied to the recorded EFI/ESP, compiler, ELF, APB1, and
  emulator identities. These runs are distinct from the predicate-only
  `refusals-*` matrix.
- `lifecycle-cycles/`: `tools/v1_cycles.py` — ten launch/close cycles, each
  with its own cursor, instance-specific retirement and exit receipts,
  settlement against the pre-cycle baseline, and visual removal.
- `gate/`: the command transcript, stage receipts, and relevant host logs
  included before packaging; the `package`/`verify` receipts and final aggregate
  live in the sibling `*-packaging/` directory to avoid recursive self-reference.
- `env/`: environment recovery receipts (emulator identity, loader, TCG
  acceleration, toolchain versions, protected guards).

A missing serial log, a missing harness transcript, an absent identity or a
missing required artifact makes the run FAIL — see `docs/v1/V2-STATUS.md` §6.
"""


def package(root: Path) -> int:
    root = root.resolve()
    if not root.is_dir():
        print(f"[evidence] {root} is not a directory", file=sys.stderr)
        return 2
    frame_problems = check_frames(root)
    if frame_problems:
        print("[evidence] refusing to package: checkpoint frames do not match "
              "their recorded identity")
        for problem in frame_problems:
            print(f"[evidence] PROBLEM: {problem}")
        return 1
    files, excluded_ppm = collect(root)
    # A single multi-megabyte blob in an evidence directory is almost always
    # scratch state (a formatted disk image, a copied boot image), and it
    # bloats the published package without giving a reader anything to check
    # that a recorded SHA-256 does not already give. Refuse rather than
    # quietly publish it.
    oversized = [rel for rel, meta in files.items() if meta["size"] > 32 * 1024 * 1024]
    if oversized:
        print("[evidence] refusing oversized artifacts (scratch, not evidence):")
        for rel in oversized:
            print(f"  {rel}: {files[rel]['size']} bytes")
        return 1
    manifest = {
        "created_utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "source_commit": _git("rev-parse", "HEAD"),
        "branch": _git("rev-parse", "--abbrev-ref", "HEAD"),
        "working_tree_dirty": bool(_git("status", "--porcelain")),
        "files": files,
        "excluded_from_package": {
            "count": excluded_ppm,
            "pattern": "*.ppm",
            "reason": "PPM screendumps are a fixed header plus the exact "
                      "raster of the published PNG; their SHA-256 is "
                      "recorded in each run's evidence.json",
        },
        "checkpoint_frames": {
            "required": list(REQUIRED_GUEST_FRAMES),
            "checked": "each published PNG re-decoded and compared with the "
                       "raster digest the harness recorded when the checkpoint "
                       "assertion passed",
            "runs": sum(1 for _ in root.rglob("evidence.json")),
        },
        "total_bytes": sum(f["size"] for f in files.values()),
    }
    (root / "MANIFEST.json").write_text(json.dumps(manifest, indent=2) + "\n")
    (root / "README.md").write_text(readme(root, manifest))
    archive = root.with_suffix(".tar.gz")
    # Archive exactly what the manifest lists (plus the manifest and README).
    # Adding the directory wholesale would ship the PPM working copies, which
    # `collect` deliberately leaves out -- and an unlisted file in the archive
    # is precisely what `verify` exists to reject.
    with tarfile.open(archive, "w:gz") as tar:
        tar.add(root, arcname=root.name, recursive=False)
        for rel in sorted(files) + ["MANIFEST.json", "README.md"]:
            tar.add(root / rel, arcname=f"{root.name}/{rel}", recursive=False)
    print(f"[evidence] packaged {len(files)} files, "
          f"{manifest['total_bytes']} bytes -> {archive} "
          f"({archive.stat().st_size} bytes, sha256 {sha256_file(archive)})")
    sidecar = archive.parent / (archive.name + ".sha256")
    sidecar.write_text(f"{sha256_file(archive)}  {archive.name}\n")
    return 0


def verify(archive: Path) -> int:
    archive = archive.resolve()
    if not archive.is_file():
        print(f"[evidence] {archive} not found", file=sys.stderr)
        return 2
    with tempfile.TemporaryDirectory(prefix="v1-evidence-verify-") as tmp:
        with tarfile.open(archive, "r:gz") as tar:
            # Reject absolute paths and ".." traversal before extracting;
            # `filter="data"` is not available on every Python version the
            # project supports, so the check is explicit.
            for member in tar.getmembers():
                name = member.name
                if name.startswith("/") or ".." in Path(name).parts:
                    print(f"[evidence] refusing unsafe member: {name}")
                    return 1
            try:
                tar.extractall(tmp, filter="data")
            except TypeError:
                tar.extractall(tmp)
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


def seal(target: Path) -> int:
    """Package, verify from a fresh extraction, then write an EXTERNAL receipt.

    The archive cannot contain a receipt that describes itself: inserting one
    changes the archive, and the digest inside would then describe the
    previous generation. Round 2 shipped exactly that ambiguity. `seal`
    therefore writes `<archive>.receipt.txt` *next to* the archive, after the
    archive is final and verified, and never inside it.
    """
    rc = package(target)
    if rc != 0:
        return rc
    archive = target.with_suffix(".tar.gz")
    rc = verify(archive)
    if rc != 0:
        print("[evidence] seal refused: the package does not verify")
        return rc
    digest = hashlib.sha256(archive.read_bytes()).hexdigest()
    manifest = json.loads((target / "MANIFEST.json").read_text())
    identities = {}
    gate = target / "gate" / "receipts" / "build.json"
    if gate.is_file():
        receipt = json.loads(gate.read_text())
        identities = {k: receipt.get(k) for k in
                      ("source_commit", "code_fingerprint", "efi_sha256",
                       "esp_sha256")}
    lines = [
        "ARENA V1 EVIDENCE — EXTERNAL ARCHIVE RECEIPT",
        "This file is written beside the archive, never inside it, so it",
        "describes the final bytes rather than an earlier generation.",
        "",
        f"archive            : {archive.name}",
        f"archive bytes      : {archive.stat().st_size}",
        f"archive sha256     : {digest}",
        f"verified files     : {len(manifest['files'])}",
        "verification       : python3 tools/v1_evidence.py verify "
        f"{archive}",
        "verification result: PASS — extracted to a fresh temporary "
        "directory, every digest re-checked, no unlisted file",
        "",
        "tested candidates (from the gate build receipt):",
    ]
    for k, v in identities.items():
        lines.append(f"  {k:<18}: {v}")
    lines += [
        "",
        f"sealed             : {time.strftime('%Y-%m-%dT%H:%M:%SZ', time.gmtime())}",
        "command            : python3 tools/v1_evidence.py seal "
        f"{target}",
    ]
    out = archive.with_suffix(archive.suffix + ".receipt.txt")
    out.write_text("\n".join(lines) + "\n")
    print(f"[evidence] external receipt -> {out}\n   {digest}  {len(manifest['files'])} files")
    return 0


def main(argv: list[str]) -> int:
    if len(argv) != 3 or argv[1] not in ("package", "verify", "seal"):
        print(__doc__, file=sys.stderr)
        return 2
    target = Path(argv[2])
    if argv[1] == "seal":
        return seal(target)
    return package(target) if argv[1] == "package" else verify(target)


if __name__ == "__main__":
    sys.exit(main(sys.argv))
