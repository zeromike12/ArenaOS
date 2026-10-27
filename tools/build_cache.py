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
commit or timestamps.

The schema-2 fingerprint covers, in addition to the source trees and
build scripts themselves:

* every Cargo configuration file Cargo would read for the build's
  crate directories — crate-local, repository-root, ancestor, and
  ``$CARGO_HOME`` configs — by parsed (semantic) content;
* effective Cargo/compiler environment overrides (profile, target,
  rustflags, feature-unification, source replacement, ...), recorded
  only as SHA-256 digests, never raw values, and only for a selected
  build-relevant allow-list — the whole environment is never dumped
  into a receipt and secret-looking variables are never fingerprinted;
* environment variables the sources themselves declare
  (``option_env!``/``env!``/``rerun-if-env-changed``);
* path dependencies, build scripts, source replacements, custom
  target specifications, and linker scripts that point outside the
  hashed trees (hashed where they live);
* symlink *target contents*, including symlinked directories;
* the tools actually selected: rustc/cargo/rust-lld/disassembler
  binaries, sysroot libraries for every configured target, PATH tools,
  and the Python modules the image scripts import (notably pyfatfs).

Where a supported configuration cannot be identified reliably — an
alternate rustc or rustc wrapper, an explicit non-sysroot linker, a
malformed config, a broken symlink, a missing toolchain component —
reuse is *disabled with an explicit reason* (``fingerprint_status``
returns ``None`` plus blockers) rather than sharing an uncertain key.
A real build always remains available; the cache can only decline.

Cache entries are published atomically with a ``COMPLETE`` marker and
served only after strict metadata/artifact validation. Any doubt —
missing marker, malformed metadata, hash mismatch — is a miss.

