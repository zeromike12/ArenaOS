#!/usr/bin/env python3
"""ArenaOS test runner with preserved evidence (testing workstream).

One entry point, four modes, explicit evidence levels:

  quick     host/static checks + one guest smoke boot. Minutes.
            NEVER a qualification claim.
  affected  suites selected from changed files via tools/test_map.json
            (+ the quick set). Unknown paths fall back to the full
            registered set — never to "run nothing".
  full      the complete 121-suite registered regression (the same set
            tools/run_tests.sh runs) + Phase 16 targeted guest/host
            suites + testing-infrastructure self-tests.
  qualify   qualification gates: fail-closed preconditions (clean tree,
            canonical build path, pinned Phase 15 patched-TCG backend
            identity, official sysroot components), then full, strict
            Phase 15 checks on the final artifacts and the preserved
            package, 100/100 final-artifact stability, and fresh-archive
            verification when a package is supplied.

Every run writes an immutable evidence directory under
``build/test-runs/<run-id>/`` (``build/`` is git-ignored): run metadata
(source commit + dirty diff, toolchain/backend/firmware identities,
exact per-suite commands), streamed per-attempt output, per-boot raw
serials/QEMU stderr (via the ARENA_TEST_ATTEMPT_DIR harness hook in
tools/mtest.py), snapshots of harness outputs in ``build/``, and a final
verdict summary. Failed attempts are preserved beside any retry; a
retry never converts a run to silent green (this runner has no retries:
``suite --repeat N`` records *all* attempts and fails if any failed).

``tools/run_tests.sh`` remains the reference runner and is untouched.
Verdict equivalence with it is demonstrated in docs/testing/.

Optimizations (all honest, all removable):
* ``--build-cache`` (default on): identical-input ``build.sh`` results
  are reused via tools/build_cache.py (content fingerprint, never a
  Git commit alone). ``--no-build-cache`` restores historical behavior.
* ``--jobs N`` (default 1): suites run in isolated ``git worktree``
  checkouts, each inside its own user+mount+network namespace bound to
  the canonical ``/workspace/ArenaOS`` path (fixed host fixture ports
  54321/1053 and the absolute-path-sensitive Phase 14 reproduction make
  plain same-checkout parallelism unsafe). Suites marked ``serial`` in
  the registry never enter the pool.

Exit codes: 0 = all run suites passed; 1 = at least one failed;
2 = blocked/usage (preconditions or selection could not be satisfied —
evidence INCOMPLETE, never a pass).
"""

from __future__ import annotations

import argparse
import concurrent.futures
import datetime
import fnmatch
import hashlib
import json
import os
import platform
import re
import secrets
import shutil
import subprocess
import sys
import time
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(REPO_ROOT / "tools"))

PINNED_QEMU_SHA256 = "e0ea2b1f5ae2a5aa65a455eb232e46e1743b5c327c48ca678630c214c69f26ff"
UNPATCHED_QEMU_SHA256 = "443c81fc9482493297ae7a3f37300bef3a82a2873e428f72884cf52d16e27284"
OFFICIAL_LIBCORE = {
    "x86_64-unknown-none": "6b75c5188dca46c917a67ade52dddea81460f96dc4732003cebea2e22a5b1b09",
    "x86_64-unknown-uefi": "a7de36b0262b7d94cdcf1acb3f78288e7f5a57d546568b3ccaec27cd122c5494",
}
PKG_OVMF_CODE_SHA256 = "7e34e3b83224968bb0714c0ebc88f75c5c37b4cc892c902e511ccc7a7c107f94"
PKG_OVMF_VARS_SHA256 = "5d2ac383371b408398accee7ec27c8c09ea5b74a0de0ceea6513388b15be5d1e"
CANONICAL_PATH = "/workspace/ArenaOS"

_SNAPSHOT_GLOBS = (
    "serial*.log", "qemu-stderr*.log", "tcp-*.log", "udp-dns-*.log",
    "vcon-*.txt", "timeout-*.txt", "*-build.log", "*.ppm",
    "stability*.log", "stability*.txt", "*-serial.log", "*-green.log",
    "*-red.log", "serial.log",
)


def _utcnow() -> str:
    return datetime.datetime.now(datetime.timezone.utc).isoformat()


def _sha256_file(p: Path) -> str | None:
    try:
        h = hashlib.sha256()
        with open(p, "rb") as f:
            for chunk in iter(lambda: f.read(1 << 20), b""):
                h.update(chunk)
        return h.hexdigest()
    except OSError:
        return None


def _run_out(cmd: list[str], env: dict | None = None, timeout: int = 30) -> str:
    try:
        p = subprocess.run(cmd, capture_output=True, text=True,
                           timeout=timeout, env=env)
        out = (p.stdout or "") + (p.stderr or "")
        return out.strip().splitlines()[0] if out.strip() else ""
    except Exception as e:  # identity is record-keeping, not a verdict
        return f"<unavailable: {e}>"


def _git(*args: str, root: Path = REPO_ROOT) -> subprocess.CompletedProcess:
    return subprocess.run(["git", "-C", str(root), *args],
                          capture_output=True, text=True)


# ------------------------------------------------------------------ registry


def load_registry() -> dict:
    return json.loads((REPO_ROOT / "tools/test_registry.json").read_text())


def load_map() -> dict:
    return json.loads((REPO_ROOT / "tools/test_map.json").read_text())


def entries_by_id(registry: dict) -> dict[str, dict]:
    return {e["id"]: e for e in registry["entries"]}


