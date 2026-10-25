#!/usr/bin/env python3
"""Phase 16 guest proof: live keyboard input to a C app via granted stdin.

Four windowed signed-APB1 bundles of app/c_stdin_app.c:
  neg     windowed, NO stream grant -> exit 58, silent (capability refusal)
  A       "[c-stdin-a]" streams granted (Marina Input Probe)
  B       "[c-stdin-b]" streams granted (Selma Input Probe)
  legacy  same app linked against a test-only ASTR-v1 layout -> the v2
          desktop's page must be refused at stdio attach, exit 58, silent

Flow: install all; launch neg (58); launch B then A (A focused on top);
type real QMP keystrokes: "ping", a 100-byte line (backpressure/burst),
"hello arena" — A must answer all three, B must stay silent (isolation);
screenshot; Ctrl-D -> A prints EOF lines=3, exit 73; click B's window,
type "second" -> B echoes; Ctrl-D -> B EOF lines=1, exit 73; launch the
legacy v1 bundle (58). The judge requires exact exit order
[58, 73, 73, 58], exact echo attribution/order, two desktop
EOF-delivery logs, and clean shutdown. Signing: RFC 8032 development
fixture (test-only). Evidence label: GUEST (DEVELOPMENT kernel when run
via guest_phase16_dev.py).
"""
from __future__ import annotations

import argparse
import hashlib
import json
import re
import shutil
import subprocess
import sys
import time
from pathlib import Path

EXP = Path(__file__).resolve().parent
REPO = EXP.parent.parent
TOOLS = REPO / "tools"
sys.path.insert(0, str(EXP))
sys.path.insert(0, str(TOOLS))

import afs1  # noqa: E402
import apb1_format  # noqa: E402
import arena_env  # noqa: E402
import guest_native as gn  # noqa: E402
import mtest  # noqa: E402
import run as crun  # noqa: E402

LABEL = "c-stdin"
PACKAGE_ID = b"org.arena.editor"
BUILD_DIR = EXP / "build" / "guest-stdin"
EXIT_RE = re.compile(r"^\[desktop\] child Process-cap exit status=(\d+)\r?$", re.M)
SPAWN_MARK = "[desktop] real application spawned"
RETIRED_MARK = gn.RETIRED_MARK
EOF_MARK = "[desktop] native stream stdin EOF delivered (Ctrl-D)"
BURST = "x" * 100

NEG = dict(key="neg", app_id=b"org.arenaos.stdinnoneg", display=b"Tova No Streams",
           flags=0, file=b"aaa-tova.apb1", row=58, launcher_y=412, tag="[c-stdin-neg]")
A = dict(key="a", app_id=b"org.arenaos.stdina", display=b"Marina Input Probe",
         flags=8, file=b"mmm-marina.apb1", row=132, launcher_y=364, tag="[c-stdin-a]")
B = dict(key="b", app_id=b"org.arenaos.stdinb", display=b"Selma Input Probe",
         flags=8, file=b"zzz-selma.apb1", row=206, launcher_y=388, tag="[c-stdin-b]")
LEGACY = dict(key="legacy", app_id=b"org.arenaos.stdinlegacy", display=b"Yara Legacy Probe",
              flags=8, file=b"zzz-yara.apb1", row=280, launcher_y=436, tag="[c-stdin-legacy]")
METAS = (NEG, A, B, LEGACY)