Enable per process with ``ARENA_TEST_BUILD_CACHE=1`` (the evidence
runner sets this; ``tools/run_tests.sh`` does not). Override the
location with ``ARENA_TEST_BUILD_CACHE_DIR`` (default
``build/test-cache``).
"""

from __future__ import annotations

import hashlib
import inspect
import json
import os
import re
import shutil
import stat
import subprocess
import sys
import time
from pathlib import Path

try:  # Python 3.11+
    import tomllib
except ModuleNotFoundError:  # pragma: no cover - supported hosts are 3.11+
    tomllib = None  # type: ignore[assignment]

REPO_ROOT = Path(__file__).resolve().parent.parent

# Source trees whose bytes can influence tools/build.sh outputs.
_INPUT_TREES = ("kernel", "userspace", "phase9-work", "vendor", "audit")
# Individual build inputs invoked (directly or transitively) by build.sh,
# plus repository-root configuration that Cargo/rustup would honor.
_INPUT_FILES = (
    "tools/build.sh",
    "tools/build_phase14_pie.sh",
    "tools/build_phase14_bundle.py",
    "tools/apb1_format.py",
    "tools/espimg.py",
    "tools/audit_kernel_isa.py",
    "tools/dev-env/env.sh",
    "tools/build_cache.py",
    "tools/mtest.py",
    ".cargo/config.toml",
    ".cargo/config",
    "Cargo.toml",
    "Cargo.lock",
    "rust-toolchain.toml",
    "rust-toolchain",
)
_SKIP_DIRS = {"target", "__pycache__", ".git"}
_ARTIFACTS = ("arena-boot.efi", "arena-esp.img", "graphics-profile.txt")
SCHEMA = 2

_KNOWN_TARGETS = ("x86_64-unknown-none", "x86_64-unknown-uefi")
_MAX_EXTERNAL_FILES = 200_000
_MAX_EXTERNAL_BYTES = 4 << 30

# Environment variables that can change build.sh outputs. Values enter
# the fingerprint only as SHA-256 digests. Prefix matches cover the
# CARGO_*/RUST*/ARENA_* override families (CARGO_PROFILE_*, CARGO_BUILD_*,
# CARGO_TARGET_*, RUSTFLAGS, the ARENA_* build switches) without capturing
# unrelated process state.
_EXPLICIT_ENV_KEYS = (
    "ARENA_BUILD_PROFILE",
    "ARENA_GRAPHICS_FIXTURE",
    "ARENA_PHASE15_HISTORICAL_DIAGNOSTIC",
    "ARENA_PHASE15_OSXSAVE_SEED",
    "ARENA_PERF",
    "ARENA_RUST_BIN",
    "ARENA_LLVM_TOOLS",
    "CARGO",
    "CARGO_HOME",
    "CARGO_TARGET_DIR",
    "CARGO_ENCODED_RUSTFLAGS",
    "RUSTC",
    "RUSTDOC",
    "RUSTFLAGS",
    "RUSTDOCFLAGS",
    "RUSTUP_HOME",
    "RUSTUP_TOOLCHAIN",
    "SOURCE_DATE_EPOCH",
    "ZERO_AR_DATE",
    "TZ",
    "LANG",
    "LC_ALL",
    "HOME",
    "PATH",
)
_ENV_PREFIXES = ("CARGO_", "RUST", "ARENA_")
_PROGRAM_ENV_RE = re.compile(
    r"^(CC|CXX|CPP|AS|AR|LD|RANLIB|OBJCOPY|OBJDUMP|STRIP|"
    r"CFLAGS|CXXFLAGS|CPPFLAGS|LDFLAGS|HOST_CC|HOST_CXX|TARGET_CC|TARGET_CXX)"
    r"(_|$)")
_SECRET_ENV_RE = re.compile(
    r"(TOKEN|PASSWORD|PASSWD|SECRET|CREDENTIAL|API[_-]?KEY|PRIVATE[_-]?KEY)",
    re.IGNORECASE)
# Harness/test/backend state: recorded in run evidence, irrelevant to the
# bytes build.sh produces, so excluded to avoid pointless invalidation.
_EXCLUDED_ENV_KEYS = {
    "ARENA_TEST_BUILD_CACHE", "ARENA_TEST_BUILD_CACHE_DIR",
    "ARENA_TEST_ATTEMPT_DIR", "ARENA_TEST_RUN_ID", "ARENA_TEST_WORKERS_DIR",
    "ARENA_WORKER_CWD", "ARENA_QEMU", "ARENA_QEMU_PATCHED",
    "ARENA_QEMU_UNPATCHED", "ARENA_OVMF_CODE", "ARENA_OVMF_VARS",
    "ARENA_PHASE15_PKG", "ARENA_PHASE15_ESP", "ARENA_PHASE15_EFI",
    "ARENA_GLIBC_DIR", "ARENA_DNS_WRONG_TXID", "ARENA_ZIG",
    "RUST_LOG", "RUST_BACKTRACE", "RUST_LIB_BACKTRACE",
    "CARGO_TERM_COLOR", "CARGO_TERM_QUIET", "CARGO_TERM_VERBOSE",
    "CARGO_TERM_PROGRESS_WHEN",
}
_CARGO_PROVIDED_ENV = {
    "CARGO", "CARGO_MANIFEST_DIR", "CARGO_MANIFEST_PATH", "CARGO_PKG_VERSION",
    "CARGO_PKG_NAME", "CARGO_PKG_DESCRIPTION", "CARGO_PKG_HOMEPAGE",
    "CARGO_PKG_REPOSITORY", "CARGO_PKG_AUTHORS", "CARGO_PKG_RUST_VERSION",
    "PROFILE", "TARGET", "HOST", "OUT_DIR", "NUM_JOBS", "DEBUG",
    "OPT_LEVEL", "RUSTDOC", "CARGO_CRATE_NAME",
}
_DEP_TABLE_KEYS = {"dependencies", "build-dependencies", "dev-dependencies"}


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


def _hash_text(text: str) -> str:
    return hashlib.sha256(text.encode()).hexdigest()


def _canonical_value(v):
    """JSON-canonical form of a parsed TOML value (dates tagged)."""
    if isinstance(v, dict):
        return {str(k): _canonical_value(v[k]) for k in sorted(v)}
    if isinstance(v, (list, tuple)):
        return [_canonical_value(x) for x in v]
    if isinstance(v, (str, int, float, bool)) or v is None:
        return v
    return {"type": type(v).__name__, "value": str(v)}


def _semantic_digest(value) -> str:
    return _hash_text(json.dumps(_canonical_value(value), sort_keys=True))


# ------------------------------------------------------------- tree inputs
#
# A manifest entry is (logical_relpath, size, sha256, detail). Symlinks
# are fingerprinted by their target *contents* (plus the raw target
# string, which can itself be embedded), symlinked directories are
# traversed, and anything unreadable/broken/cyclic becomes an explicit
# reuse blocker instead of a silently shared key.

def _walk_directory(base: Path, logical: Path, root: Path,
                    entries: list, issues: list[str],
                    ancestors: tuple[str, ...], state: dict) -> None:
    try:
        children = sorted(os.scandir(base), key=lambda c: c.name)
    except OSError as e:
        issues.append(f"unreadable directory {logical}: {e}")
        return
    for child in children:
        rel = logical / child.name
        rel_s = rel.as_posix()
        try:
            is_link = child.is_symlink()
            followed_dir = child.is_dir(follow_symlinks=True)
        except OSError as e:
            issues.append(f"unreadable path {rel_s}: {e}")
            continue
        if child.name in _SKIP_DIRS and followed_dir:
            continue
        if state.get("files", 0) > _MAX_EXTERNAL_FILES or \
                state.get("bytes", 0) > _MAX_EXTERNAL_BYTES:
            if not state.get("limit_reported"):
                issues.append("input tree exceeds fingerprint size limits")
                state["limit_reported"] = True
            return
        if is_link:
            raw = os.readlink(child.path)
            try:
                st = child.stat(follow_symlinks=True)
            except OSError:
                entries.append((rel_s, -3, _hash_text(f"broken\0{raw}"),
                                "symlink-broken"))
                issues.append(f"broken symlink in build inputs: {rel_s} -> {raw}")
                continue
            if stat.S_ISDIR(st.st_mode):
                real = os.path.realpath(child.path)
                if real in ancestors or Path(real) == root or \
                        root in Path(real).parents:
                    entries.append((rel_s, -4, _hash_text(f"cycle\0{raw}"),
                                    "symlink-dir-cycle"))
                    issues.append(f"cyclic/ancestor symlinked directory: {rel_s}")
                    continue
                entries.append((rel_s, -4,
                                _hash_text(f"{raw}\0{real}"), "symlink-dir"))
                _walk_directory(Path(child.path), rel, root, entries, issues,
                                ancestors + (real,), state)
            elif stat.S_ISREG(st.st_mode):
                try:
                    digest = _sha256_file(Path(child.path))
                except OSError as e:
                    issues.append(f"unreadable symlink target {rel_s}: {e}")
                    continue
                state["files"] = state.get("files", 0) + 1
                state["bytes"] = state.get("bytes", 0) + st.st_size
                entries.append((rel_s, st.st_size, digest,
                                f"symlink-file:{_hash_text(raw)[:16]}:"
                                f"{stat.S_IMODE(st.st_mode):o}"))
            else:
                issues.append(f"special file at symlinked path: {rel_s}")
            continue
        if child.is_dir(follow_symlinks=False):
            real = os.path.realpath(child.path)
            _walk_directory(Path(child.path), rel, root, entries, issues,
                            ancestors + (real,), state)
        elif child.is_file(follow_symlinks=False):
            p = Path(child.path)
            try:
                st = child.stat(follow_symlinks=False)
                digest = _sha256_file(p)
            except OSError as e:
                issues.append(f"unreadable build input {rel_s}: {e}")
                continue
            state["files"] = state.get("files", 0) + 1
            state["bytes"] = state.get("bytes", 0) + st.st_size
            entries.append((rel_s, st.st_size, digest,
                            f"file:{stat.S_IMODE(st.st_mode):o}"))
        else:
            issues.append(f"special file in build inputs: {rel_s}")


def _add_input_file(root: Path, rel: str, entries: list,
                    issues: list[str]) -> None:
    p = root / rel
    if p.is_symlink() or p.is_file():
        try:
            st = p.stat()
            detail = (f"symlink-file:{_hash_text(os.readlink(p))[:16]}:"
                      f"{stat.S_IMODE(st.st_mode):o}" if p.is_symlink()
                      else f"file:{stat.S_IMODE(st.st_mode):o}")
            entries.append((rel, st.st_size, _sha256_file(p), detail))
        except OSError as e:
            entries.append((rel, -3, "unreadable", "error"))
            issues.append(f"unreadable build input {rel}: {e}")
    elif p.exists():
        entries.append((rel, -5, "not-a-file", "error"))
        issues.append(f"build input {rel} exists but is not a file")
    else:
        entries.append((rel, -2, "missing", "missing"))


def _tree_manifest_with_issues(root: Path) -> tuple[list, list[str]]:
    entries: list = []
    issues: list[str] = []
    for tree in _INPUT_TREES:
        base = root / tree
        if base.is_symlink():
            entries.append((tree, -4, _hash_text(os.readlink(base)),
                            "symlink-dir"))
        if not base.is_dir():
            if base.exists() or base.is_symlink():
                issues.append(f"input tree {tree} is not a directory")
            continue
        _walk_directory(base, Path(tree), root, entries, issues,
                        (os.path.realpath(base),), {})
    for rel in _INPUT_FILES:
        _add_input_file(root, rel, entries, issues)
    entries.sort()
    return entries, issues


def _tree_manifest(root: Path) -> list:
    """Sorted manifest entries for all build-input files (compat)."""
    return _tree_manifest_with_issues(root)[0]


def _is_covered(path: Path, root: Path) -> bool:
    try:
        rel = path.resolve().relative_to(root)
    except (ValueError, OSError):
        return False
    parts = rel.parts
    if not parts:
        return False
    return parts[0] in _INPUT_TREES or rel.as_posix() in _INPUT_FILES


def _external_input_identity(path: Path, root: Path,
                             origin: str) -> tuple[dict | None, list[str]]:
    """Hash a build input that lives outside the covered trees."""
    if _is_covered(path, root):
        return None, []
    blockers: list[str] = []
    try:
        rel = path.resolve().relative_to(root).as_posix()
        path_id = f"repo:{rel}"
    except (ValueError, OSError):
        path_id = f"external:{_hash_text(str(path))[:24]}"
    rec: dict = {"path_id": path_id, "origin": origin}
    if path.is_symlink() and not path.exists():
        return rec | {"kind": "missing"}, \
            [f"{origin} points at a broken symlink: {path}"]
    if path.is_file():
        try:
            rec.update(kind="file", size=path.stat().st_size,
                       sha256=_sha256_file(path))
        except OSError as e:
            blockers.append(f"{origin} file unreadable: {path}: {e}")
            rec["kind"] = "unreadable"
        return rec, blockers
    if path.is_dir():
        entries: list = []
        issues: list[str] = []
        _walk_directory(path, Path("."), root, entries, issues,
                        (os.path.realpath(path),), {})
        blockers.extend(f"{origin}: {i}" for i in issues)
        rec.update(kind="tree", files=len(entries),
                   tree_sha256=_hash_text("".join(
                       f"{r}\0{s}\0{d}\0{t}\n" for r, s, d, t in sorted(entries))))
        return rec, blockers
    return rec | {"kind": "missing"}, \
        [f"{origin} path does not exist: {path}"]


# ---------------------------------------------------- declared env / paths

_ENV_MACRO_RE = re.compile(
    rb'(?:option_env|env)!\s*\(\s*"([A-Z][A-Z0-9_]*)"')
_ENV_VAR_RE = re.compile(rb'env::var(?:_os)?\s*\(\s*"([A-Z][A-Z0-9_]*)"')
_RERUN_ENV_RE = re.compile(rb'rerun-if-env-changed=([A-Z][A-Z0-9_]*)')
_RERUN_PATH_RE = re.compile(rb'rerun-if-changed=([^\s"\'\\]+)')


def _scan_sources(root: Path, entries: list) -> tuple[list[str], list[Path]]:
    """Env vars and extra files the sources themselves declare."""
    names: set[str] = set()
    extra_paths: list[Path] = []
    for rel, _size, _digest, _detail in entries:
        if not rel.endswith(".rs"):
            continue
        try:
            data = (root / rel).read_bytes()
        except OSError:
            continue
        for rx in (_ENV_MACRO_RE, _ENV_VAR_RE, _RERUN_ENV_RE):
            for m in rx.finditer(data):
                name = m.group(1).decode()
                if name not in _CARGO_PROVIDED_ENV and \
                        not name.startswith("CARGO_PKG_") and \
                        not name.startswith("CARGO_FEATURE_") and \
                        not name.startswith("DEP_"):
                    names.add(name)
        if rel.endswith("/build.rs") or rel == "build.rs":
            pkg = (root / rel).parent
            for m in _RERUN_PATH_RE.finditer(data):
                raw = m.group(1).decode(errors="replace").strip(",);")
                if raw and "{" not in raw and "}" not in raw:
                    extra_paths.append(pkg / raw)
    return sorted(names), extra_paths


def _manifest_external_paths(root: Path, entries: list,
                             ) -> tuple[list[Path], list[str]]:
    """Path deps / patches / workspace members / explicit targets."""
    paths: list[Path] = []
    blockers: list[str] = []
    if tomllib is None:
        return paths, ["tomllib unavailable: cannot parse Cargo manifests"]

    def dep_specs(obj) -> None:
        if isinstance(obj, dict):
            for k, v in obj.items():
                if k in _DEP_TABLE_KEYS and isinstance(v, dict):
                    for spec in v.values():
                        if isinstance(spec, dict) and \
                                isinstance(spec.get("path"), str):
                            found.append(spec["path"])
                elif k == "patch" and isinstance(v, dict):
                    for src in v.values():
                        if isinstance(src, dict):
                            for spec in src.values():
                                if isinstance(spec, dict) and \
                                        isinstance(spec.get("path"), str):
                                    found.append(spec["path"])
                else:
                    dep_specs(v)
        elif isinstance(obj, list):
            for v in obj:
                dep_specs(v)

    for rel, _s, _d, _t in entries:
        if Path(rel).name != "Cargo.toml":
            continue
        manifest = root / rel
        found: list[str] = []
        try:
            data = tomllib.loads(manifest.read_text())
        except Exception as e:
            blockers.append(f"malformed Cargo manifest {rel}: {e}")
            continue
        dep_specs(data)
        pkg = data.get("package") or {}
        if isinstance(pkg.get("build"), str):
            found.append(pkg["build"])
        ws = data.get("workspace") or {}
        for member in ws.get("members") or []:
            if isinstance(member, str) and "*" not in member:
                found.append(member)
        for section in ("lib", "bin", "test", "bench", "example"):
            sec = data.get(section)
            items = sec if isinstance(sec, list) else [sec]
            for item in items:
                if isinstance(item, dict) and isinstance(item.get("path"), str):
                    found.append(item["path"])
        for raw in found:
            p = Path(raw).expanduser()
            paths.append(p if p.is_absolute() else manifest.parent / p)
    return paths, blockers


# ------------------------------------------------------------ cargo config

def _cargo_home(env: dict) -> tuple[Path | None, list[str]]:
    raw = env.get("CARGO_HOME")
    if raw:
        p = Path(raw).expanduser()
        if not p.is_absolute():
            return None, ["CARGO_HOME is relative: Cargo configuration "
                          "depends on each invocation directory"]
        return p, []
    home = env.get("HOME")
    if home and Path(home).is_absolute():
        return Path(home) / ".cargo", []
    return None, ["cannot determine CARGO_HOME (no CARGO_HOME/HOME)"]


def _config_candidates(invocation_dirs: list[Path],
                       cargo_home: Path | None) -> dict[Path, set[str]]:
    out: dict[Path, set[str]] = {}
    for d in invocation_dirs:
        for anc in (d, *d.parents):
            for name in ("config.toml", "config"):
                out.setdefault(anc / ".cargo" / name, set()).add("ancestor")
    if cargo_home is not None:
        for name in ("config.toml", "config"):
            out.setdefault(cargo_home / name, set()).add("cargo-home")
    return out


def _config_base(path: Path, scopes: set[str],
                 cargo_home: Path | None) -> Path:
    if "cargo-home" in scopes and cargo_home is not None and \
            path.parent == cargo_home:
        return cargo_home
    if path.parent.name == ".cargo":
        return path.parent.parent
    return path.parent


def _resolve_config_path(value: str, base: Path, env: dict) -> Path:
    if value.startswith("~"):
        home = env.get("HOME")
        if home:
            value = home + value[1:]
    p = Path(value)
    return p if p.is_absolute() else base / p


def _scan_rustflags(tokens: list[str], *, origin: str, local: bool,
                    base: Path, candidate_bases: list[Path],
                    external: list[Path], blockers: list[str]) -> None:
    """Hash files rustflags can pull in (linker scripts, -L dirs, externs)."""
    def add_path(raw: str, what: str) -> None:
        p = Path(raw)
        if p.is_absolute():
            external.append(p)
        elif local:
            external.append(base / p)
        elif candidate_bases:
            hits = [b / p for b in candidate_bases if (b / p).exists()]
            if hits:
                external.extend(hits)
            else:
                blockers.append(f"{origin}: relative {what} '{raw}' cannot "
                                "be resolved to an existing build input")
        else:
            blockers.append(f"{origin}: relative {what} '{raw}' in a "
                            "non-local Cargo config is invocation-dependent")

    i = 0
    while i < len(tokens):
        tok = tokens[i]
        nxt = tokens[i + 1] if i + 1 < len(tokens) else None
        val = None
        if tok == "-L" and nxt is not None:
            val, i = nxt, i + 1
        elif re.fullmatch(r"-L(?:native|crate|dependency|framework|all)=.+", tok) or \
                (tok.startswith("-L") and len(tok) > 2):
            val = tok.split("=", 1)[-1] if "=" in tok else tok[2:]
        if val is not None:
            add_path(val, "library search path")
            i += 1
            continue
        if tok == "--extern" and nxt is not None:
            if "=" in nxt:
                add_path(nxt.split("=", 1)[1], "--extern path")
            i += 2
            continue
        if tok.startswith("--extern=") and "=" in tok[len("--extern="):]:
            add_path(tok.split("=", 2)[-1], "--extern path")
        if tok == "link-arg" and nxt is not None:
            tok, i = f"link-arg={nxt}", i + 1
        if tok.startswith("link-arg="):
            arg = tok[len("link-arg="):]
            if arg.startswith("-T") and len(arg) > 2:
                add_path(arg[2:], "linker script")
            elif arg.startswith("--script="):
                add_path(arg[len("--script="):], "linker script")
            elif "," in arg and "-T" in arg.split(","):
                parts = arg.split(",")
                j = parts.index("-T")
                if j + 1 < len(parts):
                    add_path(parts[j + 1], "linker script")
            elif "/" in arg and arg.endswith(
                    (".o", ".a", ".so", ".dylib", ".dll", ".obj", ".lib")):
                add_path(arg, "link object")
        i += 1


def _cargo_configuration(env: dict, root: Path, invocation_dirs: list[Path]):
    """Parsed identity of every Cargo config the build would read."""
    records: list[dict] = []
    blockers: list[str] = []
    targets: set[str] = set()
    external: list[Path] = []
    programs: list[dict] = []  # {kind, value, base} validated later
    cargo_home, home_blockers = _cargo_home(env)
    blockers.extend(home_blockers)
    if tomllib is None:
        return records, blockers + ["tomllib unavailable: cannot parse "
                                    "Cargo configuration"], targets, external, programs
    invocation_set = {d.resolve() for d in invocation_dirs}

    def register_target(value, base: Path, origin: str) -> None:
        vals = value if isinstance(value, list) else [value]
        for v in vals:
            if not isinstance(v, str) or not v:
                blockers.append(f"{origin}: non-string build.target")
            elif v.endswith(".json") or "/" in v:
                external.append(_resolve_config_path(v, base, env))
            else:
                targets.add(v)

    for path, scopes in sorted(
            _config_candidates(invocation_dirs, cargo_home).items(),
            key=lambda kv: str(kv[0])):
        if not path.is_file():
            if path.is_symlink():
                blockers.append(f"broken Cargo config symlink: {path}")
            continue
        try:
            data = tomllib.loads(path.read_text())
        except Exception as e:
            blockers.append(f"malformed Cargo config {path}: {e}")
            continue
        base = _config_base(path, scopes, cargo_home)
        origin = f"cargo config {path.name} at {_hash_text(str(path))[:8]}"
        records.append({
            "path_id": _hash_text(str(path)),
            "name": path.name,
            "scopes": sorted(scopes),
            "semantic_sha256": _semantic_digest(data),
        })
        if "include" in data:
            blockers.append(f"{origin}: config 'include' is not supported "
                            "by the cache identity")
        for raw in data.get("paths") or []:
            if isinstance(raw, str):
                external.append(_resolve_config_path(raw, base, env))
        for src in (data.get("source") or {}).values():
            if isinstance(src, dict):
                for key in ("directory", "local-registry"):
                    if isinstance(src.get(key), str):
                        external.append(
                            _resolve_config_path(src[key], base, env))
        build = data.get("build") or {}
        if not isinstance(build, dict):
            blockers.append(f"{origin}: [build] is not a table")
            build = {}
        if "target" in build:
            register_target(build["target"], base, origin)
        for kind, key in (("rustc", "rustc"), ("wrapper", "rustc-wrapper"),
                          ("wrapper", "rustc-workspace-wrapper")):
            if build.get(key):
                programs.append({"kind": kind, "setting": key,
                                 "value": build[key], "base": base,
                                 "origin": origin})
        local = base.resolve() in invocation_set and \
            path.parent.name == ".cargo"
        if isinstance(build.get("rustflags"), list):
            _scan_rustflags([str(x) for x in build["rustflags"]],
                            origin=origin, local=local, base=base,
                            candidate_bases=[], external=external,
                            blockers=blockers)
        for triple, tcfg in (data.get("target") or {}).items():
            if isinstance(triple, str) and re.fullmatch(
                    r"[A-Za-z0-9_][A-Za-z0-9_-]*", triple) and "-" in triple:
                targets.add(triple)
            if isinstance(tcfg, dict):
                for kind, key in (("linker", "linker"), ("ar", "ar")):
                    if tcfg.get(key):
                        programs.append({"kind": kind, "setting": key,
                                         "value": tcfg[key], "base": base,
                                         "origin": origin})
                if isinstance(tcfg.get("rustflags"), list):
                    _scan_rustflags([str(x) for x in tcfg["rustflags"]],
                                    origin=origin, local=local, base=base,
                                    candidate_bases=[], external=external,
                                    blockers=blockers)
    # Environment-level target/rustflags are hashed as values elsewhere;
    # here we only chase files they can reference.
    env_target = env.get("CARGO_BUILD_TARGET")
    if env_target:
        if env_target.endswith(".json") or "/" in env_target:
            _scan_rustflags([], origin="CARGO_BUILD_TARGET", local=False,
                            base=root, candidate_bases=invocation_dirs,
                            external=external, blockers=blockers)
            p = Path(env_target)
            hits = [b / p for b in invocation_dirs if (b / p).exists()]
            if p.is_absolute():
                external.append(p)
            elif hits:
                external.extend(hits)
            else:
                blockers.append("CARGO_BUILD_TARGET names a target spec "
                                "that cannot be resolved")
        else:
            targets.add(env_target)
    enc = env.get("CARGO_ENCODED_RUSTFLAGS")
    if enc:
        _scan_rustflags(enc.split("\x1f"), origin="CARGO_ENCODED_RUSTFLAGS",
                        local=False, base=root,
                        candidate_bases=invocation_dirs, external=external,
                        blockers=blockers)
    elif env.get("RUSTFLAGS"):
        _scan_rustflags(env["RUSTFLAGS"].split(), origin="RUSTFLAGS",
                        local=False, base=root,
                        candidate_bases=invocation_dirs, external=external,
                        blockers=blockers)
    for key, val in env.items():
        m = re.fullmatch(r"CARGO_TARGET_([A-Z0-9_]+)_RUSTFLAGS", key)
        if m and val:
            _scan_rustflags(val.split(), origin=key, local=False, base=root,
                            candidate_bases=invocation_dirs,
                            external=external, blockers=blockers)
    records.sort(key=lambda r: r["path_id"])
    return records, blockers, targets, external, programs


def _program_path(value, base: Path, env: dict) -> tuple[Path | None, str]:
    if isinstance(value, dict):
        raw = value.get("path")
        if not isinstance(raw, str):
            return None, "program table without a path"
    elif isinstance(value, str):
        raw = value
    else:
        return None, "non-string program setting"
    if not raw or " " in raw.strip():
        return None, f"program setting {raw!r} is not a single path"
    p = Path(raw)
    if p.is_absolute():
        return p, ""
    if "/" in raw:
        return base / p, ""
    found = shutil.which(raw, path=env.get("PATH"))
    return (Path(found) if found else None), \
        ("" if found else f"program {raw!r} not found on PATH")


def _validate_programs(programs: list[dict], env: dict,
                       toolchain: dict) -> list[str]:
    """Alternate compilers/wrappers/linkers: identify or disable reuse."""
    blockers: list[str] = []
    rustc = toolchain.get("rustc_path")
    sysroot = toolchain.get("sysroot")

    def same_as_rustc(p: Path | None) -> bool:
        return bool(p and rustc and os.path.realpath(p) ==
                    os.path.realpath(rustc))

    def in_sysroot(p: Path | None) -> bool:
        try:
            return bool(p and sysroot and
                        Path(os.path.realpath(p)).is_relative_to(
                            os.path.realpath(sysroot)))
        except AttributeError:  # Python < 3.9 fallback, unused here
            return False

    for prog in programs:
        p, err = _program_path(prog["value"], prog["base"], env)
        label = f"{prog['origin']} setting {prog['setting']}"
        if prog["kind"] == "wrapper":
            blockers.append(f"{label}: an explicit rustc wrapper is not "
                            "identifiable by the whole-image cache; reuse "
                            "disabled")
        elif prog["kind"] == "rustc":
            if err or not same_as_rustc(p):
                blockers.append(f"{label}: alternate rustc selection is not "
                                "the identified sysroot rustc; reuse disabled")
        elif prog["kind"] in ("linker", "ar"):
            if err or not in_sysroot(p):
                blockers.append(f"{label}: explicit non-sysroot "
                                f"{prog['kind']} cannot be identified "
                                "transitively; reuse disabled")
    for key in ("RUSTC_WRAPPER", "RUSTC_WORKSPACE_WRAPPER",
                "CARGO_BUILD_RUSTC_WRAPPER",
                "CARGO_BUILD_RUSTC_WORKSPACE_WRAPPER"):
        if env.get(key):
            blockers.append(f"{key} selects a rustc wrapper the cache "
                            "cannot identify transitively; reuse disabled")
    for key in ("RUSTC", "CARGO_BUILD_RUSTC"):
        if env.get(key):
            p, err = _program_path(env[key], REPO_ROOT, env)
            if err or not same_as_rustc(p):
                blockers.append(f"{key} selects an alternate rustc; "
                                "reuse disabled")
    for key, val in env.items():
        if val and re.fullmatch(
                r"CARGO_TARGET_[A-Z0-9_]+_(LINKER|AR|RUNNER)", key):
            p, err = _program_path(val, REPO_ROOT, env)
            if key.endswith("_RUNNER"):
                continue  # hashed as a value; does not build artifacts
            if err or not in_sysroot(p):
                blockers.append(f"{key} selects an explicit tool the cache "
                                "cannot identify transitively; reuse disabled")
    return blockers


# --------------------------------------------------------------- toolchain

def _resolve_rustc(env: dict) -> str | None:
    """Resolve rustc the way a build.sh run would see it."""
    candidates: list[Path] = []
    if env.get("ARENA_RUST_BIN"):
        candidates.append(Path(env["ARENA_RUST_BIN"]) / "rustc")
    candidates.append(Path("/opt/rust/prefix/bin/rustc"))
    for c in candidates:
        if c.is_file() and os.access(c, os.X_OK):
            return str(c)
    return shutil.which("rustc", path=env.get("PATH"))


def _tool_file(p: Path) -> dict | None:
    try:
        st = p.stat()
        return {"path": str(p), "size": st.st_size,
                "sha256": _sha256_file(p)}
    except OSError:
        return None


def _python_module_identity(env: dict, root: Path,
                            python: str) -> tuple[list[dict], list[str]]:
    """Hash the exact module files the image scripts load."""
    code = (
        "import json, sys\n"
        "from pathlib import Path\n"
        "root = Path(sys.argv[1])\n"
        "sys.path.insert(0, str(root / 'tools'))\n"
        "import espimg, build_phase14_bundle, audit_kernel_isa  # noqa\n"
        "files = sorted({str(Path(m.__file__).resolve())\n"
        "                for m in list(sys.modules.values())\n"
        "                if getattr(m, '__file__', None)\n"
        "                and Path(m.__file__).suffix in ('.py', '.so')\n"
        "                and Path(m.__file__).is_file()})\n"
        "print(json.dumps({'executable': sys.executable,\n"
        "                  'version': sys.version, 'files': files}))\n")
    try:
        out = subprocess.run([python, "-c", code, str(root)], env=env,
                             capture_output=True, text=True, timeout=60)
    except Exception as e:
        return [], [f"cannot probe build-script Python modules: {e}"]
    if out.returncode != 0:
        digest = _hash_text(out.stderr or "")[:12]
        return [], [f"build-script Python imports fail under {python} "
                    f"(stderr sha {digest}); image scripts would not run"]
    try:
        info = json.loads(out.stdout.strip().splitlines()[-1])
    except Exception:
        return [], ["cannot parse Python module probe output"]
    records = []
    for name in info.get("files", []):
        rec = _tool_file(Path(name))
        if rec is None:
            return [], [f"Python module vanished during probe: {name}"]
        records.append(rec)
    return records, []


def _toolchain_identity(env: dict, root: Path = REPO_ROOT,
                        targets: set[str] | None = None) -> dict:
    info: dict = {"python": sys.version.split()[0], "blockers": []}
    blockers: list[str] = info["blockers"]
    rustc = _resolve_rustc(env)
    info["rustc_path"] = rustc
    if rustc is None:
        blockers.append("rustc not found (ARENA_RUST_BIN/PATH)")
        return info
    rustc_rec = _tool_file(Path(rustc))
    if rustc_rec:
        info["rustc_file"] = rustc_rec
    cargo = Path(rustc).with_name("cargo")
    if not cargo.is_file():
        found = shutil.which("cargo", path=env.get("PATH"))
        cargo = Path(found) if found else cargo
    if cargo.is_file():
        info["cargo_file"] = _tool_file(cargo)
        out = subprocess.run([str(cargo), "--version"], capture_output=True,
                             text=True, timeout=30)
        info["cargo_version_sha256"] = _hash_text(
            (out.stdout or out.stderr).strip())
    else:
        blockers.append("cargo not found beside rustc or on PATH")
    try:
        out = subprocess.run([rustc, "--version", "--verbose"],
                             capture_output=True, text=True, timeout=30)
        if out.returncode != 0:
            blockers.append("rustc --version --verbose failed")
            return info
        info["rustc_vv_sha256"] = _hash_text(out.stdout)
        host = None
        for line in out.stdout.splitlines():
            if line.startswith("host:"):
                host = line.split(":", 1)[1].strip()
        sysroot = subprocess.run([rustc, "--print", "sysroot"],
                                 capture_output=True, text=True,
                                 timeout=30).stdout.strip()
        if not sysroot or not Path(sysroot).is_dir():
            blockers.append("rustc sysroot unavailable")
            return info
        info["sysroot"] = sysroot
        info["host"] = host
        sroot = Path(sysroot)
        for pat in ("librustc_driver-*.so", "libLLVM*.so"):
            libs = sorted((sroot / "lib").glob(pat))
            if libs:
                info[pat] = [_tool_file(p) for p in libs]
        host_bin = sroot / "lib" / "rustlib" / (host or "") / "bin"
        lld = host_bin / "rust-lld"
        info["rust_lld"] = _tool_file(lld) if lld.is_file() else None
        if info["rust_lld"] is None:
            blockers.append("rust-lld missing from sysroot")
        disasm = host_bin / "llvm-objdump"
        if not disasm.is_file():
            found = shutil.which("objdump", path=env.get("PATH"))
            disasm = Path(found) if found else disasm
        info["disassembler"] = _tool_file(disasm) if disasm.is_file() else None
        if info["disassembler"] is None:
            blockers.append("no disassembler (llvm-objdump/objdump) found")
        all_targets = set(_KNOWN_TARGETS) | set(targets or ())
        if host:
            all_targets.add(host)
        lib_pats = ("libcore-*.rlib", "liballoc-*.rlib",
                    "libcompiler_builtins-*.rlib", "libpanic_abort-*.rlib",
                    "libpanic_unwind-*.rlib", "librustc_std_workspace_*.rlib",
                    "libstd-*.rlib", "libstd-*.so", "libproc_macro-*.so",
                    "liblibc-*.rlib", "libunwind-*.rlib", "libcfg_if-*.rlib",
                    "libhashbrown-*.rlib")
        libs_by_target: dict = {}
        for target in sorted(all_targets):
            libdir = sroot / "lib" / "rustlib" / target / "lib"
            files: dict[str, Path] = {}
            for pat in lib_pats:
                for p in libdir.glob(pat):
                    files[p.name] = p
            for p in libdir.rglob("*.o"):
                files[str(p.relative_to(libdir))] = p
            if not any(n.startswith("libcore") for n in files):
                blockers.append(f"sysroot has no libcore for {target}")
                continue
            libs_by_target[target] = [
                {"name": n, "size": files[n].stat().st_size,
                 "sha256": _sha256_file(files[n])} for n in sorted(files)]
        info["target_libs"] = libs_by_target
    except Exception as e:
        blockers.append(f"toolchain probe failed: {type(e).__name__}: {e}")
        return info
    path_tools: dict = {}
    for name in ("bash", "python3", "sha256sum", "cc", "gcc", "ld", "ar",
                 "ranlib", "objdump"):
        found = shutil.which(name, path=env.get("PATH"))
        if found:
            path_tools[name] = _tool_file(Path(found))
    info["path_tools"] = path_tools
    python = shutil.which("python3", path=env.get("PATH")) or sys.executable
    modules, mod_blockers = _python_module_identity(env, root, python)
    blockers.extend(mod_blockers)
    info["python3"] = _tool_file(Path(python))
    info["python_modules"] = modules
    return info


def _call_toolchain_identity(env: dict, root: Path, targets: set) -> dict:
    """Call _toolchain_identity tolerating test doubles' signatures."""
    fn = _toolchain_identity
    try:
        arity = len(inspect.signature(fn).parameters)
    except (TypeError, ValueError):
        arity = 3
    if arity >= 3:
        return fn(env, root, targets)
    if arity == 2:
        return fn(env, root)  # type: ignore[arg-type]
    return fn(env)  # type: ignore[arg-type]


