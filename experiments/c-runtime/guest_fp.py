#!/usr/bin/env python3
"""Phase 16 guest proof: FP/SSE2 integration app through signed APB1 launch.

Three bundles of app/c_fp_app.c (built by build_fp_app.py), installed and
launched exactly like guest_c2.py (protected APB1 path, All Applications):

  1. fp-nostream  flags without streams  -> exit 58, no output (negative)
  2. fp-positive  streams + sync granted -> "[c-fp] VERDICT PASS", exit 71
  3. fp-mutant    same grants, probe corrupted (-DFP_PROBE_MUTATE) ->
     "[c-fp] VERDICT FAIL", exit 72 (guest negative control: corrupted FP
     state MUST be detected; a mutant PASS fails this suite)

Signing: RFC 8032 development fixture (test-only). Evidence label: GUEST.
Run via guest_phase16_dev.py --suite fp while B1 blocks official builds
(results are DEVELOPMENT evidence until re-run on an official build).
"""
from __future__ import annotations

import argparse
import hashlib
import json
import re
import subprocess
import sys
from pathlib import Path

EXP = Path(__file__).resolve().parent
REPO = EXP.parent.parent
TOOLS = REPO / "tools"
sys.path.insert(0, str(EXP))
sys.path.insert(0, str(TOOLS))

import afs1  # noqa: E402
import apb1_format  # noqa: E402
import arena_env  # noqa: E402
import build_fp_app as fpbuild  # noqa: E402
import guest_native as gn  # noqa: E402
import mtest  # noqa: E402
import run as crun  # noqa: E402

LABEL = "c-fp"
PACKAGE_ID = b"org.arena.editor"
BUILD_DIR = EXP / "build" / "guest-fp"
EXIT_RE = re.compile(r"^\[desktop\] child Process-cap exit status=(\d+)\r?$", re.M)
RESULT_RE = re.compile(r"^\[c-fp\] VERDICT (PASS|FAIL) checks=(\d+) failed=(\d+) skips=(\d+)\r?$", re.M)
HEADLESS_MARK = gn.HEADLESS_MARK
RETIRED_MARK = gn.RETIRED_MARK

NEG = dict(key="fp-nostream", app_id=b"org.arenaos.fpnostream", display=b"Vega Stream Less",
           search="Vega", flags=20, file=b"aaa-vega.apb1", row=58)
POS = dict(key="fp", app_id=b"org.arenaos.fpprobe", display=b"Lyra Float Probe",
           search="Lyra", flags=28, file=b"mmm-lyra.apb1", row=132)
MUT = dict(key="fp-mutant", app_id=b"org.arenaos.fpmutant", display=b"Rigel Mutant Probe",
           search="Rigel", flags=28, file=b"zzz-rigel.apb1", row=206)
METAS = (NEG, POS, MUT)


def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def build_bundles(cc: str) -> tuple[dict, dict]:
    elf_pos, app_o = fpbuild.build(cc, f"fp-{cc}")
    fpbuild.audit(elf_pos, app_o)
    elf_mut, _ = fpbuild.build(cc, f"fp-mutant-{cc}", ["-DFP_PROBE_MUTATE"])
    identity: dict = {"compiler": cc, "apps": {}}
    bundles: dict[str, bytes] = {}
    for meta, elf in ((POS, elf_pos), (NEG, elf_pos), (MUT, elf_mut)):
        audit = crun.audit_elf(elf)
        if audit["loader_failures"]:
            raise SystemExit(f"{meta['key']} fails the loader audit: {audit['loader_failures']}")
        nm = subprocess.run(["nm", str(elf)], capture_output=True, text=True).stdout
        for line in nm.splitlines():
            parts = line.split()
            if len(parts) >= 2 and parts[-1] in ("arena_legacy_write", "arena_legacy_serial_enable"):
                if parts[-2] not in ("w", "W"):
                    raise SystemExit(f"{meta['key']} links the legacy debug-serial path: {line}")
        elf_bytes = elf.read_bytes()
        manifest = apb1_format.make_manifest(
            app_id=meta["app_id"], package_id=PACKAGE_ID, display_name=meta["display"],
            version=1, flags=meta["flags"], requested=0, entry=b"bin/app",
            icon=b"", width=0, height=0, associations=())
        bundle = apb1_format.build_bundle([(1, b"bin/app", elf_bytes)], manifest=manifest)
        bundles[meta["key"]] = bundle
        identity["apps"][meta["key"]] = {
            "elf": str(elf), "elf_sha256": sha256(elf_bytes), "bundle_sha256": sha256(bundle),
            "manifest_flags": meta["flags"],
            "signing": "RFC 8032 development fixture (test-only, not a production key)",
        }
    return bundles, identity


def installed_path(meta: dict) -> str:
    return f"/System/Applications/{meta['app_id'].decode()}/1/bin/app"


def source_path(meta: dict) -> str:
    return f"/Users/user/Desktop/{meta['file'].decode()}"