def check_registry(registry: dict) -> list[str]:
    """Fail-closed coverage check: every suite file and every
    run_tests.sh block must be accounted for in the registry."""
    problems: list[str] = []
    by_id = entries_by_id(registry)
    seen: set[str] = set()
    for e in registry["entries"]:
        if e["id"] in seen:
            problems.append(f"duplicate entry id: {e['id']}")
        seen.add(e["id"])
        for field in ("title", "kind", "qualification", "parallel"):
            if not e.get(field):
                problems.append(f"{e['id']}: missing field {field}")
    covered_files = set()
    for e in registry["entries"]:
        for src in e.get("sources", []):
            covered_files.add(src)
    for p in sorted((REPO_ROOT / "tools").glob("test_*.py")):
        rel = f"tools/{p.name}"
        if rel not in covered_files:
            problems.append(f"suite file not in registry: {rel}")
    registered = [e for e in registry["entries"]
                  if e["qualification"] == "registered"]
    if len(registered) != registry.get("registered_count"):
        problems.append(
            f"registered entries {len(registered)} != "
            f"registry.registered_count {registry.get('registered_count')}")
    # Every test_m*.py file must be a registered entry reachable by glob
    # exactly as run_tests.sh reaches it.
    for p in sorted((REPO_ROOT / "tools").glob("test_m*.py")):
        rel = f"tools/{p.name}"
        singles = [e for e in registry["entries"]
                   if e.get("sources") == [rel]
                   and e["qualification"] == "registered"]
        if len(singles) != 1:
            problems.append(f"{rel} must have exactly one registered "
                            f"single-source entry (found {len(singles)})")
    return problems


# ------------------------------------------------------------------ environment


def runner_env(extra: dict | None = None) -> dict:
    env = dict(os.environ)
    # Put the resolved Rust toolchain first, like tools/dev-env/env.sh.
    rust_bin = env.get("ARENA_RUST_BIN")
    if not rust_bin and (Path("/opt/rust/prefix/bin/cargo").exists()):
        rust_bin = "/opt/rust/prefix/bin"
    if rust_bin and rust_bin not in env.get("PATH", "").split(":"):
        env["PATH"] = f"{rust_bin}:{env.get('PATH', '')}"
    env["PYTHONUNBUFFERED"] = "1"
    if extra:
        env.update(extra)
    return env


def _qemu_identity(env: dict) -> dict:
    info: dict = {"areNA_QEMU_env": env.get("ARENA_QEMU")}
    try:
        import arena_env

        old = dict(os.environ)
        os.environ.update({k: v for k, v in env.items()
                           if k in ("ARENA_QEMU", "ARENA_OVMF_CODE",
                                    "ARENA_OVMF_VARS")})
        try:
            cmd = arena_env.qemu_cmd()
        finally:
            os.environ.clear()
            os.environ.update(old)
    except SystemExit as e:
        info["resolution_error"] = str(e)
        info["classification"] = "MISSING"
        return info
    info["command_prefix"] = cmd
    binary = None
    for tok in reversed(cmd):
        if "qemu-system-x86_64" in tok and Path(tok).is_file():
            binary = Path(tok)
            break
    # Wrapper scripts (isolated-glibc): fingerprint wrapper AND binary.
    wrapper = Path(cmd[0]) if cmd and Path(cmd[0]).is_file() else None
    if wrapper is not None:
        info["invoked_file"] = str(wrapper)
        info["invoked_sha256"] = _sha256_file(wrapper)
        try:
            head = wrapper.read_bytes()[:4096]
            if head.startswith(b"#!"):
                info["wrapper_text"] = head.decode(errors="replace")
                for tok in re.findall(
                        r"/[^\s\"'\\]+qemu-system-x86_64[^\s\"'\\]*",
                        info["wrapper_text"]):
                    if Path(tok).is_file():
                        binary = Path(tok)
                        break
        except OSError:
            pass
    if binary is not None:
        info["emulator_binary"] = str(binary)
        info["emulator_sha256"] = _sha256_file(binary)
    info["version"] = _run_out(cmd + ["--version"], env=env)
    accel = _run_out(cmd + ["-accel", "help"], env=env)
    info["accel_help_first"] = accel
    sha = info.get("emulator_sha256") or info.get("invoked_sha256")
    if sha == PINNED_QEMU_SHA256:
        info["classification"] = "PINNED-PATCHED-TCG (Phase 15 qualified binary)"
    elif sha == UNPATCHED_QEMU_SHA256:
        info["classification"] = "UNPATCHED phase15-package binary (strict vector-19 NOT expected)"
    else:
        info["classification"] = "OTHER (not the pinned Phase 15 backend)"
    info["accelerator"] = "tcg (explicit via wrapper/-accel)" \
        if "wrapper_text" in info and "-accel tcg" in info["wrapper_text"] \
        else "harness default (see tools/mtest.py command in attempt evidence)"
    return info


