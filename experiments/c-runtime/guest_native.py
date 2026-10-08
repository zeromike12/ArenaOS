#!/usr/bin/env python3
"""C1.2 guest proof: an ordinary signed C application through the real lifecycle.

EXPERIMENTAL. Not part of the production image.

What this does (no privileged route, no special kernel image ID):
  1. Builds the static C application `app/c_native_app.c` with the C1 runtime
     (run.py build_guest), and checks its ELF with the loader rules (run.py audit).
  2. Packages it as a legitimate APB1 bundle: manifest flags 28
     (HEADLESS | STANDARD_STREAMS | NATIVE_SYNC), entry bin/cnative, signed with
     the PUBLIC RFC 8032 development fixture. That fixture is for tests only and
     is never a production key.
  3. Seeds the bundle into the AFS2 Desktop folder of a scratch disk.
  4. Boots ArenaOS in QEMU through tools/mtest.py and drives the real desktop
     with QMP: Desktop APB1 install (protected activation), then All Applications
     search and Enter (registry-backed Process launch with the startup record).
  5. Requires the exact per-group verdicts, the exact final verdict, and the
     desktop's exact `child Process-cap exit status=57` receipt. Anything else
     is a failure, and the process exits nonzero.

Evidence labels: GUEST (this run executed the app under ArenaOS). The result
is recorded with source commit, ELF and bundle SHA-256, and serial log path.
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

import run as crun  # noqa: E402  (C1 build helpers)
import afs1  # noqa: E402
import afs2  # noqa: E402
import apb1_format  # noqa: E402
import arena_env  # noqa: E402
import mtest  # noqa: E402
from test_m10_apps import Desktop, crop  # noqa: E402

LABEL = "c1-native"
BASE = arena_env.AFS2_BASE_SECTOR * 512
APP_ID = b"org.arenaos.cnative"
PACKAGE_ID = b"org.arena.editor"
DISPLAY = b"C Native Probe"
SOURCE_NAME = b"zzz-cnative.apb1"
DESKTOP_SOURCE = f"/Users/user/Desktop/{SOURCE_NAME.decode()}"
INSTALLED_ENTRY = f"/System/Applications/{APP_ID.decode()}/1/bin/cnative"
ICON_Y = 58           # first Desktop grid cell: the only seeded icon (screenshot-verified)
SEARCH = "native"
EXPECTED_EXIT = 57
GROUPS = [
    "T1 startup-abi-v2", "T2 granted-streams", "T3 stdin-bounded-read", "T4 ring-protocol",
    "T5 allocator-hardening", "T6 monotonic-time", "T7 logic", "T8 per-thread-tls",
    "T9 threads-sync",
]
GROUP_RE = re.compile(r"^\[c-native\] (T\d [\w-]+) (PASS|FAIL) checks=(\d+)\r?$", re.M)
FINAL_RE = re.compile(
    r"^\[c-native\] RESULT PASS groups=9/9 checks=(\d+) failed=0\r?$", re.M)
EXIT_RE = re.compile(r"^\[desktop\] child Process-cap exit status=57\r?$", re.M)
# C1.1 negative: the same binary, packaged WITHOUT STANDARD_STREAMS, must see
# no stream grant, fail stdio setup cleanly, and exit with the defined status 58.
NOSTREAM_APP_ID = b"org.arenaos.nostream"          # no "native" substring (search collision)
NOSTREAM_DISPLAY = b"Stream Less Probe"
NOSTREAM_SOURCE_NAME = b"aaa-nostream.apb1"         # sorts first: Desktop row 0
NOSTREAM_DESKTOP_SOURCE = f"/Users/user/Desktop/{NOSTREAM_SOURCE_NAME.decode()}"
NOSTREAM_INSTALLED_ENTRY = f"/System/Applications/{NOSTREAM_APP_ID.decode()}/1/bin/cnative"
NOSTREAM_FLAGS = 20                                  # HEADLESS | NATIVE_SYNC, no STANDARD_STREAMS
NOSTREAM_EXIT = 58
NOSTREAM_SEARCH = "Stream"
NATIVE_ICON_Y = 132                                  # Desktop row 1 (after aaa-nostream)
EXIT58_RE = re.compile(r"^\[desktop\] child Process-cap exit status=58\r?$", re.M)
EXIT57_LINE_RE = re.compile(r"^\[desktop\] child Process-cap exit status=(\d+)\r?$", re.M)
RETIRED_MARK = "[desktop] application retired:"
INSTALL_MARK = "[desktop] APB1 installed; signed version="
HEADLESS_MARK = "[desktop] verified headless application spawned"


def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def git_head() -> str:
    p = subprocess.run(["git", "rev-parse", "HEAD"], cwd=REPO, capture_output=True, text=True)
    return p.stdout.strip() or "unknown"


def git_dirty() -> list[str]:
    """Paths with uncommitted changes; a non-empty list means the receipt is for a dirty tree."""
    p = subprocess.run(["git", "status", "--porcelain"], cwd=REPO, capture_output=True, text=True)
    return [line for line in p.stdout.splitlines() if line.strip()]


def source_hashes() -> dict:
    here = Path(__file__).resolve().parent
    return {name: sha256((here / name).read_bytes()) for name in
            ("guest_native.py", "app/c_native_app.c", "run.py", "link/arena-user.ld")}


def build_native_bundle(cc: str) -> tuple[bytes, bytes, Path, dict]:
    elfs = crun.build_guest(cc)
    elf = elfs["c-native"]
    audit = crun.audit_elf(elf)
    audit["disasm"] = crun.disassemble_check(elf)
    if audit["loader_failures"] or audit["disasm"]["simd_or_x87_count"]:
        raise SystemExit(f"c-native ELF fails the loader/execution audit: {audit}")
    elf_bytes = elf.read_bytes()
    manifest = apb1_format.make_manifest(
        app_id=APP_ID,
        package_id=PACKAGE_ID,
        display_name=DISPLAY,
        version=1,
        flags=28,               # HEADLESS | STANDARD_STREAMS | NATIVE_SYNC
        requested=0,
        entry=b"bin/cnative",
        icon=b"",
        width=0,
        height=0,
        associations=(),
    )
    bundle = apb1_format.build_bundle([(1, b"bin/cnative", elf_bytes)], manifest=manifest)
    parsed = apb1_format.parse_bundle(bundle)
    if [name for _k, name, _s, _d in parsed.files] != [b"bin/cnative"]:
        raise SystemExit("APB1 round-trip changed the payload list")
    identity = {
        "compiler": cc,
        "elf": str(elf),
        "elf_sha256": sha256(elf_bytes),
        "elf_size": len(elf_bytes),
        "bundle_sha256": sha256(bundle),
        "bundle_size": len(bundle),
        "manifest_flags": 28,
        "signing": "RFC 8032 development fixture (test-only, not a production key)",
        "loader_audit": audit,
    }
    nostream_manifest = apb1_format.make_manifest(
        app_id=NOSTREAM_APP_ID,
        package_id=PACKAGE_ID,
        display_name=NOSTREAM_DISPLAY,
        version=1,
        flags=NOSTREAM_FLAGS,
        requested=0,
        entry=b"bin/cnative",
        icon=b"",
        width=0,
        height=0,
        associations=(),
    )
    nostream = apb1_format.build_bundle([(1, b"bin/cnative", elf_bytes)], manifest=nostream_manifest)
    parsed_ns = apb1_format.parse_bundle(nostream)
    if [name for _k, name, _s, _d in parsed_ns.files] != [b"bin/cnative"]:
        raise SystemExit("nostream APB1 round-trip changed the payload list")
    identity["nostream"] = {
        "app_id": NOSTREAM_APP_ID.decode(),
        "manifest_flags": NOSTREAM_FLAGS,
        "bundle_sha256": sha256(nostream),
        "bundle_size": len(nostream),
        "elf_sha256": identity["elf_sha256"],  # same binary, different manifest
    }
    return bundle, nostream, elf, identity


def seed_bundle(disk: Path, name: bytes, bundle: bytes) -> None:
    volume = afs2.Volume(disk.read_bytes()[BASE:])
    desktop = volume.resolve("/Users/user/Desktop")
    source = volume.create(desktop, name, 1)
    volume.write(source, 0, bundle, 1)
    with disk.open("r+b") as f:
        f.seek(BASE)
        f.write(volume.image())


def tree(disk: Path) -> dict:
    for _ in range(100):
        try:
            volume = afs2.Volume(disk.read_bytes()[BASE:])
            afs2.check(volume)
            return afs2.walk(volume)
        except Exception:
            time.sleep(0.1)
    raise AssertionError("AFS2 volume did not become host-readable")


def ppm_to_png(src: Path, dst: Path) -> None:
    """Convert a PPM screendump to PNG (stdlib only) so it can be inspected."""
    import struct
    import zlib
    raw = src.read_bytes()
    m = re.match(rb"P6\s+(\d+)\s+(\d+)\s+255\s", raw)
    w, h = int(m[1]), int(m[2])
    pixels = raw[m.end():]
    rows = b"".join(b"\x00" + pixels[y * w * 3:(y + 1) * w * 3] for y in range(h))

    def chunk(tag: bytes, data: bytes) -> bytes:
        return struct.pack(">I", len(data)) + tag + data + struct.pack(">I", zlib.crc32(tag + data) & 0xFFFFFFFF)

    png = b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 2, 0, 0, 0))
    png += chunk(b"IDAT", zlib.compress(rows, 6)) + chunk(b"IEND", b"")
    dst.write_bytes(png)


def _clear_and_search(d: Desktop, text: str) -> None:
    """Open All Applications, clear any old search text, type `text`, press Enter."""
    d.click(120, 12)
    for _ in range(24):  # backspace-clear any previous query (no-op when empty)
        d.q.command("input-send-event", events=[d.q._ev("backspace", True), d.q._ev("backspace", False)])
    d.q.type_text(text, gap_s=0.025)
    d.q.command("input-send-event", events=[d.q._ev("ret", True), d.q._ev("ret", False)])


def _install(d: Desktop, disk: Path, row_y: int, source_path: str, expected: bytes,
             entry_path: str, outcome: dict, label: str) -> None:
    before = d.serial().count(INSTALL_MARK)
    d.click(52, row_y)
    time.sleep(0.1)
    d.click(52, row_y)
    try:
        d.wait(lambda: d.serial().count(INSTALL_MARK) == before + 1,
               f"Desktop did not complete the protected APB1 install of {label}")
    except AssertionError:
        d.shot(f"desktop-after-install-{label}")
        p2 = arena_env.build_dir() / f"{LABEL}-desktop-after-install-{label}.ppm"
        if p2.exists():
            ppm_to_png(p2, BUILD_DIR / f"desktop-after-install-{label}.png")
        raise
    installed = tree(disk)
    if installed.get(source_path) != expected:
        raise AssertionError(f"AFS2 changed the original {label} APB1 source")
    if installed.get(entry_path) is None:
        raise AssertionError(f"installed {label} ELF payload is missing from /System/Applications")
    outcome[f"{label}_installed_entry_sha256"] = sha256(installed[entry_path])


def interaction(disk: Path, bundle: bytes, nostream: bytes, outcome: dict) -> bytes:
    d = Desktop(LABEL)
    try:
        d.wait(lambda: "[desktop] desktop surface shows /Users/user/Desktop" in d.serial(),
               "Desktop did not enumerate the seeded APB1 sources")
        d.shot("desktop-before-install")
        ppm = arena_env.build_dir() / f"{LABEL}-desktop-before-install.ppm"
        if ppm.exists():
            ppm_to_png(ppm, BUILD_DIR / "desktop-before-install.png")
        # Install both packages through the protected install path.
        _install(d, disk, ICON_Y, NOSTREAM_DESKTOP_SOURCE, nostream,
                 NOSTREAM_INSTALLED_ENTRY, outcome, "nostream")
        _install(d, disk, NATIVE_ICON_Y, DESKTOP_SOURCE, bundle, INSTALLED_ENTRY, outcome, "native")

        # Negative: no stream grant. Must exit 58 through the Process cap.
        launches_before = d.serial().count(HEADLESS_MARK)
        _clear_and_search(d, NOSTREAM_SEARCH)
        d.wait(lambda: d.serial().count(HEADLESS_MARK) == launches_before + 1,
               "All Applications did not launch the stream-less package", timeout_s=120)
        d.wait(lambda: EXIT58_RE.search(d.serial()) is not None,
               "the stream-less application did not exit with status 58 through the Process cap",
               timeout_s=180)
        d.wait(lambda: d.serial().count(RETIRED_MARK) >= 1,
               "the stream-less application was not retired", timeout_s=120)
        outcome["nostream_driven"] = True

        # Positive: the signed C application with its streams granted.
        launches_before = d.serial().count(HEADLESS_MARK)
        _clear_and_search(d, SEARCH)
        d.wait(lambda: d.serial().count(HEADLESS_MARK) == launches_before + 1,
               "All Applications did not launch the native package", timeout_s=120)
        d.wait(lambda: FINAL_RE.search(d.serial()) is not None or "[c-native] RESULT FAIL" in d.serial(),
               "the C application produced no final verdict", timeout_s=240)
        d.wait(lambda: EXIT_RE.search(d.serial()) is not None,
               "the desktop did not observe exit status 57 through the Process cap", timeout_s=180)
        d.wait(lambda: d.serial().count(RETIRED_MARK) >= 2,
               "the native application was not retired after exit", timeout_s=120)
        outcome["driven"] = True
    finally:
        d.q.close()
    # Clean guest shutdown through the console, as the other desktop tests do.
    return b"shutdown\r"


# The serial console multiplexes the desktop's relay of the app's stdout and
# stderr with kernel and desktop log lines. The desktop drains the stream ring
# in chunks, and a ring wrap can split one app line into two chunks, so a
# kernel or desktop log line can land inside it:
#   "[c-native] T2 granted-st[arena INFO  sched] spawn(...)\nreams PASS checks=5"
# That is an ordering artefact of the console, not dropped output. The app-line
# view removes those whole log fragments, then matches the app's own lines.
# A split that cannot be repaired leaves the group missing and fails the run.
LOG_FRAGMENT_RE = re.compile(r"\[(?:arena (?:INFO|WARN|ERROR|DEBUG|TRACE)|desktop)\b[^\n]*\n")


def app_stream_view(serial: str) -> str:
    return LOG_FRAGMENT_RE.sub("", serial)


def judge(serial: str) -> dict:
    """Exact verdicts only. Returns a dict of booleans and the matched lines."""
    view = app_stream_view(serial)
    groups = {name: status for name, status, _count in GROUP_RE.findall(view)}
    final = FINAL_RE.search(view)
    failures = []
    for name in GROUPS:
        matched = [g for g in groups if g.startswith(name.split(" ")[0] + " ")]
        if not matched:
            failures.append(f"missing group {name}")
        elif groups[matched[0]] != "PASS":
            failures.append(f"group {matched[0]} FAIL")
    if "[c-native] RESULT FAIL" in view:
        failures.append("RESULT FAIL line present")
    if "check failed line" in view:
        failures.append("a check failed line was printed")
    if final is None:
        failures.append("exact final verdict line missing")
    exits = [int(v) for v in EXIT57_LINE_RE.findall(serial)]
    if exits.count(57) != 1:
        failures.append(f"exact desktop exit status=57 receipt count is {exits.count(57)}, not 1")
    if exits.count(58) != 1:
        failures.append(f"exact desktop exit status=58 receipt count is {exits.count(58)}, not 1")
    if exits != [58, 57]:
        failures.append(f"exit order is {exits}, expected [58, 57] (negative then positive)")
    if view.count("[c-native] RESULT ") != 1:
        failures.append("RESULT line count is not exactly 1")
    return {
        "groups_pass": sum(1 for s in groups.values() if s == "PASS"),
        "groups_total": len(GROUPS),
        "checks": int(final.group(1)) if final else None,
        "final_line": final.group(0) if final else None,
        "exit_lines": [int(v) for v in EXIT57_LINE_RE.findall(serial)],
        "failures": failures,
        "pass": not failures,
    }


def one_run(cc: str, bundle: bytes, nostream: bytes, identity: dict, index: int) -> dict:
    esp = mtest.build(LABEL, desktop=True)
    disk = arena_env.make_scratch_disk()
    rc, serial, _ = mtest.boot(LABEL + "-seed", esp, [(b"arena>", 1, b"shutdown\r")], disk, pointer=True)
    if rc != 0 or "no AFS2 region" not in serial:
        return {"run": index, "pass": False, "reason": "AFS2 seed boot failed", "rc": rc}
    with disk.open("r+b") as f:
        f.truncate(arena_env.AFS2_DISK_MIB * 1024 * 1024)
    if afs1.audit(disk):
        return {"run": index, "pass": False, "reason": "AFS1 import area damaged before install"}
    rc, serial, _ = mtest.boot(LABEL + "-afs2", esp,
                               [(b"filesd: AFS2 mounted", 1, b"shutdown\r")], disk, pointer=True)
    if rc != 0 or "filesd: AFS2 mounted" not in serial:
        return {"run": index, "pass": False, "reason": "AFS2 mount boot failed", "rc": rc}
    seed_bundle(disk, SOURCE_NAME, bundle)
    seed_bundle(disk, NOSTREAM_SOURCE_NAME, nostream)
    seeded = tree(disk)
    if seeded.get(DESKTOP_SOURCE) != bundle or seeded.get(NOSTREAM_DESKTOP_SOURCE) != nostream:
        return {"run": index, "pass": False, "reason": "seeded bundles are not byte-identical on AFS2"}
    outcome: dict = {}
    feed = [((b"[desktop] real desktop frame presented", b"filesd: AFS2 mounted"), 1,
             lambda: interaction(disk, bundle, nostream, outcome))]
    rc, serial, elapsed = mtest.boot(LABEL, esp, feed, disk, pointer=True, timeout_s=900)
    log = BUILD_DIR / f"serial-{LABEL}-{cc}-run{index}.log"
    log.write_text(serial)
    verdict = judge(serial)
    verdict.update({"run": index, "compiler": cc, "boot_rc": rc, "elapsed_s": round(elapsed, 1),
                    "serial_log": str(log), "driver": outcome})
    if rc != 0:
        verdict["pass"] = False
        verdict["failures"].append(f"boot rc={rc} (clean shutdown required)")
    return verdict


BUILD_DIR = EXP / "build" / "guest-native"


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--cc", default="clang", choices=list(crun.COMPILERS))
    ap.add_argument("--runs", type=int, default=1)
    args = ap.parse_args()
    BUILD_DIR.mkdir(parents=True, exist_ok=True)
    bundle, nostream, elf, identity = build_native_bundle(args.cc)
    (BUILD_DIR / "bundle-identity.json").write_text(json.dumps(identity, indent=2))
    results = []
    for i in range(1, args.runs + 1):
        results.append(one_run(args.cc, bundle, nostream, identity, i))
        print(json.dumps({k: v for k, v in results[-1].items() if k != "driver"}, indent=2), flush=True)
    passed = sum(1 for r in results if r.get("pass"))
    summary = {
        "source_commit": git_head(),
        "worktree_dirty_paths": git_dirty(),
        "source_sha256": source_hashes(),
        "identity": identity,
        "runs": len(results),
        "passed": passed,
        "failed": len(results) - passed,
        "evidence_class": "GUEST",
        "results": results,
    }
    out = BUILD_DIR / f"summary-{identity['compiler']}.json"
    out.write_text(json.dumps(summary, indent=2, default=str))
    print(f"C1.2 guest native app: {passed}/{len(results)} exact PASS (summary: {out})")
    return 0 if passed == len(results) else 1


if __name__ == "__main__":
    sys.exit(main())
