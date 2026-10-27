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
        parent = Path(td)
        root = parent / "repo"
        kernel = root / "kernel"
        (kernel / "src").mkdir(parents=True)
        home = root / "home"
        cargo_home = root / "cargo-home"
        home.mkdir()
        cargo_home.mkdir()
        input_file = kernel / "a.txt"
        input_file.write_text("alpha")
        (kernel / "Cargo.toml").write_text(
            '[package]\nname = "synthetic"\nversion = "0.1.0"\n'
            '[dependencies]\next = { path = "../external-dep" }\n')
        external_dep = root / "external-dep"
        external_dep.mkdir()
        (external_dep / "Cargo.toml").write_text(
            '[package]\nname = "ext"\nversion = "0.1.0"\n')
        external_value = external_dep / "value.txt"
        external_value.write_text("one")
        (kernel / "src" / "lib.rs").write_text(
            'pub const X: Option<&str> = option_env!("ARENA_SYNTHETIC_OPTION");\n')
        (kernel / "build.rs").write_text(
            'fn main() { println!("cargo:rerun-if-env-changed='
            'SYNTHETIC_BUILD_ENV"); }\n')
        linked_file = parent / "linked.txt"
        linked_file.write_text("linked-one")
        (kernel / "linked.txt").symlink_to(linked_file)
        linked_dir = parent / "linked-dir"
        linked_dir.mkdir()
        linked_dir_file = linked_dir / "value.txt"
        linked_dir_file.write_text("dir-one")
        (kernel / "linked-dir").symlink_to(linked_dir, target_is_directory=True)

        old_trees, old_files = build_cache._INPUT_TREES, build_cache._INPUT_FILES
        old_toolchain = build_cache._toolchain_identity
        old_env = os.environ.get("ARENA_TEST_BUILD_CACHE_DIR")
        build_cache._INPUT_TREES = ("kernel",)
        build_cache._INPUT_FILES = ()
        build_cache._toolchain_identity = (
            lambda env, root=build_cache.REPO_ROOT, targets=None:
            {"synthetic": True, "blockers": []})
        os.environ["ARENA_TEST_BUILD_CACHE_DIR"] = str(root / "cache")
        try:
            env = {
                "PATH": os.environ.get("PATH", ""),
                "HOME": str(home),
                "CARGO_HOME": str(cargo_home),
            }

            def status(e=None):
                return build_cache.fingerprint_status(env if e is None else e,
                                                      root)

            fp1, cfg1 = status()
            check(fp1 is not None,
                  f"synthetic cache identity is usable {cfg1.get('reuse')}")
            if fp1 is None:
                return
            input_file.write_text("beta")  # relevant input changes
            fp2, _ = status()
            check(fp1 != fp2, "cache fingerprint invalidates on input change")
            input_file.write_text("alpha")
            os.utime(input_file, (1_700_000_000, 1_700_000_000))
            fp3, _ = status()
            check(fp3 == fp1, "fingerprint is content-stable (not mtime-based)")
            check(build_cache.fingerprint(dict(
                env, ARENA_GRAPHICS_FIXTURE="phase9"), root)[0] != fp1,
                "fingerprint includes effective build environment")
            check(build_cache.fingerprint(dict(
                env, CARGO_PROFILE_RELEASE_OPT_LEVEL="0"), root)[0] != fp1,
                "CARGO_PROFILE_* overrides invalidate the fingerprint (C repro)")
            check(build_cache.fingerprint(dict(
                env, CARGO_BUILD_TARGET="x86_64-unknown-none"), root)[0] != fp1,
                "CARGO_BUILD_TARGET invalidates the fingerprint")
            check(build_cache.fingerprint(dict(
                env, ARENA_SYNTHETIC_OPTION="1"), root)[0] != fp1,
                "source-declared option_env! variables invalidate")
            check(build_cache.fingerprint(dict(
                env, SYNTHETIC_BUILD_ENV="1"), root)[0] != fp1,
                "build-script rerun-if-env-changed variables invalidate")
            check(build_cache.fingerprint(dict(
                env, UNRELATED_SETTING="x", SOME_API_TOKEN="secret"),
                root)[0] == fp1,
                "unrelated/secret environment is not fingerprinted")

            # Repository-root Cargo config (C's second repro): comments do
            # not change the effective input, rustflags do.
            root_cfg_dir = root / ".cargo"
            root_cfg_dir.mkdir()
            root_cfg = root_cfg_dir / "config.toml"
            root_cfg.write_text(
                '[build]\nrustflags = ["-C", "target-cpu=generic"]\n')
            fp_root_cfg, _ = status()
            check(fp_root_cfg not in (None, fp1),
                  "root .cargo/config.toml rustflags invalidate (C repro)")
            root_cfg.write_text(
                '# a comment\n[build]\nrustflags=["-C","target-cpu=generic"]\n')
            check(status()[0] == fp_root_cfg,
                  "identical effective Cargo config hits despite comments")
            root_cfg.write_text(
                '[build]\nrustflags = ["-C", "target-cpu=native"]\n')
            check(status()[0] not in (None, fp_root_cfg),
                  "changed root rustflags produce a new fingerprint")
            root_cfg.unlink()
            check(status()[0] == fp1, "removing root config restores identity")

            parent_cfg_dir = parent / ".cargo"
            parent_cfg_dir.mkdir()
            parent_cfg = parent_cfg_dir / "config.toml"
            parent_cfg.write_text(
                '[target.x86_64-unknown-none]\n'
                'rustflags = ["-C", "force-frame-pointers=yes"]\n')
            check(status()[0] not in (None, fp1),
                  "ancestor Cargo config invalidates the fingerprint")
            parent_cfg.unlink()
            home_cfg = cargo_home / "config.toml"
            home_cfg.write_text('[build]\nincremental = true\n')
            check(status()[0] not in (None, fp1),
                  "CARGO_HOME config invalidates the fingerprint")
            home_cfg.unlink()
            check(status()[0] == fp1,
                  "ancestor/CARGO_HOME config removal restores identity")

            linked_file.write_text("linked-two")
            check(status()[0] not in (None, fp1),
                  "symlink target *contents* invalidate the fingerprint")
            linked_file.write_text("linked-one")
            linked_dir_file.write_text("dir-two")
            check(status()[0] not in (None, fp1),
                  "symlinked-directory contents invalidate")
            linked_dir_file.write_text("dir-one")
            external_value.write_text("two")
            check(status()[0] not in (None, fp1),
                  "path-dependency contents outside the trees invalidate")
            external_value.write_text("one")
            check(status()[0] == fp1,
                  "restoring symlink/external contents restores identity")

            # Unsupported configurations disable reuse explicitly.
            root_cfg.write_text(
                '[target.x86_64-unknown-none]\nlinker = "/bin/false"\n')
            fp_bad, cfg_bad = status()
            check(fp_bad is None and any("linker" in b for b in
                                         cfg_bad["reuse"]["blockers"]),
                  "explicit non-sysroot linker disables reuse with a reason")
            root_cfg.write_text('[build\nrustflags = [\n')
            check(status()[0] is None,
                  "malformed Cargo config disables reuse")
            root_cfg.write_text('[build]\ntarget = "custom-target.json"\n')
            check(status()[0] is None,
                  "missing custom target spec disables reuse")
            custom_target = root / "custom-target.json"
            custom_target.write_text('{"arch": "x86_64"}')
            fp_custom, _ = status()
            custom_target.write_text('{"arch": "x86_64", "os": "none"}')
            check(fp_custom is not None and status()[0] not in (None, fp_custom),
                  "custom target spec contents are fingerprinted")
            root_cfg.unlink()
            custom_target.unlink()
            check(status()[0] == fp1, "identity restored for store controls")
            try:
                build_cache.fingerprint(
                    dict(env, RUSTC_WRAPPER="sccache"), root)
                raised = False
            except ValueError:
                raised = True
            check(raised, "rustc wrapper disables reuse (fingerprint raises)")

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
            check(build_cache.lookup(fp_root_cfg) is None,
                  "a changed Cargo config cannot restore the previous image")
            entry = build_cache.entry_path(fp1)
            (entry / "arena-esp.img").write_bytes(b"tampered")
            check(build_cache.lookup(fp1) is None,
                  "tampered artifact fails lookup (hash check)")
            (entry / "arena-esp.img").write_bytes(b"arena-esp.img-v1")
            meta_path = entry / "meta.json"
            meta = json.loads(meta_path.read_text())
            broken_meta = json.loads(json.dumps(meta))
            del broken_meta["artifacts"]["arena-esp.img"]
            meta_path.write_text(json.dumps(broken_meta))
            check(build_cache.lookup(fp1) is None,
                  "incomplete artifact metadata is rejected")
            broken_meta = json.loads(json.dumps(meta))
            broken_meta["config"]["reuse"]["enabled"] = False
            meta_path.write_text(json.dumps(broken_meta))
            check(build_cache.lookup(fp1) is None,
                  "entry recorded under a disabled identity is rejected")
            meta_path.write_text(json.dumps(meta))
            (entry / "arena-boot.efi").unlink()
            check(build_cache.lookup(fp1) is None,
                  "missing artifact is rejected")
            (entry / "arena-boot.efi").write_bytes(b"arena-boot.efi-v1")
            (entry / "COMPLETE").unlink()
            check(build_cache.lookup(fp1) is None,
                  "incomplete entry (no COMPLETE) is rejected")
        finally:
            build_cache._INPUT_TREES, build_cache._INPUT_FILES = old_trees, old_files
            build_cache._toolchain_identity = old_toolchain
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