def environment_identity(env: dict) -> dict:
    info: dict = {
        "utc": _utcnow(),
        "uname": platform.uname()._asdict(),
        "cpu_count": os.cpu_count(),
        "python": sys.version.split()[0],
        "repo_root": str(REPO_ROOT),
        "canonical_path": str(REPO_ROOT) == CANONICAL_PATH,
    }
    try:
        mem = {}
        for line in Path("/proc/meminfo").read_text().splitlines()[:3]:
            k, v = line.split(":", 1)
            mem[k] = v.strip()
        info["meminfo"] = mem
    except OSError:
        pass
    info["rustc"] = _run_out(["rustc", "--version"], env=env)
    info["cargo"] = _run_out(["cargo", "--version"], env=env)
    sysroot = _run_out(["rustc", "--print", "sysroot"], env=env)
    info["sysroot"] = sysroot
    libcores = {}
    if sysroot and not sysroot.startswith("<"):
        for target, expected in OFFICIAL_LIBCORE.items():
            cores = sorted((Path(sysroot) / "lib/rustlib" / target / "lib")
                           .glob("libcore-*.rlib"))
            sha = _sha256_file(cores[0]) if cores else None
            libcores[target] = {"sha256": sha, "official": sha == expected}
    info["libcore"] = libcores
    info["gcc"] = _run_out(["gcc", "--version"], env=env)
    zig = env.get("ARENA_ZIG", "zig")
    info["zig"] = _run_out([zig, "version"], env=env)
    info["qemu"] = _qemu_identity(env)
    fw = {}
    try:
        import arena_env

        old = dict(os.environ)
        os.environ.update({k: v for k, v in env.items()
                           if k in ("ARENA_OVMF_CODE", "ARENA_OVMF_VARS")})
        try:
            code, vars_t = arena_env.ovmf_code(), arena_env.ovmf_vars_template()
        finally:
            os.environ.clear()
            os.environ.update(old)
        fw = {"OVMF_CODE": str(code), "OVMF_CODE_sha256": _sha256_file(code),
              "OVMF_VARS_template": str(vars_t),
              "OVMF_VARS_template_sha256": _sha256_file(vars_t),
              "matches_phase15_package":
                  _sha256_file(code) == PKG_OVMF_CODE_SHA256
                  and _sha256_file(vars_t) == PKG_OVMF_VARS_SHA256}
    except BaseException as e:  # SystemExit from arena_env included
        fw = {"error": f"{type(e).__name__}: {e}"}
    info["firmware"] = fw
    glibc_dir = env.get("ARENA_GLIBC_DIR")
    info["host_libc"] = _run_out(["ldd", "--version"], env=env)
    if glibc_dir:
        info["isolated_glibc"] = {
            "dir": glibc_dir,
            "libc_sha256": _sha256_file(Path(glibc_dir) / "lib/libc.so.6"),
            "ld_sha256": _sha256_file(
                Path(glibc_dir) / "lib/ld-linux-x86-64.so.2"),
        }
    return info


def source_identity(run_dir: Path) -> dict:
    commit = _git("rev-parse", "HEAD").stdout.strip()
    branch = _git("rev-parse", "--abbrev-ref", "HEAD").stdout.strip()
    status = _git("status", "--porcelain").stdout
    diff = _git("diff", "HEAD").stdout
    untracked = _git("ls-files", "--others", "--exclude-standard").stdout
    (run_dir / "source.diff").write_text(diff)
    (run_dir / "source-status.txt").write_text(status + untracked)
    return {
        "commit": commit,
        "branch": branch,
        "dirty": bool(status.strip()),
        "status_lines": [ln for ln in status.splitlines() if ln],
        "diff_sha256": hashlib.sha256(diff.encode()).hexdigest(),
        "diff_bytes": len(diff.encode()),
        "untracked": [ln for ln in untracked.splitlines() if ln],
    }


# ------------------------------------------------------------------ selection


def select_mode(registry: dict, mode: str) -> list[dict]:
    ents = registry["entries"]
    if mode == "quick":
        return [e for e in ents if e.get("quick")]
    if mode == "full":
        return [e for e in ents
                if e["qualification"] in ("registered", "phase16-targeted",
                                           "phase16-host", "infra")]
    if mode == "qualify":
        return [e for e in ents
                if e["qualification"] in ("registered", "phase16-targeted",
                                           "phase16-host", "infra",
                                           "strict-phase15", "stability",
                                           "package")]
    raise ValueError(mode)


def changed_files(base: str) -> list[str]:
    files: set[str] = set()
    mb = _git("merge-base", base, "HEAD")
    if mb.returncode == 0 and mb.stdout.strip():
        r = _git("diff", "--name-only", f"{mb.stdout.strip()}..HEAD")
        files.update(ln for ln in r.stdout.splitlines() if ln)
    for args in (["diff", "--name-only", "HEAD"],
                 ["diff", "--name-only", "--cached"],
                 ["ls-files", "--others", "--exclude-standard"]):
        r = _git(*args)
        files.update(ln for ln in r.stdout.splitlines() if ln)
    return sorted(files)