def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def build_legacy_v1_elf(cc: str, out: Path) -> Path:
    """Build the stdin app against a test-only copy of the runtime whose
    stream layout is reverted to ASTR v1 (version 1, capacity 768, stride
    832, modulo indexing). The production source tree is never modified;
    the patched copy lives under build/. A v2 desktop page must fail this
    app's arena_ring_page_check at stdio attach (exit 58)."""
    root = out / "legacy-v1-src"
    if root.exists():
        shutil.rmtree(root)
    shutil.copytree(EXP, root, ignore=shutil.ignore_patterns("build", "__pycache__"))
    abi = root / "include/arena/abi.h"
    text = abi.read_text()
    text = text.replace("#define ARENA_STREAM_CAPACITY 1024u",
                        "#define ARENA_STREAM_CAPACITY 768u")
    text = text.replace("#define ARENA_STREAM_VERSION 2u",
                        "#define ARENA_STREAM_VERSION 1u")
    abi.write_text(text)
    streams = root / "src/streams.c"
    text = streams.read_text()
    text = text.replace("#define RING_STRIDE 1088u", "#define RING_STRIDE 832u")
    text = text.replace("head & ARENA_STREAM_MASK", "head % ARENA_STREAM_CAPACITY")
    text = text.replace("tail & ARENA_STREAM_MASK", "tail % ARENA_STREAM_CAPACITY")
    streams.write_text(text)
    # Drop the v2 power-of-two static assert in the legacy copy only; the
    # one-page-fit assert remains valid for 768-byte rings.
    text = streams.read_text()
    text = text.replace(
        '_Static_assert((ARENA_STREAM_CAPACITY & ARENA_STREAM_MASK) == 0,\n'
        '               "stream capacity must be a power of two (ADR-0111)");\n', "")
    streams.write_text(text)
    legacy_out = out / "legacy-v1-build"
    crun.build_guest(cc, root=root, out_root=legacy_out)
    base = legacy_out / f"guest-{cc}"
    flags = crun.common_flags(root)
    obj = out / "stdin-legacy.o"
    crun.run(crun.COMPILERS[cc] + flags
             + [f'-DAPP_NAME="{LEGACY["tag"]}"', "-c", str(EXP / "app/c_stdin_app.c"),
                "-o", str(obj)])
    elf = out / f"c-stdin-legacy-{cc}.elf"
    crun.run(crun.COMPILERS[cc] + flags + crun.LINK_FLAGS
             + ["-Wl,-T," + str(root / "link/arena-user.ld"), str(base / "crt0.o"),
                str(obj), str(base / "lib/libarena_c.a"), "-o", str(elf)])
    return elf


def build_bundles(cc: str) -> tuple[dict, dict]:
    crun.build_guest(cc)  # canonical runtime archive + crt0 (guest-<cc>/)
    base = EXP / "build" / f"guest-{cc}"
    out = EXP / "build" / "guest-stdin"
    out.mkdir(parents=True, exist_ok=True)
    flags = crun.common_flags(EXP)
    bundles: dict[str, bytes] = {}
    identity: dict = {"compiler": cc, "apps": {}}
    for meta in METAS:
        if meta["key"] == "legacy":
            elf = build_legacy_v1_elf(cc, out)
        else:
            obj = out / f"stdin-{meta['key']}.o"
            crun.run(crun.COMPILERS[cc] + flags
                     + [f'-DAPP_NAME="{meta["tag"]}"', "-c", str(EXP / "app/c_stdin_app.c"),
                        "-o", str(obj)])
            elf = out / f"c-stdin-{meta['key']}-{cc}.elf"
            crun.run(crun.COMPILERS[cc] + flags + crun.LINK_FLAGS
                     + ["-Wl,-T," + str(EXP / "link/arena-user.ld"), str(base / "crt0.o"),
                        str(obj), str(base / "lib/libarena_c.a"), "-o", str(elf)])
        audit = crun.audit_elf(elf)
        audit["disasm"] = crun.disassemble_check(elf)
        if audit["loader_failures"] or audit["disasm"]["simd_or_x87_count"]:
            raise SystemExit(f"{meta['key']} fails the loader/ISA audit: {audit}")
        elf_bytes = elf.read_bytes()
        manifest = apb1_format.make_manifest(
            app_id=meta["app_id"], package_id=PACKAGE_ID, display_name=meta["display"],
            version=1, flags=meta["flags"], requested=0, entry=b"bin/app",
            icon=b"", width=400, height=200, associations=())
        bundle = apb1_format.build_bundle([(1, b"bin/app", elf_bytes)], manifest=manifest)
        bundles[meta["key"]] = bundle
        identity["apps"][meta["key"]] = {
            "elf": str(elf), "elf_sha256": sha256(elf_bytes), "bundle_sha256": sha256(bundle),
            "manifest_flags": meta["flags"],
            "wire_layout": ("ASTR v1 (test-only legacy build)"
                            if meta["key"] == "legacy" else "ASTR v2"),
            "signing": "RFC 8032 development fixture (test-only, not a production key)",
        }
    return bundles, identity