def test_attempt_evidence_sequences() -> None:
    import subprocess

    tools_dir = Path(__file__).resolve().parent
    with tempfile.TemporaryDirectory() as td:
        td = Path(td)
        attempt = td / "attempt"
        src = td / "serial.log"
        src.write_text("guest evidence\n")
        code = (
            "import sys\n"
            f"sys.path.insert(0, {str(tools_dir)!r})\n"
            "import mtest\n"
            "from pathlib import Path\n"
            "src = Path(sys.argv[1])\n"
            "for label in sys.argv[2:]:\n"
            "    mtest._attempt_write('boots', label, {'label': label}, "
            "{'serial.log': src})\n"
        )
        env = dict(os.environ, ARENA_TEST_ATTEMPT_DIR=str(attempt))
        for labels in (("first-boot", "second-boot"), ("first-boot", "third-boot")):
            r = subprocess.run(
                [sys.executable, "-c", code, str(src), *labels],
                env=env, capture_output=True, text=True, timeout=60)
            if r.returncode != 0:
                check(False, f"evidence sequence subprocess failed: {r.stderr.strip()}")
                return
        dirs = sorted(p for p in (attempt / "boots").iterdir() if p.is_dir())
        prefixes = [p.name.split("-", 1)[0] for p in dirs]
        labels = sorted(json.loads((p / "boot.json").read_text())["label"]
                        for p in dirs)
        check(prefixes == ["001", "002", "003", "004"],
              "per-boot evidence directories are monotonic across processes")
        check(labels == ["first-boot", "first-boot", "second-boot", "third-boot"],
              "re-imported harness processes cannot overwrite boot evidence")