def select_affected(registry: dict, pathmap: dict, changed: list[str]
                    ) -> tuple[list[dict], dict]:
    """Map changed paths to suites. Unknown paths => full registered."""
    by_id = entries_by_id(registry)
    groups = pathmap.get("groups", {})
    selected: dict[str, str] = {}
    fallback_paths: list[str] = []
    reasons: dict[str, list[str]] = {}

    def select(entry_id: str, why: str) -> None:
        if entry_id in by_id:
            selected.setdefault(entry_id, why)
            reasons.setdefault(entry_id, []).append(why)

    def select_group(g: str, why: str) -> None:
        for e in registry["entries"]:
            if g == "quick" and e.get("quick"):
                select(e["id"], why)
            elif g == "registered" and e["qualification"] == "registered":
                select(e["id"], why)
            elif g == "registered-guests" and e["qualification"] == "registered" \
                    and e["kind"].startswith("guest"):
                select(e["id"], why)
            elif g == "phase16" and e["qualification"] in (
                    "phase16-targeted", "phase16-host"):
                select(e["id"], why)

    for g in groups.get("always", ["quick"]):
        select_group(g, "always included")

    # Reverse map: test module -> suites importing it (from registry).
    importers: dict[str, list[str]] = {}
    file_entry: dict[str, str] = {}
    for e in registry["entries"]:
        for src in e.get("sources", []):
            file_entry[src] = e["id"]
        for mod in e.get("imports", []):
            importers.setdefault(mod, []).append(e["id"])

    for path in changed:
        matched = False
        for rule in pathmap.get("rules", []):
            if fnmatch.fnmatch(path, rule["glob"]):
                matched = True
                why = f"{path} matches {rule['glob']}: {rule.get('reason', '')}"
                for sel in rule.get("select_groups", []):
                    select_group(sel, why)
                for sel in rule.get("select", []):
                    select(sel, why)
                for prefix in rule.get("select_prefixes", []):
                    for e in registry["entries"]:
                        if e["id"].startswith(prefix):
                            select(e["id"], why)
                if rule.get("fallback"):
                    fallback_paths.append(path)
                break
        if path in file_entry:
            matched = True
            select(file_entry[path], f"{path} changed itself")
            mod = Path(path).stem
            for imp in importers.get(mod, []):
                select(imp, f"imports changed {mod}")
        for e in registry["entries"]:
            for glob in e.get("paths", []):
                if fnmatch.fnmatch(path, glob):
                    matched = True
                    select(e["id"], f"{path} in declared paths of {e['id']}")
                    break
        if not matched:
            fallback_paths.append(path)

    if fallback_paths:
        select_group("registered",
                     "conservative fallback: unclassified change(s) "
                     + ", ".join(sorted(set(fallback_paths))))
    result = [by_id[i] for i in registry_order(registry, selected)]
    report = {
        "changed_files": changed,
        "fallback_paths": sorted(set(fallback_paths)),
        "selected": {k: v for k, v in selected.items()},
        "reasons": {k: reasons.get(k, []) for k in selected},
        "fallback_to_full": bool(fallback_paths),
    }
    return result, report


def registry_order(registry: dict, ids: dict | set) -> list[str]:
    return [e["id"] for e in registry["entries"] if e["id"] in ids]


# ------------------------------------------------------------------ execution


def _snapshot_manifest(base: Path) -> dict[str, str | None]:
    manifest: dict[str, str | None] = {}
    bdir = base / "build"
    if bdir.is_dir():
        for glob in _SNAPSHOT_GLOBS:
            for p in bdir.glob(glob):
                if p.is_file():
                    manifest[f"build/{p.name}"] = _sha256_file(p)
    ebuild = base / "experiments/c-runtime/build"
    if ebuild.is_dir():
        for p in ebuild.rglob("*"):
            if p.is_file() and p.suffix in (".json", ".log"):
                rel = p.relative_to(base).as_posix()
                manifest[rel] = _sha256_file(p)
    return manifest


def _snapshot_copy(base: Path, before: dict, dest: Path) -> list[str]:
    after = _snapshot_manifest(base)
    copied = []
    for rel, sha in sorted(after.items()):
        if before.get(rel) != sha:
            out = dest / rel
            out.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(base / rel, out)
            copied.append(rel)
    return copied


class SuiteResult:
    def __init__(self, entry_id: str):
        self.entry_id = entry_id
        self.verdict = "NOT_RUN"
        self.attempts: list[dict] = []


def run_attempt(entry: dict, attempt_dir: Path, env: dict, work_root: Path,
                echo: bool, worker_cmd_prefix: list[str] | None = None,
                outer_timeout_s: int | None = None) -> dict:
    """Run one suite attempt, preserving all raw evidence in attempt_dir."""
    attempt_dir.mkdir(parents=True, exist_ok=False)
    cmd = list(entry["cmd"])
    cwd = work_root / entry.get("cwd", ".")
    suite_env = dict(env)
    suite_env["ARENA_TEST_ATTEMPT_DIR"] = str(attempt_dir)
    if worker_cmd_prefix:
        suite_env["ARENA_WORKER_CWD"] = str(
            Path(CANONICAL_PATH) / entry.get("cwd", "."))
    # In a worker namespace the attempt dir must be inside the worktree;
    # the caller passes a worker-local attempt dir and copies it out.
    timeout_s = outer_timeout_s or int(entry.get("timeout_s", 900))
    full_cmd = (worker_cmd_prefix or []) + cmd
    (attempt_dir / "cmd.json").write_text(json.dumps({
        "entry": entry["id"], "argv": full_cmd, "cwd": str(cwd),
        "timeout_s": timeout_s,
        "env_overrides": {k: v for k, v in suite_env.items()
                          if k.startswith("ARENA_")},
        "started_utc": _utcnow(),
    }, indent=2))
    before = _snapshot_manifest(work_root)
    status_before = _git("status", "--porcelain", root=work_root).stdout
    t0 = time.monotonic()
    timed_out = False
    exit_code: int | None = None
    out_path = attempt_dir / "output.log"
    with open(out_path, "wb") as logf:
        proc = subprocess.Popen(
            full_cmd, cwd=cwd, env=suite_env,
            stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
            start_new_session=True)
        assert proc.stdout is not None
        deadline = t0 + timeout_s
        try:
            for raw in proc.stdout:
                logf.write(raw)
                logf.flush()
                if echo:
                    sys.stdout.write(raw.decode(errors="replace"))
                    sys.stdout.flush()
                if time.monotonic() > deadline:
                    timed_out = True
                    try:
                        os.killpg(proc.pid, 9)
                    except ProcessLookupError:
                        pass
                    break
            exit_code = proc.wait(timeout=30)
        except subprocess.TimeoutExpired:
            timed_out = True
            try:
                os.killpg(proc.pid, 9)
            except ProcessLookupError:
                pass
            exit_code = proc.wait()
    dt = time.monotonic() - t0
    status_after = _git("status", "--porcelain", root=work_root).stdout
    outputs = _snapshot_copy(work_root, before, attempt_dir / "outputs")
    boots = []
    boots_dir = attempt_dir / "boots"
    if boots_dir.is_dir():
        for b in sorted(boots_dir.iterdir()):
            bj = b / "boot.json"
            if bj.is_file():
                boots.append(json.loads(bj.read_text()))
    builds = []
    builds_dir = attempt_dir / "builds"
    if builds_dir.is_dir():
        for b in sorted(builds_dir.iterdir()):
            bj = b / "build.json"
            if bj.is_file():
                builds.append(json.loads(bj.read_text()))
    verdict = "PASS" if (exit_code == 0 and not timed_out) else "FAIL"
    result = {
        "entry": entry["id"],
        "attempt_dir": str(attempt_dir),
        "exit_code": exit_code,
        "timed_out": timed_out,
        "verdict": verdict,
        "duration_s": round(dt, 3),
        "started_utc": json.loads((attempt_dir / "cmd.json").read_text())["started_utc"],
        "finished_utc": _utcnow(),
        "output_log": "output.log",
        "output_log_sha256": _sha256_file(out_path),
        "snapshot_outputs": outputs,
        "boots": boots,
        "builds": builds,
        "worktree_status_changed": status_before != status_after,
        "negative_controls": entry.get("negative_controls", ""),
    }
    (attempt_dir / "result.json").write_text(json.dumps(result, indent=2))
    return result