# --------------------------------------------------------------- fingerprint

def _env_identity(env: dict, declared: list[str]) -> list[dict]:
    names: set[str] = set(_EXPLICIT_ENV_KEYS) | set(declared)
    for key in env:
        if key.startswith(_ENV_PREFIXES) or _PROGRAM_ENV_RE.match(key):
            names.add(key)
    records = []
    for name in sorted(names):
        if name in _EXCLUDED_ENV_KEYS or _SECRET_ENV_RE.search(name):
            continue
        present = name in env
        value = env.get(name, "")
        records.append({
            "name": name,
            "present": present,
            "sha256": _hash_text(value) if present else None,
            "bytes": len(value.encode()) if present else 0,
        })
    return records


def fingerprint_status(env: dict, root: Path = REPO_ROOT
                       ) -> tuple[str | None, dict]:
    """Content fingerprint, or (None, config) with reuse blockers.

    The config records *digests only* for environment values and parsed
    Cargo configuration, so it is safe to store in cache metadata and
    run evidence.
    """
    root = Path(root).resolve()
    blockers: list[str] = []
    entries, issues = _tree_manifest_with_issues(root)
    blockers.extend(issues)
    tree_hash = _hash_text("".join(
        f"{rel}\0{size}\0{digest}\0{detail}\n"
        for rel, size, digest, detail in entries))
    declared, rerun_paths = _scan_sources(root, entries)
    manifest_paths, manifest_blockers = _manifest_external_paths(root, entries)
    blockers.extend(manifest_blockers)
    invocation_dirs = {root}
    for tree in _INPUT_TREES:
        invocation_dirs.add(root / tree)
    for rel, _s, _d, _t in entries:
        if Path(rel).name == "Cargo.toml":
            invocation_dirs.add((root / rel).parent)
    invocation_sorted = sorted(invocation_dirs, key=str)
    cargo_records, cargo_blockers, targets, cargo_external, programs = \
        _cargo_configuration(env, root, invocation_sorted)
    blockers.extend(cargo_blockers)
    toolchain = _call_toolchain_identity(env, root, targets)
    blockers.extend(toolchain.get("blockers") or [])
    blockers.extend(_validate_programs(programs, env, toolchain))
    external_records: list[dict] = []
    seen: set[str] = set()
    for origin, paths in (("cargo-config", cargo_external),
                          ("cargo-manifest", manifest_paths),
                          ("build-script", rerun_paths)):
        for p in paths:
            key = str(p)
            if key in seen:
                continue
            seen.add(key)
            rec, ext_blockers = _external_input_identity(Path(p), root, origin)
            blockers.extend(ext_blockers)
            if rec is not None:
                external_records.append(rec)
    external_records.sort(key=lambda r: (r["path_id"], r["origin"]))
    blockers = sorted(set(blockers))
    config = {
        "schema": SCHEMA,
        "root": str(root),
        "tree_sha256": tree_hash,
        "tree_files": len(entries),
        "declared_env": declared,
        "env": _env_identity(env, declared),
        "cargo_configs": cargo_records,
        "external_inputs": external_records,
        "toolchain": toolchain,
        "reuse": {"enabled": not blockers, "blockers": blockers},
    }
    if blockers:
        return None, config
    fp = _hash_text(json.dumps(config, sort_keys=True))
    return fp, config