def _pid_alive(pid: int) -> bool:
    try:
        os.kill(pid, 0)
        return True
    except ProcessLookupError:
        return False
    except PermissionError:
        return True


def test_output_independent_timeouts() -> None:
    import time

    import arena_test

    cases = [
        ("no output", "import time; time.sleep(3)", b""),
        ("print then stall",
         "import time; print('hello', flush=True); time.sleep(3)", b"hello"),
        ("partial line",
         "import sys, time; sys.stdout.write('partial'); "
         "sys.stdout.flush(); time.sleep(3)", b"partial"),
        ("continuous output",
         "import time\n"
         "end = time.monotonic() + 3\n"
         "while time.monotonic() < end:\n"
         "    print('tick', flush=True)\n"
         "    time.sleep(0.02)\n", b"tick"),
    ]
    with tempfile.TemporaryDirectory() as td:
        work = Path(td)
        for idx, (name, code, needle) in enumerate(cases):
            entry = {
                "id": f"timeout-{idx}",
                "cmd": [sys.executable, "-c", code],
                "cwd": ".", "timeout_s": 1,
            }
            att_dir = work / f"attempt-{idx}"
            t0 = time.monotonic()
            result = arena_test.run_attempt(entry, att_dir, dict(os.environ),
                                            work, False)
            elapsed = time.monotonic() - t0
            out = (att_dir / "output.log").read_bytes()
            check(result["verdict"] == "FAIL" and result["timed_out"]
                  and 0.9 <= elapsed < 2.5 and needle in out,
                  f"timeout is output-independent: {name} "
                  f"(elapsed {elapsed:.2f}s, output preserved)")

        # A descendant in the same process group must not survive.
        pidfile = work / "descendant.pid"
        code = (
            "import pathlib, subprocess, sys, time\n"
            "p = subprocess.Popen([sys.executable, '-c', "
            "'import time; time.sleep(3)'])\n"
            f"pathlib.Path({str(pidfile)!r}).write_text(str(p.pid))\n"
            "print('parent', flush=True)\n"
            "time.sleep(3)\n")
        entry = {"id": "timeout-descendant",
                 "cmd": [sys.executable, "-c", code],
                 "cwd": ".", "timeout_s": 1}
        result = arena_test.run_attempt(entry, work / "attempt-desc",
                                        dict(os.environ), work, False)
        pid = int(pidfile.read_text())
        for _ in range(50):
            if not _pid_alive(pid):
                break
            time.sleep(0.02)
        check(result["timed_out"] and not _pid_alive(pid),
              "timeout terminates and reaps the whole process group")

        # A suite that exits 0 while a descendant holds stdout open must
        # not hang the runner; the descendant is cleaned up.
        pidfile2 = work / "pipeholder.pid"
        code = (
            "import pathlib, subprocess, sys\n"
            "p = subprocess.Popen([sys.executable, '-c', "
            "'import time; time.sleep(3)'])\n"
            f"pathlib.Path({str(pidfile2)!r}).write_text(str(p.pid))\n"
            "print('parent done', flush=True)\n")
        entry = {"id": "pipe-holder", "cmd": [sys.executable, "-c", code],
                 "cwd": ".", "timeout_s": 5}
        t0 = time.monotonic()
        result = arena_test.run_attempt(entry, work / "attempt-pipe",
                                        dict(os.environ), work, False)
        elapsed = time.monotonic() - t0
        pid = int(pidfile2.read_text())
        for _ in range(50):
            if not _pid_alive(pid):
                break
            time.sleep(0.02)
        check(result["verdict"] == "PASS" and not result["timed_out"]
              and elapsed < 2.5 and result["descendants_terminated"]
              and not _pid_alive(pid),
              "descendant holding the pipe neither hangs nor survives the attempt")