def make_run_dir(mode: str, commit: str, dirty: bool) -> tuple[str, Path]:
    base = REPO_ROOT / "build" / "test-runs"
    base.mkdir(parents=True, exist_ok=True)
    for _ in range(100):
        stamp = datetime.datetime.now(datetime.timezone.utc).strftime("%Y%m%dT%H%M%SZ")
        run_id = (f"{stamp}-{mode}-{commit[:7] or 'unknown'}"
                  f"{'-dirty' if dirty else ''}-{os.getpid()}-"
                  f"{secrets.token_hex(3)}")
        path = base / run_id
        try:
            path.mkdir()
            return run_id, path
        except FileExistsError:
            continue
    raise RuntimeError("could not allocate a unique run directory")


def worker_prepare(worker_root: Path, snapshot_commit: str) -> list[str]:
    """Create one isolated worktree; return the namespace command prefix."""
    worker_root.parent.mkdir(parents=True, exist_ok=True)
    r = _git("worktree", "add", "--detach", str(worker_root), snapshot_commit)
    if r.returncode != 0:
        raise RuntimeError(f"git worktree add failed: {r.stderr}")
    script = worker_root.parent / f"{worker_root.name}-enter.sh"
    script.write_text(
        "#!/bin/sh\n"
        "set -e\n"
        f"mount --bind {worker_root} {CANONICAL_PATH}\n"
        "ip link set lo up\n"
        "# cd to the entry cwd under its canonical (post-bind) name so\n"
        "# getcwd()/realpath spell /workspace/ArenaOS: cargo/rustc embed\n"
        "# absolute paths, and the Phase 14 PIE hash guard depends on it.\n"
        "if [ -n \"$ARENA_WORKER_CWD\" ]; then cd \"$ARENA_WORKER_CWD\"; fi\n"
        "export GIT_CONFIG_COUNT=1 GIT_CONFIG_KEY_0=safe.directory "
        "GIT_CONFIG_VALUE_0='*'\n"
        "exec \"$@\"\n")
    script.chmod(0o755)
    Path(CANONICAL_PATH).mkdir(parents=True, exist_ok=True)
    return ["unshare", "-rmn", str(script)]


def verify_run(run_dir: Path) -> list[str]:
    """Fail-closed completeness check of a finished run directory."""
    problems: list[str] = []
    run_json = run_dir / "run.json"
    if not run_json.is_file():
        return [f"missing {run_json}"]
    try:
        data = json.loads(run_json.read_text())
    except Exception as e:
        return [f"run.json unreadable: {e}"]
    for field in ("run_id", "mode", "source", "environment", "results",
                  "verdict"):
        if field not in data:
            problems.append(f"run.json missing field: {field}")
    for sid, res in (data.get("results") or {}).items():
        if res.get("verdict") in ("PASS", "FAIL") and not res.get("attempts"):
            problems.append(f"{sid}: verdict without attempts (hidden retry?)")
        for att in res.get("attempts", []):
            adir = Path(att.get("attempt_dir", ""))
            if not adir.is_dir() and not (run_dir / "attempts" / sid).is_dir():
                problems.append(f"{sid}: attempt dir missing: {adir}")
                continue
            out = adir / "output.log" if adir.is_dir() else None
            if out is not None and att.get("output_log_sha256"):
                if _sha256_file(out) != att["output_log_sha256"]:
                    problems.append(f"{sid}: output.log hash mismatch")
    return problems


# ------------------------------------------------------------------ main run


def qualify_preconditions(env: dict, env_info: dict, src: dict) -> list[str]:
    blocked = []
    if src["dirty"]:
        blocked.append("worktree is dirty (qualification requires clean source)")
    if not env_info["canonical_path"]:
        blocked.append(f"repo root is {REPO_ROOT}, not canonical {CANONICAL_PATH} "
                       "(protected reproduction is absolute-path sensitive)")
    q = env_info["qemu"]
    if q.get("emulator_sha256") != PINNED_QEMU_SHA256:
        blocked.append(f"emulator is not the pinned patched QEMU "
                       f"({q.get('classification')}); strict vector-19 "
                       "evidence requires the exact binary")
    for target, rec in (env_info.get("libcore") or {}).items():
        if not rec.get("official"):
            blocked.append(f"sysroot libcore for {target} is not the "
                           "official Phase 16 component")
    if not (env_info.get("firmware") or {}).get("matches_phase15_package"):
        blocked.append("firmware does not match the Phase 15 package identities")
    return blocked