def fingerprint(env: dict, root: Path = REPO_ROOT) -> tuple[str, dict]:
    """Content fingerprint of everything build.sh's outputs depend on.

    Raises ValueError with the explicit blockers when reuse is disabled;
    callers that want the reasons without an exception should use
    :func:`fingerprint_status`.
    """
    fp, config = fingerprint_status(env, root)
    if fp is None:
        raise ValueError("build-cache reuse disabled: "
                         + "; ".join(config["reuse"]["blockers"]))
    return fp, config


def _git(root: Path, *args: str) -> str:
    try:
        return subprocess.run(["git", "-C", str(root), *args],
                              capture_output=True, text=True, timeout=30).stdout
    except Exception:
        return ""


def entry_path(fp: str) -> Path:
    return cache_dir() / "builds" / fp


def _is_sha256(value) -> bool:
    return isinstance(value, str) and \
        re.fullmatch(r"[0-9a-f]{64}", value) is not None


def lookup(fp: str) -> Path | None:
    """Return a strictly verified entry directory for fp, or None."""
    if not _is_sha256(fp):
        return None
    d = entry_path(fp)
    try:
        if d.is_symlink() or not d.is_dir():
            return None
        complete = d / "COMPLETE"
        if not complete.is_file() or complete.read_text().strip() != fp:
            return None
        meta = json.loads((d / "meta.json").read_text())
        if not isinstance(meta, dict):
            return None
        if meta.get("fingerprint") != fp or meta.get("schema") != SCHEMA:
            return None
        config = meta.get("config")
        if not isinstance(config, dict) or config.get("schema") != SCHEMA \
                or not (config.get("reuse") or {}).get("enabled"):
            return None
        artifacts = meta.get("artifacts")
        if not isinstance(artifacts, dict) or \
                set(artifacts) != set(_ARTIFACTS):
            return None
        for name in _ARTIFACTS:
            if not _is_sha256(artifacts[name]):
                return None
            p = d / name
            if p.is_symlink() or not p.is_file() or \
                    _sha256_file(p) != artifacts[name]:
                return None
        return d
    except Exception:
        return None