def test_evidence_copy_failure_fails_attempt() -> None:
    import arena_test

    tools_dir = Path(__file__).resolve().parent
    with tempfile.TemporaryDirectory() as td:
        work = Path(td)
        src = work / "serial.log"
        src.write_text("guest evidence\n")
        script = work / "evidence_fail.py"
        script.write_text(
            "import sys\n"
            f"sys.path.insert(0, {str(tools_dir)!r})\n"
            "import mtest\n"
            "from pathlib import Path\n"
            "def boom(a, b):\n"
            "    raise OSError('simulated evidence copy failure')\n"
            "mtest.shutil.copyfile = boom\n"
            f"mtest._attempt_write('boots', 'copy-fail', "
            f"{{'label': 'copy-fail'}}, {{'serial.log': Path({str(src)!r})}})\n"
            "print('suite body succeeded')\n")
        entry = {"id": "evidence-copy-fail",
                 "cmd": [sys.executable, str(script)],
                 "cwd": ".", "timeout_s": 60}
        att_dir = work / "attempt"
        result = arena_test.run_attempt(entry, att_dir, dict(os.environ),
                                        work, False)
        check(result["exit_code"] == 0 and result["verdict"] == "FAIL"
              and result["evidence_errors"],
              "required evidence-copy failure cannot produce a PASS attempt")
        check(any((att_dir / "evidence-errors").glob("*/error.json")),
              "evidence-copy failure leaves a verifier-visible sentinel")


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
    gate = (b"arena>", (b"packaged READY", b"boot namespace scan refused"))
    check(not mtest._markers_satisfied(
        b"arena> packaged READY boot namespace scan refused", gate, 2),
        "marker gate: any-of does not sum alternatives toward nth")
    check(mtest._markers_satisfied(
        b"arena> arena> packaged READY packaged READY", gate, 2),
        "marker gate: one alternative at nth settles the conjunction")


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

        def clone(name: str) -> Path:
            dest = Path(td) / name / "extracted-run"
            dest.parent.mkdir(parents=True)
            shutil.copytree(new[0], dest)
            return dest

        def reseal(path: Path, mutate) -> None:
            rec = json.loads((path / "run.json").read_text())
            mutate(rec)
            arena_test._seal_run(path, rec)

        # Verification is portable: it must resolve evidence inside the
        # extracted copy even when the original absolute paths are gone.
        extracted = clone("portable")
        hidden = new[0].with_name(new[0].name + "-hidden")
        os.rename(new[0], hidden)
        try:
            problems = arena_test.verify_run(extracted)
            check(problems == [],
                  f"run verifies after extraction elsewhere {problems}")
        finally:
            os.rename(hidden, new[0])

        # C's fabricated empty PASS is rejected, not graded complete.
        fake = Path(td) / "fake-full"
        fake.mkdir()
        (fake / "run.json").write_text(json.dumps({
            "run_id": "fake", "mode": "full", "source": {},
            "environment": {}, "results": {}, "verdict": "PASS"}))
        check(arena_test.verify_run(fake) != [],
              "fabricated empty full PASS fails verification")

        p = clone("missing-selected")
        reseal(p, lambda rec: rec.update(results={}))
        check(arena_test.verify_run(p) != [],
              "missing selected entry result fails verification")
        p = clone("missing-attempt")
        shutil.rmtree(p / "attempts/flaky-fake/attempt-002")
        reseal(p, lambda rec: None)
        check(arena_test.verify_run(p) != [],
              "missing required attempt fails verification")
        p = clone("missing-output")
        (p / "attempts/flaky-fake/attempt-001/output.log").unlink()
        reseal(p, lambda rec: None)
        check(arena_test.verify_run(p) != [],
              "missing raw attempt output fails verification")
        p = clone("contradictory")

        def contradict(rec):
            rec["verdict"] = "PASS"
            rec["results"]["flaky-fake"]["verdict"] = "PASS"
            rec["counts"] = {"selected": 1, "pass": 1, "fail": 0,
                             "not_run": 0, "attempts": 2}
        reseal(p, contradict)
        check(arena_test.verify_run(p) != [],
              "PASS over a preserved failed attempt fails verification")
        p = clone("interrupted")

        def interrupt(rec):
            rec["status"] = "RUNNING"
            rec["completion"]["state"] = "RUNNING"
            rec["finished_utc"] = None
        reseal(p, interrupt)
        check(arena_test.verify_run(p) != [],
              "interrupted run cannot satisfy completion verification")

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
    test_attempt_evidence_sequences()
    test_output_independent_timeouts()
    test_evidence_copy_failure_fails_attempt()
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