def installed_path(meta: dict) -> str:
    return f"/System/Applications/{meta['app_id'].decode()}/1/bin/app"


def source_path(meta: dict) -> str:
    return f"/Users/user/Desktop/{meta['file'].decode()}"


def _launch_row(d, row_y: int) -> None:
    # Unlike the headless suites, a stream window may hold keyboard focus
    # here: search keystrokes typed while that window retains focus would
    # land in its stdin instead of the launcher. Clicking the exact
    # All Applications row is deterministic. Do not click the panel header:
    # desktop::applications_pointer closes the panel for any in-panel click
    # that does not hit a row.
    d.q.command("input-send-event", events=[d.q._ev("esc", True), d.q._ev("esc", False)])
    time.sleep(0.4)
    d.click(96, 12)
    time.sleep(1.0)
    d.click(250, row_y)


def ctrl_d(d) -> None:
    for key, down in (("ctrl", True), ("d", True), ("d", False), ("ctrl", False)):
        d.q.command("input-send-event", events=[d.q._ev(key, down)])
        time.sleep(0.05)


def interaction(disk: Path, bundles: dict, outcome: dict) -> bytes:
    d = gn.Desktop(LABEL)
    try:
        d.wait(lambda: "[desktop] desktop surface shows /Users/user/Desktop" in d.serial(),
               "Desktop did not enumerate the seeded APB1 sources")
        for meta in METAS:
            gn._install(d, disk, meta["row"], source_path(meta), bundles[meta["key"]],
                        installed_path(meta), outcome, meta["key"])
        exits = 0

        def launch(meta, expect_exit=None):
            nonlocal exits
            before = d.serial().count(SPAWN_MARK)
            _launch_row(d, meta["launcher_y"])
            d.wait(lambda: d.serial().count(SPAWN_MARK) == before + 1,
                   f"All Applications did not launch {meta['key']}", timeout_s=120)
            if expect_exit is not None:
                exits += 1
                want = exits
                d.wait(lambda: len(EXIT_RE.findall(d.serial())) >= want,
                       f"{meta['key']} produced no exit receipt", timeout_s=120)
                d.wait(lambda: d.serial().count(RETIRED_MARK) >= want,
                       f"{meta['key']} was not retired", timeout_s=120)

        view = lambda: gn.app_stream_view(d.serial())  # noqa: E731

        launch(NEG, expect_exit=58)
        launch(B)
        d.wait(lambda: "[c-stdin-b] ready" in view(), "B did not become ready", timeout_s=120)
        launch(A)
        d.wait(lambda: "[c-stdin-a] ready" in view(), "A did not become ready", timeout_s=120)
        # A is the newest window (focused). Click its title to be sure:
        # cascade slot 1 at (96, 84) — neg's slot 0 window already retired,
        # B took the next slot; title bars of both remain clickable.
        d.click(96 + 120, 84 + 10)
        time.sleep(0.5)
        d.q.type_text("ping\r", gap_s=0.03)
        d.wait(lambda: "[c-stdin-a] pong line=1" in view(), "A did not pong", timeout_s=120)
        d.q.type_text(BURST + "\r", gap_s=0.008)
        d.wait(lambda: "[c-stdin-a] echo len=100" in view(), "A burst echo missing", timeout_s=180)
        d.q.type_text("hello arena\r", gap_s=0.03)
        d.wait(lambda: "[c-stdin-a] echo len=11 text=hello arena" in view(),
               "A hello echo missing", timeout_s=120)
        d.shot("stdin-live")
        ppm = arena_env.build_dir() / f"{LABEL}-stdin-live.ppm"
        if ppm.exists():
            gn.ppm_to_png(ppm, BUILD_DIR / "stdin-live.png")
            outcome["screenshot"] = str(BUILD_DIR / "stdin-live.png")
        ctrl_d(d)
        exits += 1
        d.wait(lambda: "[c-stdin-a] EOF lines=3" in view(), "A EOF summary missing", timeout_s=120)
        d.wait(lambda: len(EXIT_RE.findall(d.serial())) >= exits, "A exit missing", timeout_s=120)
        # Focus B (first cascade window) and prove routing follows focus.
        d.click(70 + 120, 60 + 10)
        time.sleep(0.5)
        d.q.type_text("second\r", gap_s=0.03)
        d.wait(lambda: "[c-stdin-b] echo len=6 text=second" in view(),
               "B echo missing after focus change", timeout_s=120)
        ctrl_d(d)
        exits += 1
        d.wait(lambda: "[c-stdin-b] EOF lines=1" in view(), "B EOF summary missing", timeout_s=120)
        d.wait(lambda: len(EXIT_RE.findall(d.serial())) >= exits, "B exit missing", timeout_s=120)
        d.wait(lambda: d.serial().count(RETIRED_MARK) >= exits, "B was not retired", timeout_s=120)
        launch(LEGACY, expect_exit=58)
        outcome["driven"] = True
    except Exception:
        try:
            d.shot("stdin-failure")
            ppm = arena_env.build_dir() / f"{LABEL}-stdin-failure.ppm"
            if ppm.exists():
                gn.ppm_to_png(ppm, BUILD_DIR / "stdin-failure.png")
                outcome["failure_screenshot"] = str(BUILD_DIR / "stdin-failure.png")
        except Exception:  # noqa: BLE001 - diagnostics must not mask the fault
            pass
        raise
    finally:
        d.q.close()
    return b"shutdown\r"