def restore(entry: Path, root: Path = REPO_ROOT) -> bool:
    """Copy the entry's artifacts into build/ atomically; verify hashes."""
    try:
        if entry == entry_path(entry.name) and lookup(entry.name) is None:
            return False
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
    except Exception:
        return False


def store(fp: str, config: dict, build_log: str, duration_s: float,
          root: Path = REPO_ROOT) -> Path | None:
    """Atomically publish the current build/ artifacts under fp."""
    if not _is_sha256(fp) or not isinstance(config, dict) or \
            not (config.get("reuse") or {}).get("enabled", True):
        return None
    builds = cache_dir() / "builds"
    builds.mkdir(parents=True, exist_ok=True)
    tmp = builds / f"{fp}.tmp-{os.getpid()}-{time.time_ns()}"
    dest = builds / fp
    try:
        tmp.mkdir()
        artifacts = {}
        for name in _ARTIFACTS:
            src = root / "build" / name
            if src.is_symlink() or not src.is_file():
                shutil.rmtree(tmp, ignore_errors=True)
                return None
            shutil.copyfile(src, tmp / name)
            artifacts[name] = _sha256_file(tmp / name)
            if artifacts[name] != _sha256_file(src):
                shutil.rmtree(tmp, ignore_errors=True)
                return None
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
        fp, config = fingerprint_status(dict(os.environ))
        print(json.dumps({"fingerprint": fp,
                          "reuse_enabled": fp is not None,
                          "blockers": config.get("reuse", {}).get("blockers"),
                          "computed_in_s": round(time.monotonic() - t0, 3),
                          "config": config}, indent=2, sort_keys=True))
        return 0 if fp is not None else 3
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
                    try:
                        m = json.loads(meta_p.read_text())
                        out.append({"fingerprint": d.name,
                                    "efi": m["artifacts"]["arena-boot.efi"],
                                    "dirty": m["git_dirty"],
                                    "created": m["created_utc"]})
                    except Exception:
                        out.append({"fingerprint": d.name,
                                    "error": "unreadable metadata"})
        print(json.dumps(out, indent=2))
        return 0
    print(f"unknown command: {cmd}", file=sys.stderr)
    return 2


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
