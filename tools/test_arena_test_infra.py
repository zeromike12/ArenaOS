#!/usr/bin/env python3
"""Negative controls for the testing infrastructure itself.

Each check targets a failure mode the new runner/cache could introduce:
stale-cache reuse, mutated-vs-canonical confusion, evidence collision,
hidden retries, missing-evidence-as-pass, QEMU-exit-0-as-pass,
accelerator-init-failure-as-pass, and affected-selection silently
running nothing. Host-only except one deliberately broken fake-QEMU
boot that never starts a guest. See docs/testing/ARCHITECTURE.md.
"""

from __future__ import annotations

import json
import os
import shutil
import sys
import tempfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

FAILURES: list[str] = []


def check(cond: bool, why: str) -> None:
    print(f"[infra] {'PASS' if cond else 'FAIL'}: {why}", flush=True)
    if not cond:
        FAILURES.append(why)


def test_cache_fingerprint_and_store() -> None:
    import build_cache

    with tempfile.TemporaryDirectory() as td:
        root = Path(td)
        (root / "kernel").mkdir()
        input_file = root / "kernel" / "a.txt"
        input_file.write_text("alpha")
        old_trees, old_files = build_cache._INPUT_TREES, build_cache._INPUT_FILES
        old_env = os.environ.get("ARENA_TEST_BUILD_CACHE_DIR")
        build_cache._INPUT_TREES = ("kernel",)
        build_cache._INPUT_FILES = ()
        os.environ["ARENA_TEST_BUILD_CACHE_DIR"] = str(root / "cache")
        try:
            env = dict(os.environ)
            fp1, cfg1 = build_cache.fingerprint(env, root)
            input_file.write_text("beta")  # relevant input changes
            fp2, _ = build_cache.fingerprint(env, root)
            check(fp1 != fp2, "cache fingerprint invalidates on input change")
            input_file.write_text("alpha")
            fp3, _ = build_cache.fingerprint(env, root)
            check(fp3 == fp1, "fingerprint is content-stable (not mtime-based)")
            env_phase9 = dict(env, ARENA_GRAPHICS_FIXTURE="phase9")
            fp4, _ = build_cache.fingerprint(env_phase9, root)
            check(fp4 != fp1, "fingerprint includes effective build environment")

            # Store/lookup/restore, then each tamper must fail closed.
            (root / "build").mkdir()
            for name in build_cache._ARTIFACTS:
                (root / "build" / name).write_bytes(f"{name}-v1".encode())
            stored = build_cache.store(fp1, cfg1, "log", 1.0, root)
            check(stored is not None and build_cache.lookup(fp1) is not None,
                  "complete cache entry is served")
            for name in build_cache._ARTIFACTS:
                (root / "build" / name).write_bytes(b"clobbered")
            ok = build_cache.restore(build_cache.lookup(fp1), root)
            check(ok and (root / "build/arena-esp.img").read_bytes() == b"arena-esp.img-v1",
                  "restore copies identified artifacts and verifies hashes")
            check(build_cache.lookup(fp2) is None,
                  "mutated-input fingerprint cannot hit the canonical entry")
            entry = build_cache.entry_path(fp1)
            (entry / "arena-esp.img").write_bytes(b"tampered")
            check(build_cache.lookup(fp1) is None,
                  "tampered artifact fails lookup (hash check)")
            (entry / "arena-esp.img").write_bytes(b"arena-esp.img-v1")
            (entry / "COMPLETE").unlink()
            check(build_cache.lookup(fp1) is None,
                  "incomplete entry (no COMPLETE) is rejected")
        finally:
            build_cache._INPUT_TREES, build_cache._INPUT_FILES = old_trees, old_files
            if old_env is None:
                os.environ.pop("ARENA_TEST_BUILD_CACHE_DIR", None)
            else:
                os.environ["ARENA_TEST_BUILD_CACHE_DIR"] = old_env


