#!/usr/bin/env python3
"""C2.7 guest proof: two independent native C applications on ONE static runtime.

EXPERIMENTAL. Not part of the production image. Uses the C1.2 harness helpers
(guest_native.py) for the desktop driver, protected APB1 install, and log view.

Applications (both linked against build/guest-<cc>/lib/libarena_c.a + crt0.o):
  A  c2-console  app/c2_console_app.c   stdio, allocator, time, threads, sync, exit
  B  xxhash-app  app/xxhash_app.c + the UNMODIFIED upstream xxHash 0.8.4
Each is packaged twice: a positive bundle (streams granted) and a negative
bundle (streams NOT granted). Four bundles are installed through the protected
APB1 path and launched from All Applications.

Expected, in launch order (the exact exit-status sequence is part of the verdict):
  1. A without streams   -> exit 58, no application output
  2. B without streams   -> exit 58, no application output
  3. A with streams      -> six PASS groups, RESULT PASS, exit 61, atexit line AFTER RESULT
  4. B with streams      -> corpus digest 1df15d5d2925ddb0 over 319 inputs, RESULT PASS, exit 63

Signing: RFC 8032 development fixture (test-only, never a production key), as in C1.
Evidence label: GUEST. Any missing or extra line, wrong status, or unclean boot fails.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import re
import subprocess
import sys
import time
from pathlib import Path

EXP = Path(__file__).resolve().parent
REPO = EXP.parent.parent
TOOLS = REPO / "tools"
sys.path.insert(0, str(EXP))
sys.path.insert(0, str(TOOLS))

import run as crun  # noqa: E402
import afs1  # noqa: E402
import apb1_format  # noqa: E402
import arena_env  # noqa: E402
import mtest  # noqa: E402
import guest_native as gn  # noqa: E402

LABEL = "c2-runtime"
PACKAGE_ID = b"org.arena.editor"
BUILD_DIR = EXP / "build" / "guest-c2"
DEXPECT_DIGEST = "1df15d5d2925ddb0"
DEXPECT_INPUTS = 319

# (key, app id, display name (search term is the unique word), flags, entry name, source file name)
POS_C2 = dict(key="c2", app_id=b"org.arenaos.c2console", display=b"Quartz Console Probe",
              search="Quartz", flags=28, file=b"mmm-quartz.apb1")
POS_XX = dict(key="xx", app_id=b"org.arenaos.xxhashprobe", display=b"Lumen Hash Probe",
              search="Lumen", flags=12, file=b"mmm-lumen.apb1")
NEG_C2 = dict(key="c2-nostream", app_id=b"org.arenaos.c2nostream", display=b"Ember Stream Less",
              search="Ember", flags=20, file=b"aaa-ember.apb1")
NEG_XX = dict(key="xx-nostream", app_id=b"org.arenaos.xxnostream", display=b"Opal Stream Less",
              search="Opal", flags=4, file=b"aab-opal.apb1")

# Desktop rows are ordered by SOURCE FILE NAME (aaa-ember, aab-opal, mmm-lumen,
# mmm-quartz), 74 px apart, first cell at y=58. Confirmed by two runs: the row-2 install
# wrote the lumen (xxhash) package and the row-3 expectation failed as designed.
ROW_Y = {"c2-nostream": 58, "xx-nostream": 132, "xx": 206, "c2": 280}
EXPECTED_EXITS = [58, 58, 61, 63]

GROUP_NAMES = ["G1 stdio", "G2 stdin", "G3 allocator", "G4 time", "G5 threads", "G6 exit"]
C2_GROUP_RE = re.compile(r"^\[c2-console\] (G\d [\w ]+?) (PASS|FAIL) checks=(\d+)\r?$", re.M)
C2_RESULT_RE = re.compile(r"^\[c2-console\] RESULT PASS groups=6/6 checks=(\d+) failed=0\r?$", re.M)
C2_ATEXIT_RE = re.compile(r"^\[c2-console\] atexit handler ran\r?$", re.M)
C2_STDIN_RE = re.compile(r"^\[c2-console\] stdin class=(would-block|eof|data)\r?$", re.M)
C2_ENTER = "[c2-console] C2 console application entered (ARST v2 gate)"
XX_RESULT_RE = re.compile(r"^\[xxhash-app\] RESULT PASS checks=7 failed=0\r?$", re.M)
XX_DIGEST_RE = re.compile(
    rf"^\[xxhash-app\] corpus inputs={DEXPECT_INPUTS} digest={DEXPECT_DIGEST}\r?$", re.M)
XX_BANNER = "[xxhash-app] xxHash 0.8.4 (upstream, unmodified) on the ArenaOS C runtime"
EXIT_RE = re.compile(r"^\[desktop\] child Process-cap exit status=(\d+)\r?$", re.M)
HEADLESS_MARK = gn.HEADLESS_MARK
RETIRED_MARK = gn.RETIRED_MARK
INSTALL_MARK = gn.INSTALL_MARK


def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def git_head() -> str:
    p = subprocess.run(["git", "rev-parse", "HEAD"], cwd=REPO, capture_output=True, text=True)
    return p.stdout.strip() or "unknown"


def git_dirty() -> list[str]:
    p = subprocess.run(["git", "status", "--porcelain"], cwd=REPO, capture_output=True, text=True)
    return [line for line in p.stdout.splitlines() if line.strip()]


def build_c2_bundles(cc: str) -> tuple[dict, dict]:
    """Build the two applications, audit them, and return ({key: bundle}, identity)."""
    elfs = crun.build_guest(cc)
    identity: dict = {"compiler": cc, "apps": {}, "runtime": {}}
    lib = EXP / "build" / f"guest-{cc}" / "lib" / "libarena_c.a"
    identity["runtime"] = {"archive": str(lib), "archive_sha256": sha256(lib.read_bytes()),
                           "crt0_sha256": sha256((EXP / "build" / f"guest-{cc}" / "crt0.o").read_bytes())}
    bundles: dict[str, bytes] = {}
    for app_name, meta_pos, meta_neg, entry_src in (
        ("c2-console", POS_C2, NEG_C2, "bin/app"),
        ("xxhash-app", POS_XX, NEG_XX, "bin/app"),
    ):
        elf = elfs[app_name]
        audit = crun.audit_elf(elf)
        audit["disasm"] = crun.disassemble_check(elf)
        if audit["loader_failures"] or audit["disasm"]["simd_or_x87_count"]:
            raise SystemExit(f"{app_name} fails the loader/execution audit: {audit}")
        # No debug syscall: the legacy SYS_DEBUG_WRITE sink must not be DEFINED or
        # strongly referenced (a weak undefined arena_legacy_write resolves to NULL).
        nm = subprocess.run(["nm", str(elf)], capture_output=True, text=True).stdout
        for line in nm.splitlines():
            parts = line.split()
            if len(parts) >= 2 and parts[-1] in ("arena_legacy_write", "arena_legacy_serial_enable"):
                kind = parts[-2]
                if kind not in ("w", "W"):
                    raise SystemExit(f"{app_name} links the legacy debug-serial path: {line}")
        text = subprocess.run(["objdump", "-d", str(elf)], capture_output=True, text=True).stdout
        if re.search(r"syscall", text) is None:
            raise SystemExit(f"{app_name} has no syscall instruction (cannot be a native app)")
        elf_bytes = elf.read_bytes()
        for meta in (meta_pos, meta_neg):
            manifest = apb1_format.make_manifest(
                app_id=meta["app_id"], package_id=PACKAGE_ID, display_name=meta["display"],
                version=1, flags=meta["flags"], requested=0, entry=entry_src.encode(),
                icon=b"", width=0, height=0, associations=())
            bundle = apb1_format.build_bundle([(1, entry_src.encode(), elf_bytes)], manifest=manifest)
            parsed = apb1_format.parse_bundle(bundle)
            if [name for _k, name, _s, _d in parsed.files] != [entry_src.encode()]:
                raise SystemExit("APB1 round-trip changed the payload list")
            bundles[meta["key"]] = bundle
            identity["apps"].setdefault(app_name, {})[meta["key"]] = {
                "app_id": meta["app_id"].decode(), "manifest_flags": meta["flags"],
                "bundle_sha256": sha256(bundle), "bundle_size": len(bundle),
                "elf": str(elf), "elf_sha256": sha256(elf_bytes), "elf_size": len(elf_bytes),
                "loader_audit": audit if meta is meta_pos else None,
                "signing": "RFC 8032 development fixture (test-only, not a production key)",
            }
    return bundles, identity




def installed_path(meta: dict) -> str:
    return f"/System/Applications/{meta['app_id'].decode()}/1/bin/app"


def source_path(meta: dict) -> str:
    return f"/Users/user/Desktop/{meta['file'].decode()}"


def _search(d, text: str) -> None:
    """Dismiss stray windows, open the Apps menu, then search and launch.

    The C1 helper clicked once and typed at once; with more desktop icons a stray
    Terminal window took focus, so this variant first closes any open window."""
    d.q.command("input-send-event", events=[d.q._ev("esc", True), d.q._ev("esc", False)])
    time.sleep(0.4)
    d.click(96, 12)
    time.sleep(0.8)
    for _ in range(24):
        d.q.command("input-send-event", events=[d.q._ev("backspace", True), d.q._ev("backspace", False)])
    d.q.type_text(text, gap_s=0.025)
    time.sleep(0.3)
    d.q.command("input-send-event", events=[d.q._ev("ret", True), d.q._ev("ret", False)])


def interaction(disk: Path, bundles: dict, outcome: dict) -> bytes:
    d = gn.Desktop(LABEL)
    try:
        d.wait(lambda: "[desktop] desktop surface shows /Users/user/Desktop" in d.serial(),
               "Desktop did not enumerate the seeded APB1 sources")
        for meta in (NEG_C2, NEG_XX, POS_C2, POS_XX):
            try:
                gn._install(d, disk, ROW_Y[meta["key"]], source_path(meta), bundles[meta["key"]],
                            installed_path(meta), outcome, meta["key"])
            except AssertionError:
                # Diagnostic: list what the installer actually wrote before failing.
                listing = sorted(k for k in gn.tree(disk) if "/System/Applications" in k)
                print("DIAG installed entries:", listing, flush=True)
                raise
        exits_seen = 0
        for meta in (NEG_C2, NEG_XX, POS_C2, POS_XX):
            launches_before = d.serial().count(HEADLESS_MARK)
            _search(d, meta["search"])
            try:
                d.wait(lambda: d.serial().count(HEADLESS_MARK) == launches_before + 1,
                       f"All Applications did not launch {meta['key']}", timeout_s=120)
            except AssertionError:
                # Diagnostic evidence only: the screen at the moment the launch failed.
                d.shot(f"launch-failure-{meta['key']}")
                ppm = arena_env.build_dir() / f"{LABEL}-launch-failure-{meta['key']}.ppm"
                if ppm.exists():
                    gn.ppm_to_png(ppm, BUILD_DIR / f"launch-failure-{meta['key']}.png")
                raise
            exits_seen += 1
            want = exits_seen
            d.wait(lambda: len(EXIT_RE.findall(d.serial())) >= want,
                   f"{meta['key']} produced no exit receipt through the Process cap", timeout_s=300)
            d.wait(lambda: d.serial().count(RETIRED_MARK) >= want,
                   f"{meta['key']} was not retired after exit", timeout_s=120)
            outcome[f"driven-{meta['key']}"] = True
    finally:
        d.q.close()
    return b"shutdown\r"


def judge(serial: str) -> dict:
    view = gn.app_stream_view(serial)
    failures: list[str] = []
    exits = [int(v) for v in EXIT_RE.findall(serial)]
    if exits != EXPECTED_EXITS:
        failures.append(f"exit status sequence is {exits}, expected {EXPECTED_EXITS}")

    # --- application A, positive ---
    groups = {m[0]: (m[1], int(m[2])) for m in C2_GROUP_RE.findall(view)}
    for name in GROUP_NAMES:
        if name not in groups:
            failures.append(f"missing group {name}")
        elif groups[name][0] != "PASS":
            failures.append(f"group {name} FAIL")
    if len(groups) != len(GROUP_NAMES):
        failures.append(f"group line count {len(groups)}, expected {len(GROUP_NAMES)}")
    result = C2_RESULT_RE.search(view)
    if result is None:
        failures.append("exact console RESULT PASS line missing")
    if view.count("[c2-console] RESULT ") != 1:
        failures.append("console RESULT line count is not exactly 1")
    if C2_ENTER not in view:
        failures.append("console entry line missing")
    if view.count(C2_ENTER) != 1:
        failures.append("console entered more than once (or in the negative run)")
    if not C2_STDIN_RE.search(view):
        failures.append("stdin classification line missing")
    atexit = C2_ATEXIT_RE.search(view)
    if atexit is None:
        failures.append("atexit handler line missing")
    elif result is not None and atexit.start() < result.start():
        failures.append("atexit handler ran before the RESULT line")

    # --- application B, positive ---
    xx_banner_count = view.count(XX_BANNER)
    if xx_banner_count != 1:
        failures.append(f"xxhash banner count {xx_banner_count}, expected exactly 1 (positive run only)")
    if XX_DIGEST_RE.search(view) is None:
        failures.append(f"xxhash corpus digest line is not exactly {DEXPECT_DIGEST} over {DEXPECT_INPUTS}")
    if XX_RESULT_RE.search(view) is None:
        failures.append("xxhash RESULT PASS checks=7 failed=0 line missing")

    if "check failed line" in view:
        failures.append("a check failed line was printed")
    return {
        "exit_lines": exits,
        "console_groups": {k: v[0] for k, v in groups.items()},
        "console_checks": int(result.group(1)) if result else None,
        "console_result": result.group(0) if result else None,
        "xxhash_digest_ok": XX_DIGEST_RE.search(view) is not None,
        "failures": failures,
        "pass": not failures,
    }


def one_run(cc: str, bundles: dict, identity: dict, index: int) -> dict:
    esp = mtest.build(LABEL, desktop=True)
    disk = arena_env.make_scratch_disk()
    rc, serial, _ = mtest.boot(LABEL + "-seed", esp, [(b"arena>", 1, b"shutdown\r")], disk, pointer=True)
    if rc != 0 or "no AFS2 region" not in serial:
        return {"run": index, "pass": False, "reason": "AFS2 seed boot failed", "rc": rc}
    with disk.open("r+b") as f:
        f.truncate(arena_env.AFS2_DISK_MIB * 1024 * 1024)
    if afs1.audit(disk):
        return {"run": index, "pass": False, "reason": "AFS1 import area damaged before install"}
    rc, serial, _ = mtest.boot(LABEL + "-afs2", esp, [(b"filesd: AFS2 mounted", 1, b"shutdown\r")],
                               disk, pointer=True)
    if rc != 0 or "filesd: AFS2 mounted" not in serial:
        return {"run": index, "pass": False, "reason": "AFS2 mount boot failed", "rc": rc}
    for meta in (NEG_C2, NEG_XX, POS_C2, POS_XX):
        gn.seed_bundle(disk, meta["file"], bundles[meta["key"]])
    seeded = gn.tree(disk)
    for meta in (NEG_C2, NEG_XX, POS_C2, POS_XX):
        if seeded.get(source_path(meta)) != bundles[meta["key"]]:
            return {"run": index, "pass": False, "reason": f"seeded {meta['key']} bundle not byte-identical"}
    outcome: dict = {}
    feed = [((b"[desktop] real desktop frame presented", b"filesd: AFS2 mounted"), 1,
             lambda: interaction(disk, bundles, outcome))]
    rc, serial, elapsed = mtest.boot(LABEL, esp, feed, disk, pointer=True, timeout_s=1200)
    log = BUILD_DIR / f"serial-{cc}-run{index}.log"
    log.write_text(serial)
    verdict = judge(serial)
    verdict.update({"run": index, "compiler": cc, "boot_rc": rc, "elapsed_s": round(elapsed, 1),
                    "serial_log": str(log), "driver": outcome})
    if rc != 0:
        verdict["pass"] = False
        verdict["failures"].append(f"boot rc={rc} (clean shutdown required)")
    return verdict


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--cc", default="clang", choices=list(crun.COMPILERS))
    ap.add_argument("--runs", type=int, default=1)
    args = ap.parse_args()
    BUILD_DIR.mkdir(parents=True, exist_ok=True)
    bundles, identity = build_c2_bundles(args.cc)
    (BUILD_DIR / f"bundle-identity-{args.cc}.json").write_text(json.dumps(identity, indent=2))
    results = []
    for i in range(1, args.runs + 1):
        results.append(one_run(args.cc, bundles, identity, i))
        print(json.dumps({k: v for k, v in results[-1].items() if k != "driver"}, indent=2), flush=True)
    passed = sum(1 for r in results if r.get("pass"))
    summary = {
        "source_commit": git_head(),
        "worktree_dirty_paths": git_dirty(),
        "identity": identity,
        "runs": len(results),
        "passed": passed,
        "failed": len(results) - passed,
        "evidence_class": "GUEST",
        "results": results,
    }
    out = BUILD_DIR / f"summary-{args.cc}.json"
    out.write_text(json.dumps(summary, indent=2, default=str))
    print(f"C2 guest multi-app: {passed}/{len(results)} exact PASS (summary: {out})")
    return 0 if passed == len(results) else 1


if __name__ == "__main__":
    sys.exit(main())
