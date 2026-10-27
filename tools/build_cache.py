#!/usr/bin/env python3
"""Content-addressed whole-image build cache for the ArenaOS test harness.

Why this exists: a complete registered run invokes ``mtest.build()``
(which runs ``tools/build.sh --image`` end to end) well over a hundred
times against *identical* inputs. Cargo already makes the compile steps
incremental, but the traversal itself (two dozen cargo invocations, the
Phase 14 fixture guard, the kernel ISA audit, ESP imaging) costs several
seconds per call and dominates feedback time. This module lets
``mtest.build()`` reuse the exact artifacts of a previous real build —
but only when identity is established from *content*, never from a Git
commit or timestamps:

* The fingerprint is a SHA-256 over every build-input file's bytes
  (source trees that ``build.sh`` compiles or embeds, the build scripts
  themselves), the effective build environment (graphics fixture,
  profile, Phase 15 diagnostic feature switches), the absolute build
  path (protected reproduction is absolute-path sensitive), and the
  toolchain identity (rustc verbose version, sysroot ``libcore`` hashes
  for both bare-metal targets, ``rust-lld`` hash, Python version).
  A dirty worktree simply yields a different fingerprint; deliberately
  mutated sources can therefore never be confused with canonical ones.
* A cache entry stores the three ``build.sh --image`` outputs
  (``arena-boot.efi``, ``arena-esp.img``, ``graphics-profile.txt``) plus
  a ``meta.json`` recording the fingerprint, artifact hashes, Git
  commit/dirty state, and toolchain. Entries are published atomically
  (write a temp directory, then ``os.rename``) with a ``COMPLETE``
  marker written last; incomplete entries are never served.
* Restoring copies the artifacts into ``build/`` (never hands out the
  cache files themselves) and re-verifies their hashes against
  ``meta.json``. Any doubt — missing marker, hash mismatch, unreadable
  metadata — falls back to a real build. Cache problems must never fail
  a test suite that a real build would pass, and must never fake a pass.

Enable per process with ``ARENA_TEST_BUILD_CACHE=1`` (the new runner
sets this; ``tools/run_tests.sh`` does not, so the reference runner's
behavior is unchanged). Override the location with
``ARENA_TEST_BUILD_CACHE_DIR`` (default ``build/test-cache``).
"""

from __future__ import annotations

import hashlib
import json
import os
import shutil
import subprocess
import sys
import time
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent

# Source trees whose bytes can influence tools/build.sh outputs.
_INPUT_TREES = ("kernel", "userspace", "phase9-work", "vendor", "audit")
# Individual build scripts invoked (directly or transitively) by build.sh.
_INPUT_FILES = (
    "tools/build.sh",
    "tools/build_phase14_pie.sh",
    "tools/build_phase14_bundle.py",
    "tools/apb1_format.py",
    "tools/espimg.py",
    "tools/audit_kernel_isa.py",
    "tools/dev-env/env.sh",
)
_SKIP_DIRS = {"target", "__pycache__", ".git"}
_ARTIFACTS = ("arena-boot.efi", "arena-esp.img", "graphics-profile.txt")
_ENV_KEYS = (
    "ARENA_BUILD_PROFILE",
    "ARENA_GRAPHICS_FIXTURE",
    "ARENA_PHASE15_HISTORICAL_DIAGNOSTIC",
    "ARENA_PHASE15_OSXSAVE_SEED",
    "CARGO_TARGET_DIR",
    "RUSTFLAGS",
    "CARGO_ENCODED_RUSTFLAGS",
)
SCHEMA = 1


def cache_dir() -> Path:
    env = os.environ.get("ARENA_TEST_BUILD_CACHE_DIR")
    if env:
        return Path(env)
    return REPO_ROOT / "build" / "test-cache"


def enabled() -> bool:
    return os.environ.get("ARENA_TEST_BUILD_CACHE") == "1"