def test_unique_run_dirs() -> None:
    import arena_test

    made: list[Path] = []
    try:
        ids = set()
        for _ in range(50):
            run_id, path = arena_test.make_run_dir("infra", "deadbeefcafe", False)
            ids.add(run_id)
            made.append(path)
        check(len(ids) == 50, "50 run ids allocated without collision")
    finally:
        for p in made:
            shutil.rmtree(p, ignore_errors=True)


def test_semantic_verdicts() -> None:
    import mtest

    good = ("m3: RESULT PASS (13/13)\nm4: RESULT PASS (9/9)\n"
            "halting via UEFI ResetSystem(shutdown)\n")
    check(mtest.semantic_exit_rc(0, good, "t") == 0,
          "clean serial + QEMU 0 is a pass")
    check(mtest.semantic_exit_rc(0, good + "[arena ERROR halt]\n", "t") == 97,
          "semantic guest failure rejected despite QEMU exit 0 (fatal marker)")
    check(mtest.semantic_exit_rc(0, "m3: RESULT PASS (13/13)\n", "t") == 97,
          "semantic guest failure rejected despite QEMU exit 0 (missing markers)")
    check(mtest.semantic_exit_rc(1, good, "t") == 1,
          "nonzero QEMU exit stays a failure")
    check(mtest.semantic_exit_rc(None, good, "t") is None,
          "killed boot (None) is never turned into success")
    check(mtest._markers_satisfied(b"arena> packaged READY", b"arena>", 1),
          "marker gate: single marker")
    check(mtest._markers_satisfied(
        b"arena> packaged READY", (b"arena>", b"packaged READY"), 1),
        "marker gate: flat tuple is a conjunction")
    check(not mtest._markers_satisfied(b"arena>", (b"arena>", b"packaged READY"), 1),
          "marker gate: conjunction refuses partial readiness")
    check(mtest._markers_satisfied(
        b"arena> boot namespace scan refused",
        (b"arena>", (b"packaged READY", b"boot namespace scan refused")), 1),
        "marker gate: nested tuple is any-of (refusal settles)")
    check(not mtest._markers_satisfied(
        b"arena>", (b"arena>", (b"packaged READY", b"boot namespace scan refused")), 1),
        "marker gate: any-of still refuses when neither settled")


def test_accelerator_failure_cannot_pass() -> None:
    import arena_env
    import mtest

    try:
        arena_env.ovmf_code()
        arena_env.ovmf_vars_template()
    except SystemExit:
        print("[infra] SKIP: no OVMF in this environment for the fake-QEMU control")
        return
    with tempfile.TemporaryDirectory() as td:
        fake = Path(td) / "fake-qemu.sh"
        fake.write_text("#!/bin/sh\necho 'Could not initialize TCG: broken' >&2\nexit 1\n")
        fake.chmod(0o755)
        esp = Path(td) / "dummy-esp.img"
        esp.write_bytes(b"\0" * 4096)
        old = os.environ.get("ARENA_QEMU")
        os.environ["ARENA_QEMU"] = str(fake)
        try:
            rc, _serial, _dt = mtest.run_qemu("infra-fake-qemu", esp)
        finally:
            if old is None:
                os.environ.pop("ARENA_QEMU", None)
            else:
                os.environ["ARENA_QEMU"] = old
        check(rc not in (0, None),
              "accelerator/process initialization failure cannot pass")
        check(mtest.LAST_QEMU_EXIT.get("infra-fake-qemu") == 1,
              "actual QEMU process exit is preserved separately")