def _search(d, text: str) -> None:
    import time
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
        for meta in METAS:
            gn._install(d, disk, meta["row"], source_path(meta), bundles[meta["key"]],
                        installed_path(meta), outcome, meta["key"])
        exits_seen = 0
        for meta in METAS:
            before = d.serial().count(HEADLESS_MARK)
            _search(d, meta["search"])
            d.wait(lambda: d.serial().count(HEADLESS_MARK) == before + 1,
                   f"All Applications did not launch {meta['key']}", timeout_s=120)
            exits_seen += 1
            want = exits_seen
            d.wait(lambda: len(EXIT_RE.findall(d.serial())) >= want,
                   f"{meta['key']} produced no exit receipt", timeout_s=300)
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
    if exits != [58, 71, 72]:
        failures.append(f"exit sequence {exits}, expected [58, 71, 72]")
    results = [(m.group(1), int(m.group(2)), int(m.group(3)), int(m.group(4)))
               for m in RESULT_RE.finditer(view)]
    if len(results) != 2:
        failures.append(f"RESULT line count {len(results)}, expected 2 (positive + mutant)")
    else:
        (v1, _c1, f1, _s1), (v2, _c2, f2, _s2) = results
        if v1 != "PASS" or f1 != 0:
            failures.append(f"positive app did not PASS cleanly: {results[0]}")
        if v2 != "FAIL" or f2 == 0:
            failures.append(f"mutant was not detected as FAIL: {results[1]}")
    if view.count("[c-fp] start") != 2:
        failures.append("start line count != 2 (streamless run must be silent)")
    # The mutant legitimately prints check-failed lines; the positive run's
    # failed=0 in its VERDICT line is the exact guard for the positive app.
    return {"exit_lines": exits, "fp_results": results, "failures": failures,
            "pass": not failures}


def one_run(cc: str, bundles: dict, index: int) -> dict:
    esp = mtest.build(LABEL, desktop=True)
    disk = arena_env.make_scratch_disk()
    rc, serial, _ = mtest.boot(LABEL + "-seed", esp, [(b"arena>", 1, b"shutdown\r")], disk, pointer=True)
    if rc != 0 or "no AFS2 region" not in serial:
        return {"run": index, "pass": False, "reason": "AFS2 seed boot failed", "rc": rc}
    with disk.open("r+b") as f:
        f.truncate(arena_env.AFS2_DISK_MIB * 1024 * 1024)
    if afs1.audit(disk):
        return {"run": index, "pass": False, "reason": "AFS1 import area damaged"}
    rc, serial, _ = mtest.boot(LABEL + "-afs2", esp, [(b"filesd: AFS2 mounted", 1, b"shutdown\r")],
                               disk, pointer=True)
    if rc != 0 or "filesd: AFS2 mounted" not in serial:
        return {"run": index, "pass": False, "reason": "AFS2 mount boot failed", "rc": rc}
    for meta in METAS:
        gn.seed_bundle(disk, meta["file"], bundles[meta["key"]])
    seeded = gn.tree(disk)
    for meta in METAS:
        if seeded.get(source_path(meta)) != bundles[meta["key"]]:
            return {"run": index, "pass": False, "reason": f"seeded {meta['key']} not byte-identical"}
    outcome: dict = {}
    feed = [((b"[desktop] real desktop frame presented", b"filesd: AFS2 mounted"), 1,
             lambda: interaction(disk, bundles, outcome))]
    rc, serial, elapsed = mtest.boot(LABEL, esp, feed, disk, pointer=True, timeout_s=1200)
    log = BUILD_DIR / f"serial-{cc}-run{index}.log"
    log.write_text(serial)
    verdict = judge(serial)
    verdict.update({"run": index, "compiler": cc, "boot_rc": rc,
                    "elapsed_s": round(elapsed, 1), "serial_log": str(log), "driver": outcome})
    if rc != 0:
        verdict["pass"] = False
        verdict["failures"].append(f"boot rc={rc} (clean shutdown required)")
    return verdict


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--cc", default="gcc", choices=["gcc", "clang"])
    ap.add_argument("--runs", type=int, default=1)
    args = ap.parse_args()
    BUILD_DIR.mkdir(parents=True, exist_ok=True)
    bundles, identity = build_bundles(args.cc)
    results = [one_run(args.cc, bundles, i) for i in range(1, args.runs + 1)]
    for r in results:
        print(json.dumps({k: v for k, v in r.items() if k != "driver"}, indent=2), flush=True)
    passed = sum(1 for r in results if r.get("pass"))
    summary = {"identity": identity, "runs": len(results), "passed": passed,
               "evidence_class": "GUEST", "results": results}
    out = BUILD_DIR / f"summary-{args.cc}.json"
    out.write_text(json.dumps(summary, indent=2, default=str))
    print(f"FP guest: {passed}/{len(results)} exact PASS (summary: {out})")
    return 0 if passed == len(results) else 1


if __name__ == "__main__":
    sys.exit(main())