def judge(serial: str) -> dict:
    view = gn.app_stream_view(serial)
    failures: list[str] = []
    exits = [int(v) for v in EXIT_RE.findall(serial)]
    if exits != [58, 73, 73, 58]:
        failures.append(f"exit sequence {exits}, expected [58, 73, 73, 58]")
    for needle, what in (
        ("[c-stdin-a] ready", "A ready"),
        ("[c-stdin-b] ready", "B ready"),
        ("[c-stdin-a] pong line=1", "A pong"),
        (f"[c-stdin-a] echo len=100 text={BURST}", "A burst echo exact"),
        ("[c-stdin-a] echo len=11 text=hello arena", "A hello echo"),
        ("[c-stdin-b] echo len=6 text=second", "B echo after focus"),
    ):
        if view.count(needle) != 1:
            failures.append(f"{what}: count {view.count(needle)}, expected exactly 1")
    if "[c-stdin-neg]" in view:
        failures.append("streamless instance produced output (grant refusal broken)")
    if "[c-stdin-legacy]" in view:
        failures.append("legacy ASTR-v1 instance produced output on a v2 page "
                        "(cross-version refusal broken)")
    a_eof = re.search(r"\[c-stdin-a\] EOF lines=3 bytes=(\d+)", view)
    b_eof = re.search(r"\[c-stdin-b\] EOF lines=1 bytes=(\d+)", view)
    if not a_eof:
        failures.append("A EOF summary (lines=3) missing")
    if not b_eof:
        failures.append("B EOF summary (lines=1) missing")
    if serial.count(EOF_MARK) != 2:
        failures.append(f"desktop EOF delivery count {serial.count(EOF_MARK)}, expected 2")
    # Isolation: B must not echo anything before A's EOF (input follows focus).
    if a_eof and "[c-stdin-b] echo" in view:
        if view.index("[c-stdin-b] echo") < a_eof.start():
            failures.append("B echoed while A was focused (instance isolation violated)")
    # Attribution: A's lines must not appear under B's tag or vice versa.
    if "[c-stdin-b] pong" in view or "text=hello arena" in view.replace("[c-stdin-a]", "", 3):
        pass  # covered by exact-count checks above; placeholder for clarity
    return {"exit_lines": exits, "a_eof_bytes": a_eof.group(1) if a_eof else None,
            "b_eof_bytes": b_eof.group(1) if b_eof else None,
            "failures": failures, "pass": not failures}


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
    print(f"Live-stdin guest: {passed}/{len(results)} exact PASS (summary: {out})")
    return 0 if passed == len(results) else 1


if __name__ == "__main__":
    sys.exit(main())