def test_affected_selection() -> None:
    import arena_test

    registry = arena_test.load_registry()
    check(arena_test.check_registry(registry) == [],
          "registry covers every suite file and the 121 registered count")
    pathmap = arena_test.load_map()

    _sel, rep = arena_test.select_affected(
        registry, pathmap, ["some/never-classified-thing.xyz"])
    selected_ids = {e["id"] for e in _sel}
    registered = {e["id"] for e in registry["entries"]
                  if e["qualification"] == "registered"}
    check(rep["fallback_to_full"] and registered <= selected_ids,
          "unknown path falls back to the full registered set")

    sel, rep = arena_test.select_affected(registry, pathmap, ["docs/TESTING.md"])
    check(not rep["fallback_to_full"]
          and all(e.get("quick") for e in sel),
          "docs-only change selects the quick set explicitly (not nothing)")

    sel, _rep = arena_test.select_affected(
        registry, pathmap, ["tools/test_m84_stage.py"])
    ids = {e["id"] for e in sel}
    check("m84-stage" in ids and "m84-crash" in ids,
          "changed helper suite selects itself and its importers")

    sel, _rep = arena_test.select_affected(
        registry, pathmap, ["userspace/fsd/src/main.rs"])
    ids = {e["id"] for e in sel}
    check("m5" in ids and "m84-crash" in ids and "smoke-m1" in ids,
          "service change selects its milestone suites + smoke")


def test_failed_attempt_stays_visible() -> None:
    import arena_test

    td = tempfile.mkdtemp()
    counter = Path(td) / "count"
    script = Path(td) / "flaky.py"
    script.write_text(
        "import pathlib, sys\n"
        f"p = pathlib.Path({str(counter)!r})\n"
        "n = int(p.read_text()) if p.exists() else 0\n"
        "p.write_text(str(n + 1))\n"
        "print(f'attempt {n + 1}')\n"
        "sys.exit(0 if n >= 1 else 1)\n")
    fake_registry = {
        "schema": 1, "registered_count": 0,
        "entries": [{
            "id": "flaky-fake", "title": "synthetic flaky suite",
            "kind": "host-python", "cmd": [sys.executable, str(script)],
            "cwd": ".", "sources": [], "qualification": "standalone",
            "quick": False, "mutates_source": False, "parallel": "host",
            "timeout_s": 60, "imports": [], "paths": [],
            "negative_controls": "",
        }],
    }
    real_load = arena_test.load_registry
    arena_test.load_registry = lambda: fake_registry
    runs_root = arena_test.REPO_ROOT / "build" / "test-runs"
    before = set(runs_root.iterdir()) if runs_root.is_dir() else set()
    try:
        class Args:
            jobs = 1
            no_build_cache = True
            fail_fast = False
            force = False
            repeat = 2

        rc = arena_test.execute(fake_registry["entries"], "suite", Args(), None)
        check(rc == 1, "a passing second attempt does NOT turn the run green")
        new = [p for p in runs_root.iterdir()
               if p not in before and p.is_dir() and p.name != "latest"]
        check(len(new) == 1, "exactly one run dir created")
        data = json.loads((new[0] / "run.json").read_text())
        attempts = data["results"]["flaky-fake"]["attempts"]
        check([a["verdict"] for a in attempts] == ["FAIL", "PASS"],
              "failed first attempt remains recorded beside the passing retry")
        problems = arena_test.verify_run(new[0])
        check(problems == [], f"run evidence verifies complete {problems}")
        # Missing evidence must fail closed.
        out = Path(attempts[0]["attempt_dir"]) / "output.log"
        out.write_bytes(b"tampered")
        check(arena_test.verify_run(new[0]) != [],
              "tampered/missing evidence fails verification (fail closed)")
        shutil.rmtree(new[0])
    finally:
        arena_test.load_registry = real_load
        shutil.rmtree(td, ignore_errors=True)


def main() -> int:
    test_cache_fingerprint_and_store()
    test_unique_run_dirs()
    test_semantic_verdicts()
    test_affected_selection()
    test_failed_attempt_stays_visible()
    test_accelerator_failure_cannot_pass()
    if FAILURES:
        print(f"[infra] {len(FAILURES)} FAILURE(S)")
        return 1
    print("[infra] ALL INFRASTRUCTURE CONTROLS PASS")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