def _sha256_file(p: Path) -> str:
    h = hashlib.sha256()
    with open(p, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def _tree_manifest(root: Path) -> list[tuple[str, int, str]]:
    """Sorted (relpath, size, sha256) for all build-input files."""
    entries: list[tuple[str, int, str]] = []
    for tree in _INPUT_TREES:
        base = root / tree
        if not base.is_dir():
            continue
        for dirpath, dirnames, filenames in os.walk(base):
            dirnames[:] = sorted(d for d in dirnames if d not in _SKIP_DIRS)
            for name in sorted(filenames):
                p = Path(dirpath) / name
                if p.suffix in (".pyc",) or p.is_symlink():
                    # Symlinks are fingerprinted by target string below.
                    if p.is_symlink():
                        rel = p.relative_to(root).as_posix()
                        entries.append((rel, -1, hashlib.sha256(
                            os.readlink(p).encode()).hexdigest()))
                    continue
                rel = p.relative_to(root).as_posix()
                entries.append((rel, p.stat().st_size, _sha256_file(p)))
    for rel in _INPUT_FILES:
        p = root / rel
        if p.is_file():
            entries.append((rel, p.stat().st_size, _sha256_file(p)))
        else:
            entries.append((rel, -2, "missing"))
    entries.sort()
    return entries


def _resolve_rustc(env: dict) -> str | None:
    """Resolve rustc the way a build.sh run would see it.

    build.sh sources tools/dev-env/env.sh (puts /opt/rust/prefix/bin
    first when present); tools/build_phase14_pie.sh additionally honors
    ARENA_RUST_BIN. Fingerprinting a different rustc than the build
    actually used would be an identity lie, so mirror that order here.
    """
    candidates: list[Path] = []
    if env.get("ARENA_RUST_BIN"):
        candidates.append(Path(env["ARENA_RUST_BIN"]) / "rustc")
    candidates.append(Path("/opt/rust/prefix/bin/rustc"))
    for c in candidates:
        if c.is_file() and os.access(c, os.X_OK):
            return str(c)
    return shutil.which("rustc", path=env.get("PATH"))


def _toolchain_identity(env: dict) -> dict:
    info: dict = {"python": sys.version.split()[0]}
    rustc = _resolve_rustc(env)
    info["rustc_path"] = rustc
    if rustc is None:
        info["rustc_error"] = "not found"
        return info
    try:
        out = subprocess.run([rustc, "--version", "--verbose"],
                             capture_output=True, text=True, timeout=30)
        info["rustc_vv_sha256"] = hashlib.sha256(
            (out.stdout or out.stderr).encode()).hexdigest()
        sysroot = subprocess.run([rustc, "--print", "sysroot"],
                                 capture_output=True, text=True,
                                 timeout=30).stdout.strip()
        info["sysroot"] = sysroot
        for target in ("x86_64-unknown-none", "x86_64-unknown-uefi"):
            libdir = Path(sysroot) / "lib" / "rustlib" / target / "lib"
            cores = sorted(libdir.glob("libcore-*.rlib"))
            info[f"libcore_{target}"] = (
                _sha256_file(cores[0]) if cores else "missing")
        lld = (Path(sysroot) / "lib" / "rustlib" /
               "x86_64-unknown-linux-gnu" / "bin" / "rust-lld")
        info["rust_lld"] = _sha256_file(lld) if lld.is_file() else "missing"
    except Exception as e:  # identity must fail *distinct*, never shared
        info["rustc_error"] = f"{type(e).__name__}: {e}"
    return info


def fingerprint(env: dict, root: Path = REPO_ROOT) -> tuple[str, dict]:
    """Content fingerprint of everything build.sh's outputs depend on."""
    manifest = _tree_manifest(root)
    tree_hash = hashlib.sha256(
        "".join(f"{rel}\0{size}\0{digest}\n"
                for rel, size, digest in manifest).encode()).hexdigest()
    effective_env = {k: env.get(k, "<unset>") for k in _ENV_KEYS}
    config = {
        "schema": SCHEMA,
        "root": str(root),
        "tree_sha256": tree_hash,
        "tree_files": len(manifest),
        "env": effective_env,
        "toolchain": _toolchain_identity(env),
    }
    fp = hashlib.sha256(json.dumps(config, sort_keys=True).encode()).hexdigest()
    return fp, config


def _git(root: Path, *args: str) -> str:
    try:
        return subprocess.run(["git", "-C", str(root), *args],
                              capture_output=True, text=True, timeout=30).stdout
    except Exception:
        return ""


def entry_path(fp: str) -> Path:
    return cache_dir() / "builds" / fp


def lookup(fp: str) -> Path | None:
    """Return a verified entry directory for fp, or None (fail closed)."""
    d = entry_path(fp)
    try:
        if not (d / "COMPLETE").is_file():
            return None
        meta = json.loads((d / "meta.json").read_text())
        if meta.get("fingerprint") != fp or meta.get("schema") != SCHEMA:
            return None
        for name in _ARTIFACTS:
            p = d / name
            if not p.is_file() or _sha256_file(p) != meta["artifacts"][name]:
                return None
        return d
    except Exception:
        return None


def restore(entry: Path, root: Path = REPO_ROOT) -> bool:
    """Copy the entry's artifacts into build/ atomically; verify hashes."""
    meta = json.loads((entry / "meta.json").read_text())
    bdir = root / "build"
    bdir.mkdir(exist_ok=True)
    for name in _ARTIFACTS:
        tmp = bdir / f".{name}.cache-tmp-{os.getpid()}"
        shutil.copyfile(entry / name, tmp)
        os.replace(tmp, bdir / name)
        if _sha256_file(bdir / name) != meta["artifacts"][name]:
            return False
    return True


def store(fp: str, config: dict, build_log: str, duration_s: float,
          root: Path = REPO_ROOT) -> Path | None:
    """Atomically publish the current build/ artifacts under fp."""
    builds = cache_dir() / "builds"
    builds.mkdir(parents=True, exist_ok=True)
    tmp = builds / f"{fp}.tmp-{os.getpid()}-{time.time_ns()}"
    dest = builds / fp
    try:
        tmp.mkdir()
        artifacts = {}
        for name in _ARTIFACTS:
            src = root / "build" / name
            if not src.is_file():
                shutil.rmtree(tmp, ignore_errors=True)
                return None
            shutil.copyfile(src, tmp / name)
            artifacts[name] = _sha256_file(tmp / name)
        status = _git(root, "status", "--porcelain")
        meta = {
            "schema": SCHEMA,
            "fingerprint": fp,
            "config": config,
            "artifacts": artifacts,
            "git_commit": _git(root, "rev-parse", "HEAD").strip(),
            "git_dirty": bool(status.strip()),
            "git_status_lines": [ln for ln in status.splitlines() if ln],
            "created_utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
            "build_duration_s": round(duration_s, 3),
            "classification": ("cache entry for an identified build; "
                               "not a qualification claim"),
        }
        (tmp / "meta.json").write_text(json.dumps(meta, indent=2, sort_keys=True))
        (tmp / "build.log").write_text(build_log)
        (tmp / "COMPLETE").write_text(fp + "\n")
        try:
            os.rename(tmp, dest)  # atomic; first publisher wins
        except OSError:
            shutil.rmtree(tmp, ignore_errors=True)
            return dest if lookup(fp) else None
        return dest
    except Exception:
        shutil.rmtree(tmp, ignore_errors=True)
        return None


def prune(keep: int = 8) -> list[str]:
    """Delete all but the newest `keep` complete entries. Returns removed."""
    builds = cache_dir() / "builds"
    removed: list[str] = []
    if not builds.is_dir():
        return removed
    entries = []
    for d in builds.iterdir():
        if d.is_dir() and (d / "COMPLETE").is_file():
            entries.append((d.stat().st_mtime, d))
        elif d.is_dir() and ".tmp-" in d.name:
            shutil.rmtree(d, ignore_errors=True)
            removed.append(d.name)
    entries.sort(reverse=True)
    for _, d in entries[keep:]:
        shutil.rmtree(d, ignore_errors=True)
        removed.append(d.name)
    return removed


def main(argv: list[str]) -> int:
    if len(argv) < 2:
        print(__doc__)
        return 2
    cmd = argv[1]
    if cmd == "fingerprint":
        t0 = time.monotonic()
        fp, config = fingerprint(dict(os.environ))
        print(json.dumps({"fingerprint": fp,
                          "computed_in_s": round(time.monotonic() - t0, 3),
                          "config": config}, indent=2, sort_keys=True))
        return 0
    if cmd == "prune":
        keep = int(argv[2]) if len(argv) > 2 else 8
        print(json.dumps({"removed": prune(keep)}))
        return 0
    if cmd == "list":
        builds = cache_dir() / "builds"
        out = []
        if builds.is_dir():
            for d in sorted(builds.iterdir()):
                meta_p = d / "meta.json"
                if meta_p.is_file():
                    m = json.loads(meta_p.read_text())
                    out.append({"fingerprint": d.name,
                                "efi": m["artifacts"]["arena-boot.efi"],
                                "dirty": m["git_dirty"],
                                "created": m["created_utc"]})
        print(json.dumps(out, indent=2))
        return 0
    print(f"unknown command: {cmd}", file=sys.stderr)
    return 2


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