def execute(entries: list[dict], mode: str, args, extra_report: dict | None) -> int:
    env = runner_env()
    if args.no_build_cache:
        env.pop("ARENA_TEST_BUILD_CACHE", None)
    else:
        env["ARENA_TEST_BUILD_CACHE"] = "1"
    commit = _git("rev-parse", "HEAD").stdout.strip()
    dirty = bool(_git("status", "--porcelain").stdout.strip())
    run_id, run_dir = make_run_dir(mode, commit, dirty)
    env["ARENA_TEST_RUN_ID"] = run_id
    src = source_identity(run_dir)
    env_info = environment_identity(env)
    blocked_reasons: list[str] = []
    if mode == "qualify":
        blocked_reasons = qualify_preconditions(env, env_info, src)
    if mode == "quick" and env_info["qemu"].get("emulator_sha256") != PINNED_QEMU_SHA256:
        # The smoke boot would fail for a backend reason unrelated to the
        # code under test (in-guest m15 needs genuine vector 19). Say so
        # instead of manufacturing either a pass or a misleading fail.
        before = len(entries)
        entries = [e for e in entries if e["id"] != "smoke-m1"]
        if len(entries) != before:
            blocked_reasons.append(
                "guest smoke boot NOT RUN: emulator is not the pinned "
                "patched backend (stock QEMU fails the in-guest m15 "
                "vector-19 suite by design)")
    by_id = entries_by_id(load_registry())
    omitted = [e["id"] for e in load_registry()["entries"]
               if e["id"] not in {x["id"] for x in entries}]
    run_data: dict = {
        "schema": 1,
        "run_id": run_id,
        "mode": mode,
        "evidence_level": {
            "quick": "DEVELOPMENT SMOKE — not qualification",
            "affected": "DEVELOPMENT AFFECTED-SET — not qualification",
            "full": "FULL REGRESSION (registered + Phase 16) — not by itself milestone qualification",
            "qualify": "MILESTONE QUALIFICATION GATES (if verdict PASS and preconditions met)",
            "suite": "FOCUSED SUITE RUN — not qualification",
        }.get(mode, mode),
        "source": src,
        "environment": env_info,
        "selected": [e["id"] for e in entries],
        "omitted": omitted,
        "blocked_reasons": blocked_reasons,
        "selection_report": extra_report,
        "options": {"jobs": args.jobs, "build_cache": not args.no_build_cache,
                    "repeat": getattr(args, "repeat", 1),
                    "fail_fast": args.fail_fast},
        "started_utc": _utcnow(),
        "results": {},
    }
    (run_dir / "run.json").write_text(json.dumps(run_data, indent=2))

    if mode == "qualify" and blocked_reasons and not args.force:
        run_data["verdict"] = "BLOCKED"
        run_data["finished_utc"] = _utcnow()
        (run_dir / "run.json").write_text(json.dumps(run_data, indent=2))
        print(f"RUN {run_id}: BLOCKED")
        for b in blocked_reasons:
            print(f"  BLOCKED: {b}")
        print(f"evidence: {run_dir}")
        return 2

    results: dict[str, SuiteResult] = {e["id"]: SuiteResult(e["id"]) for e in entries}
    serial_entries = [e for e in entries if e.get("parallel") == "serial"]
    pool_entries = [e for e in entries if e.get("parallel") != "serial"]
    jobs = max(1, args.jobs)
    stopped_early = False

    def do_entry(entry: dict, work_root: Path, prefix: list[str] | None,
                 echo: bool) -> None:
        res = results[entry["id"]]
        for i in range(1, max(1, getattr(args, "repeat", 1)) + 1):
            attempt_dir = run_dir / "attempts" / entry["id"] / f"attempt-{i:03d}"
            attempt_dir.parent.mkdir(parents=True, exist_ok=True)
            if prefix is None:
                att = run_attempt(entry, attempt_dir, env, work_root, echo)
            else:
                # Workers write evidence inside their worktree; copy out.
                w_att = (work_root / "build" / "test-attempts" / run_id
                         / entry["id"] / f"attempt-{i:03d}")
                w_att.parent.mkdir(parents=True, exist_ok=True)
                w_env = dict(env)
                # Inside the namespace the worktree is at CANONICAL_PATH.
                w_env["ARENA_TEST_ATTEMPT_DIR"] = str(
                    Path(CANONICAL_PATH) / w_att.relative_to(work_root))
                # Share the content-addressed build cache across workers
                # (each worker's own build/ would start cold). Atomic
                # publish + hash-verified lookup make this safe.
                w_env.setdefault("ARENA_TEST_BUILD_CACHE_DIR",
                                 "/tmp/arena-test-cache")
                att = run_attempt(entry, w_att, w_env, work_root, echo,
                                  worker_cmd_prefix=prefix)
                if attempt_dir.exists():
                    shutil.rmtree(attempt_dir)
                shutil.copytree(w_att, attempt_dir)
                att["attempt_dir"] = str(attempt_dir)
                att["worker"] = str(work_root)
                (attempt_dir / "result.json").write_text(json.dumps(att, indent=2))
                shutil.rmtree(w_att)
                # A previous suite in this worker may have left the tree
                # dirty (failed restoration). Reset for the next suite,
                # but only after recording it above.
                _git("checkout", "--", ".", root=work_root)
                _git("clean", "-fd", root=work_root)
            res.attempts.append(att)
            print(f"[{entry['id']}] attempt {i}: {att['verdict']} "
                  f"({att['duration_s']:.1f}s, exit {att['exit_code']})")
        res.verdict = ("PASS" if all(a["verdict"] == "PASS" for a in res.attempts)
                       else "FAIL")

    t_all = time.monotonic()
    if jobs == 1:
        for entry in pool_entries + serial_entries:
            do_entry(entry, REPO_ROOT, None, echo=True)
            run_data["results"] = {
                k: {"verdict": v.verdict, "attempts": v.attempts}
                for k, v in results.items()}
            (run_dir / "run.json").write_text(json.dumps(run_data, indent=2))
            if args.fail_fast and results[entry["id"]].verdict == "FAIL":
                stopped_early = True
                break
    else:
        snapshot = _git("stash", "create").stdout.strip() or commit
        untracked = _git("ls-files", "--others", "--exclude-standard").stdout.strip()
        if untracked:
            print("BLOCKED: --jobs>1 requires no untracked (non-ignored) "
                  "files; workers are git worktrees of a snapshot commit:\n"
                  + untracked)
            run_data["verdict"] = "BLOCKED"
            (run_dir / "run.json").write_text(json.dumps(run_data, indent=2))
            return 2
        # Workers must live OUTSIDE the repo tree: inside it, the bind
        # mount makes the inherited cwd path string stop existing, and
        # getcwd() becomes ambiguous (tools reopening cwd by path break).
        workers_dir = (Path(os.environ.get("ARENA_TEST_WORKERS_DIR",
                                           "/tmp/arena-test-workers"))
                       / run_id)
        prefixes: list[tuple[Path, list[str]]] = []
        try:
            for i in range(jobs):
                wroot = workers_dir / f"w{i}"
                prefixes.append((wroot, worker_prepare(wroot, snapshot)))
            # Static round-robin assignment keeps each worker's tree warm
            # and makes the schedule reproducible.
            queues: list[list[dict]] = [[] for _ in range(jobs)]
            for idx, entry in enumerate(pool_entries):
                queues[idx % jobs].append(entry)

            def work(i: int) -> None:
                wroot, prefix = prefixes[i]
                for entry in queues[i]:
                    do_entry(entry, wroot, prefix, echo=False)
                    if args.fail_fast and results[entry["id"]].verdict == "FAIL":
                        break

            with concurrent.futures.ThreadPoolExecutor(max_workers=jobs) as ex:
                list(ex.map(lambda i: work(i), range(jobs)))
        finally:
            for wroot, _ in prefixes:
                _git("worktree", "remove", "--force", str(wroot))
            shutil.rmtree(workers_dir, ignore_errors=True)
        for entry in serial_entries:
            if all(results[e["id"]].verdict == "PASS" for e in pool_entries) \
                    or not args.fail_fast:
                do_entry(entry, REPO_ROOT, None, echo=True)

    total_dt = time.monotonic() - t_all
    run_data["results"] = {
        k: {"verdict": v.verdict, "attempts": v.attempts}
        for k, v in results.items()}
    n_pass = sum(1 for v in results.values() if v.verdict == "PASS")
    n_fail = sum(1 for v in results.values() if v.verdict == "FAIL")
    status_end = [ln for ln in _git("status", "--porcelain").stdout.splitlines() if ln]
    source_changed = status_end != src["status_lines"]
    run_data["source_status_at_end"] = status_end
    if blocked_reasons and n_fail == 0:
        verdict = "BLOCKED"
    elif n_fail or stopped_early:
        verdict = "FAIL"
    elif source_changed:
        verdict = "FAIL"
        run_data["source_changed_during_run"] = True
    else:
        verdict = "PASS"
    run_data["verdict"] = verdict
    run_data["duration_s"] = round(total_dt, 1)
    run_data["finished_utc"] = _utcnow()
    (run_dir / "run.json").write_text(json.dumps(run_data, indent=2))
    # Convenience pointer only; run dirs themselves are the evidence.
    latest = REPO_ROOT / "build" / "test-runs" / "latest"
    try:
        if latest.is_symlink() or latest.exists():
            latest.unlink()
        latest.symlink_to(run_dir.name)
    except OSError:
        pass
    print("=" * 70)
    print(f"RUN {run_id}: {verdict} — {n_pass} passed, {n_fail} failed, "
          f"{len(omitted)} omitted, {total_dt:.1f}s")
    print(f"evidence level: {run_data['evidence_level']}")
    for b in blocked_reasons:
        print(f"  BLOCKED: {b}")
    if source_changed:
        print("  SOURCE CHANGED DURING RUN (see git status)")
    print(f"evidence: {run_dir}")
    return {"PASS": 0, "FAIL": 1, "BLOCKED": 2}[verdict]


# ------------------------------------------------------------------ CLI


def main(argv: list[str]) -> int:
    ap = argparse.ArgumentParser(
        description="ArenaOS test runner with preserved evidence. "
                    "See docs/testing/README.md.")
    sub = ap.add_subparsers(dest="cmd", required=True)

    def add_run_options(p):
        p.add_argument("--jobs", type=int, default=1,
                       help="parallel isolated workers (default 1 = serial, safest)")
        p.add_argument("--no-build-cache", action="store_true",
                       help="disable the content-addressed build cache")
        p.add_argument("--fail-fast", action="store_true",
                       help="stop scheduling after the first failure (run is INCOMPLETE/FAIL)")
        p.add_argument("--force", action="store_true",
                       help="qualify only: run despite unmet preconditions (verdict still BLOCKED-graded as recorded)")

    for name in ("quick", "full", "qualify"):
        p = sub.add_parser(name)
        add_run_options(p)
    p = sub.add_parser("affected")
    add_run_options(p)
    p.add_argument("--base", default="main",
                   help="git ref the change set is computed against (default: main)")
    p.add_argument("--explain", action="store_true",
                   help="print the selection and exit without running")
    p = sub.add_parser("suite")
    add_run_options(p)
    p.add_argument("ids", nargs="+", help="registry entry ids (see: arena_test.py list)")
    p.add_argument("--repeat", type=int, default=1,
                   help="run each suite N times; ALL attempts preserved; any failure fails the run")
    p = sub.add_parser("list")
    p.add_argument("--mode", default=None,
                   choices=["quick", "full", "qualify", "registered"])
    sub.add_parser("check-registry")
    sub.add_parser("env-info")
    p = sub.add_parser("verify-run")
    p.add_argument("run_dir")
    p = sub.add_parser("inventory", help="print the INVENTORY.md markdown")
    p.add_argument("--durations", default=None,
                   help="optional TSV (entry_id<TAB>seconds) to merge in")

    args = ap.parse_args(argv)

    if args.cmd == "env-info":
        print(json.dumps(environment_identity(runner_env()), indent=2))
        return 0
    if args.cmd == "check-registry":
        problems = check_registry(load_registry())
        if problems:
            for p in problems:
                print(f"REGISTRY PROBLEM: {p}")
            return 1
        print(f"registry OK: {len(load_registry()['entries'])} entries, "
              f"{load_registry()['registered_count']} registered")
        return 0
    if args.cmd == "verify-run":
        problems = verify_run(Path(args.run_dir))
        if problems:
            for p in problems:
                print(f"VERIFY FAIL: {p}")
            return 1
        print(f"run evidence complete: {args.run_dir}")
        return 0
    if args.cmd == "list":
        registry = load_registry()
        entries = registry["entries"]
        if args.mode in ("quick", "full", "qualify"):
            entries = select_mode(registry, args.mode)
        elif args.mode == "registered":
            entries = [e for e in entries if e["qualification"] == "registered"]
        for e in entries:
            print(f"{e['id']:42s} {e['kind']:16s} {e['qualification']:18s} {e['title']}")
        print(f"{len(entries)} entries")
        return 0
    if args.cmd == "inventory":
        registry = load_registry()
        durations = {}
        if args.durations:
            for line in Path(args.durations).read_text().splitlines():
                if "\t" in line:
                    k, v = line.split("\t", 1)
                    durations[k.strip()] = v.strip()
        print(render_inventory(registry, durations))
        return 0

    registry = load_registry()
    problems = check_registry(registry)
    if problems:
        for p in problems:
            print(f"REGISTRY PROBLEM: {p}", file=sys.stderr)
        print("refusing to run with an incomplete registry", file=sys.stderr)
        return 2
    if args.cmd in ("quick", "full", "qualify"):
        entries = select_mode(registry, args.cmd)
        return execute(entries, args.cmd, args, None)
    if args.cmd == "affected":
        pathmap = load_map()
        changed = changed_files(args.base)
        entries, report = select_affected(registry, pathmap, changed)
        if args.explain:
            print(json.dumps({"changed": changed,
                              "selected": [e["id"] for e in entries],
                              "report": report}, indent=2))
            return 0
        args.repeat = 1
        return execute(entries, "affected", args, report)
    if args.cmd == "suite":
        by_id = entries_by_id(registry)
        unknown = [i for i in args.ids if i not in by_id]
        if unknown:
            print(f"unknown suite ids: {unknown}", file=sys.stderr)
            return 2
        return execute([by_id[i] for i in args.ids], "suite", args, None)
    return 2


def render_inventory(registry: dict, durations: dict) -> str:
    lines = [
        "# ArenaOS test inventory (generated)",
        "",
        "Generated by `python3 tools/arena_test.py inventory` from",
        "`tools/test_registry.json`. Do not edit by hand; update the registry",
        "(see docs/testing/AGENT-GUIDE.md) and regenerate. Durations, when",
        "present, are merged from the T workstream receipts on one sandbox",
        "(2 vCPU, pinned patched TCG backend) and are advisory only.",
        "",
        "| Entry | Kind | Qualification role | Source | Mutates source | Parallel | ~Duration (s) | Purpose |",
        "|---|---|---|---|---|---|---|---|",
    ]
    for e in registry["entries"]:
        dur = durations.get(e["id"], e.get("duration_s", ""))
        lines.append(
            f"| {e['id']} | {e['kind']} | {e['qualification']} | "
            f"{', '.join(e.get('sources', []))} | "
            f"{'yes' if e.get('mutates_source') else 'no'} | {e['parallel']} | "
            f"{dur} | {e['title']} |")
    lines.append("")
    counts: dict[str, int] = {}
    for e in registry["entries"]:
        counts[e["qualification"]] = counts.get(e["qualification"], 0) + 1
    lines.append("Counts by qualification role: "
                 + ", ".join(f"{k}={v}" for k, v in sorted(counts.items())))
    lines.append(f"Total entries: {len(registry['entries'])}; registered "
                 f"(run_tests.sh-equivalent): {registry['registered_count']}.")
    return "\n".join(lines) + "\n"


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
