#!/usr/bin/env python3
"""Phase 16 guest Notification-selection positive/negative proof.

The same signed C probe (`app/c_notification_app.c`) is launched twice:

1. Production Desktop (canonical source): one Other/RW Notification, so
   the C runtime must bind that sole candidate and the app exits 75.
2. RED-mutation Desktop fixture: following the established Phase 14
   mutation-test pattern, this script temporarily inserts one distinct
   second Other/RW Notification descriptor+grant into Desktop's headless
   launcher, builds that fixture, and requires the C runtime to select
   NONE and return ARENA_E_NO_WAITER from a blocking empty-stdin read
   (app exit 76). The production Desktop source and exact production
   EFI/ESP/profile artifacts are restored in a finally block.

The mutant is a test fixture, not a production launch policy and not the
canonical artifact. Its marker, source hashes, and classification are
recorded in the summary so the evidence cannot be confused with the
production positive control.
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

import run as crun  # noqa: E402
import afs1  # noqa: E402
import apb1_format  # noqa: E402
import arena_env  # noqa: E402
import guest_native as gn  # noqa: E402
import mtest  # noqa: E402
from test_m10_apps import Desktop  # noqa: E402

LABEL = "c-notify"
BUILD_DIR = EXP / "build" / "guest-notification"
APP_ID = b"org.arenaos.cnotify"
PACKAGE_ID = b"org.arena.editor"
DISPLAY = b"C Notify Probe"
SOURCE_NAME = b"zzz-cnotify.apb1"
DESKTOP_SOURCE = f"/Users/user/Desktop/{SOURCE_NAME.decode()}"
INSTALLED_ENTRY = f"/System/Applications/{APP_ID.decode()}/1/bin/app"
FLAGS = 12  # HEADLESS | STANDARD_STREAMS (no NATIVE_SYNC: descriptor slot 4 is free)
SEARCH = "Notify"
ICON_Y = 58
EXIT_SOLE = 75
EXIT_AMBIGUOUS = 76
MUTANT_MARK = "[desktop] TEST mutant ambiguous notification record prepared"
DESKTOP_RS = REPO / "userspace/desktop/src/bin/desktop.rs"
EFI = REPO / "build/arena-boot.efi"
ESP = REPO / "build/arena-esp.img"
PROFILE = REPO / "build/graphics-profile.txt"

SOLE_RE = re.compile(
    r"^\[c-notify\] RESULT PASS mode=sole candidates=1 selected_index=0 "
    r"selected_slot=1 peek=-1005\r?$", re.M)
AMBIGUOUS_RE = re.compile(
    r"^\[c-notify\] RESULT PASS mode=ambiguous candidates=2 selected=none "
    r"slots=1,4 peek=-1005 blocking=-1012\r?$", re.M)
EXIT_LINE_RE = re.compile(r"^\[desktop\] child Process-cap exit status=(\d+)\r?$", re.M)

DESCRIPTOR_NEEDLE = """    if wants_sync {
        descriptors[descriptor_count] = startup_role_descriptor(
            (descriptor_count + 1) as u16,
            CapabilityRole::SyncDomain,
            startup_abi::CAP_KIND_SYNC_DOMAIN,
            RIGHTS_READ | RIGHTS_WRITE,
        );
        descriptor_count += 1;
    }
    let spec = startup_abi::StartupSpec {
"""
DESCRIPTOR_MUTANT = """    if wants_sync {
        descriptors[descriptor_count] = startup_role_descriptor(
            (descriptor_count + 1) as u16,
            CapabilityRole::SyncDomain,
            startup_abi::CAP_KIND_SYNC_DOMAIN,
            RIGHTS_READ | RIGHTS_WRITE,
        );
        descriptor_count += 1;
    }
    // TEST MUTANT (Phase 16 notification negative): deliberately publish a
    // second indistinguishable Other/RW Notification descriptor. Production
    // Desktop never does this; guest_notification.py applies and restores
    // this exact fixture following the Phase 14 RED-mutation pattern.
    let test_ambiguous_notification = wants_streams && !wants_sync;
    if test_ambiguous_notification {
        descriptors[descriptor_count] = startup_cap_descriptor(
            (descriptor_count + 1) as u16,
            startup_abi::CAP_KIND_NOTIFICATION,
            RIGHTS_READ | RIGHTS_WRITE,
        );
        descriptor_count += 1;
        log(b"[desktop] TEST mutant ambiguous notification record prepared\\n");
    }
    let spec = startup_abi::StartupSpec {
"""

GRANT_NEEDLE = """    let sync_slot = if wants_sync {
        let Some(slot) = free_cap_slot() else {
            destroy(startup_cap);
            return Err(STATUS_BUSY);
        };
        let create_sync = unsafe {
            syscall6(
                SYS_SYNC_DOMAIN_CREATE,
                SYNC_DOMAIN_FACTORY_SLOT,
                slot,
                0,
                0,
                0,
                0,
            )
        };
        if create_sync != 0 {
            destroy(startup_cap);
            return Err(create_sync);
        }
        grants[grant_count] = InheritGrant::new(slot as u8, (RIGHTS_READ | RIGHTS_WRITE) as u32);
        grant_count += 1;
        Some(slot)
    } else {
        None
    };
    let process = spawn_child(i, image, &grants[..grant_count]);
"""
GRANT_MUTANT = """    let sync_slot = if wants_sync {
        let Some(slot) = free_cap_slot() else {
            destroy(startup_cap);
            return Err(STATUS_BUSY);
        };
        let create_sync = unsafe {
            syscall6(
                SYS_SYNC_DOMAIN_CREATE,
                SYNC_DOMAIN_FACTORY_SLOT,
                slot,
                0,
                0,
                0,
                0,
            )
        };
        if create_sync != 0 {
            destroy(startup_cap);
            return Err(create_sync);
        }
        grants[grant_count] = InheritGrant::new(slot as u8, (RIGHTS_READ | RIGHTS_WRITE) as u32);
        grant_count += 1;
        Some(slot)
    } else {
        None
    };
    // TEST MUTANT: create the distinct Notification object named by the
    // extra descriptor above and grant exactly READ|WRITE to the child.
    let test_ambiguous_slot = if test_ambiguous_notification {
        let Some(slot) = free_cap_slot() else {
            destroy(startup_cap);
            if let Some(slot) = sync_slot {
                destroy(slot);
            }
            return Err(STATUS_BUSY);
        };
        let create_notification = unsafe {
            syscall6(
                SYS_NOTIFICATION_CREATE,
                NOTIFICATION_FACTORY_SLOT,
                slot,
                0,
                0,
                0,
                0,
            )
        };
        if create_notification != 0 {
            destroy(startup_cap);
            if let Some(slot) = sync_slot {
                destroy(slot);
            }
            return Err(create_notification);
        }
        grants[grant_count] =
            InheritGrant::new(slot as u8, (RIGHTS_READ | RIGHTS_WRITE) as u32);
        grant_count += 1;
        Some(slot)
    } else {
        None
    };
    let process = spawn_child(i, image, &grants[..grant_count]);
"""

DESTROY_NEEDLE = """    let process = spawn_child(i, image, &grants[..grant_count]);
    destroy(startup_cap);
    let process = match process {
"""
DESTROY_MUTANT = """    let process = spawn_child(i, image, &grants[..grant_count]);
    destroy(startup_cap);
    if let Some(slot) = test_ambiguous_slot {
        destroy(slot);
    }
    let process = match process {
"""


def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def git_out(*args: str) -> str:
    return subprocess.run(["git", "-C", str(REPO), *args],
                          capture_output=True, text=True).stdout.strip()


def build_bundle(cc: str) -> tuple[bytes, dict]:
    crun.build_guest(cc)  # canonical runtime archive + crt0
    base = EXP / "build" / f"guest-{cc}"
    BUILD_DIR.mkdir(parents=True, exist_ok=True)
    flags = crun.common_flags(EXP)
    obj = BUILD_DIR / f"c-notification-{cc}.o"
    elf = BUILD_DIR / f"c-notification-{cc}.elf"
    crun.run(crun.COMPILERS[cc] + flags +
             ["-c", str(EXP / "app/c_notification_app.c"), "-o", str(obj)])
    crun.run(crun.COMPILERS[cc] + flags + crun.LINK_FLAGS +
             ["-Wl,-T," + str(EXP / "link/arena-user.ld"), str(base / "crt0.o"),
              str(obj), str(base / "lib/libarena_c.a"), "-o", str(elf)])
    audit = crun.audit_elf(elf)
    audit["disasm"] = crun.disassemble_check(elf)
    if audit["loader_failures"] or audit["disasm"]["simd_or_x87_count"]:
        raise SystemExit(f"c-notification ELF fails loader/ISA audit: {audit}")
    elf_bytes = elf.read_bytes()
    manifest = apb1_format.make_manifest(
        app_id=APP_ID, package_id=PACKAGE_ID, display_name=DISPLAY,
        version=1, flags=FLAGS, requested=0, entry=b"bin/app",
        icon=b"", width=0, height=0, associations=())
    bundle = apb1_format.build_bundle([(1, b"bin/app", elf_bytes)], manifest=manifest)
    parsed = apb1_format.parse_bundle(bundle)
    if [name for _k, name, _s, _d in parsed.files] != [b"bin/app"]:
        raise SystemExit("APB1 round-trip changed the payload list")
    identity = {
        "compiler": cc,
        "elf": str(elf),
        "elf_sha256": sha256(elf_bytes),
        "elf_size": len(elf_bytes),
        "bundle_sha256": sha256(bundle),
        "bundle_size": len(bundle),
        "manifest_flags": FLAGS,
        "signing": "RFC 8032 development fixture (test-only, not a production key)",
        "loader_audit": audit,
    }
    return bundle, identity


def interaction(label: str, disk: Path, bundle: bytes, outcome: dict) -> bytes:
    # guest_native's install helper uses its module globals for labels and
    # screenshot destinations; point them at this suite and boot label (the
    # QMP socket is named after the mtest boot label).
    gn.LABEL = label
    gn.BUILD_DIR = BUILD_DIR
    d = Desktop(label)
    try:
        d.wait(lambda: "[desktop] desktop surface shows /Users/user/Desktop" in d.serial(),
               "Desktop did not enumerate the seeded APB1 source")
        gn._install(d, disk, ICON_Y, DESKTOP_SOURCE, bundle, INSTALLED_ENTRY,
                    outcome, "notify")
        launches_before = d.serial().count(gn.HEADLESS_MARK)
        gn._clear_and_search(d, SEARCH)
        d.wait(lambda: d.serial().count(gn.HEADLESS_MARK) == launches_before + 1,
               "All Applications did not launch the notification probe", timeout_s=120)
        d.wait(lambda: "[c-notify] RESULT " in gn.app_stream_view(d.serial()),
               "notification probe produced no verdict", timeout_s=180)
        d.wait(lambda: len(EXIT_LINE_RE.findall(d.serial())) >= 1,
               "Desktop produced no Process-cap exit receipt", timeout_s=120)
        d.wait(lambda: d.serial().count(gn.RETIRED_MARK) >= 1,
               "notification probe was not retired", timeout_s=120)
        outcome["driven"] = True
    finally:
        d.q.close()
    return b"shutdown\r"


def judge(serial: str, mode: str) -> dict:
    view = gn.app_stream_view(serial)
    exits = [int(v) for v in EXIT_LINE_RE.findall(serial)]
    failures: list[str] = []
    if mode == "production":
        if SOLE_RE.search(view) is None:
            failures.append("exact sole-candidate PASS line missing")
        if exits != [EXIT_SOLE]:
            failures.append(f"exit sequence {exits}, expected [{EXIT_SOLE}]")
        if MUTANT_MARK in serial:
            failures.append("mutant marker present in production run")
    elif mode == "mutant":
        if AMBIGUOUS_RE.search(view) is None:
            failures.append("exact ambiguous-candidate PASS line missing")
        if exits != [EXIT_AMBIGUOUS]:
            failures.append(f"exit sequence {exits}, expected [{EXIT_AMBIGUOUS}]")
        if serial.count(MUTANT_MARK) != 1:
            failures.append(f"mutant marker count {serial.count(MUTANT_MARK)}, expected 1")
    else:
        failures.append(f"unknown mode {mode!r}")
    if "[c-notify] RESULT FAIL" in view:
        failures.append("RESULT FAIL line present")
    if view.count("[c-notify] RESULT ") != 1:
        failures.append("RESULT line count is not exactly 1")
    return {
        "mode": mode,
        "exit_lines": exits,
        "mutant_marker_count": serial.count(MUTANT_MARK),
        "failures": failures,
        "pass": not failures,
    }


def one_run(cc: str, bundle: bytes, mode: str, index: int) -> dict:
    label = f"{LABEL}-{mode}"
    esp = mtest.build(label, desktop=True)
    disk = arena_env.make_scratch_disk()
    rc, serial, _ = mtest.boot(label + "-seed", esp,
                               [(b"arena>", 1, b"shutdown\r")], disk, pointer=True)
    if rc != 0 or "no AFS2 region" not in serial:
        return {"run": index, "mode": mode, "compiler": cc, "pass": False,
                "reason": "AFS2 seed boot failed", "boot_rc": rc, "failures": ["seed boot"]}
    with disk.open("r+b") as f:
        f.truncate(arena_env.AFS2_DISK_MIB * 1024 * 1024)
    if afs1.audit(disk):
        return {"run": index, "mode": mode, "compiler": cc, "pass": False,
                "reason": "AFS1 import area damaged", "failures": ["AFS1 audit"]}
    rc, serial, _ = mtest.boot(label + "-afs2", esp,
                               [(b"filesd: AFS2 mounted", 1, b"shutdown\r")],
                               disk, pointer=True)
    if rc != 0 or "filesd: AFS2 mounted" not in serial:
        return {"run": index, "mode": mode, "compiler": cc, "pass": False,
                "reason": "AFS2 mount boot failed", "boot_rc": rc, "failures": ["mount boot"]}
    gn.seed_bundle(disk, SOURCE_NAME, bundle)
    seeded = gn.tree(disk)
    if seeded.get(DESKTOP_SOURCE) != bundle:
        return {"run": index, "mode": mode, "compiler": cc, "pass": False,
                "reason": "seeded bundle is not byte-identical", "failures": ["seed bytes"]}
    outcome: dict = {}
    feed = [((b"[desktop] real desktop frame presented", b"filesd: AFS2 mounted"), 1,
             lambda: interaction(label, disk, bundle, outcome))]
    rc, serial, elapsed = mtest.boot(label, esp, feed, disk, pointer=True, timeout_s=900)
    log = BUILD_DIR / f"serial-{label}-{cc}-run{index}.log"
    log.write_text(serial)
    verdict = judge(serial, mode)
    verdict.update({
        "run": index,
        "compiler": cc,
        "boot_rc": rc,
        "elapsed_s": round(elapsed, 1),
        "serial_log": str(log),
        "driver": outcome,
        "image_esp_sha256": sha256(esp.read_bytes()),
        "image_efi_sha256": sha256(EFI.read_bytes()) if EFI.exists() else None,
    })
    if rc != 0:
        verdict["pass"] = False
        verdict["failures"].append(f"boot rc={rc} (clean shutdown required)")
    return verdict


def mutate_desktop(original: bytes) -> bytes:
    text = original.decode()
    start = text.index("fn launch_headless_image_v2(")
    end = text.index("\nfn file_grants", start)
    segment = text[start:end]
    for needle, mutant in (
        (DESCRIPTOR_NEEDLE, DESCRIPTOR_MUTANT),
        (GRANT_NEEDLE, GRANT_MUTANT),
        (DESTROY_NEEDLE, DESTROY_MUTANT),
    ):
        if segment.count(needle) != 1:
            raise AssertionError(
                f"mutation needle count {segment.count(needle)}, expected 1")
        segment = segment.replace(needle, mutant)
    return (text[:start] + segment + text[end:]).encode()


def self_test() -> int:
    sole = ("[desktop] verified headless application spawned\n"
            "[c-notify] RESULT PASS mode=sole candidates=1 selected_index=0 "
            "selected_slot=1 peek=-1005\n"
            "[desktop] child Process-cap exit status=75\n")
    ambiguous = (MUTANT_MARK + "\n"
                 "[c-notify] RESULT PASS mode=ambiguous candidates=2 selected=none "
                 "slots=1,4 peek=-1005 blocking=-1012\n"
                 "[desktop] child Process-cap exit status=76\n")
    first_pick_regression = ambiguous.replace(
        "mode=ambiguous candidates=2 selected=none slots=1,4 peek=-1005 blocking=-1012",
        "mode=sole candidates=1 selected_index=0 selected_slot=1 peek=-1005")
    cases = [
        ("production exact", judge(sole, "production")["pass"], True),
        ("mutant exact", judge(ambiguous, "mutant")["pass"], True),
        ("mutant marker absent", judge(ambiguous.replace(MUTANT_MARK, ""), "mutant")["pass"], False),
        ("wrong exit", judge(ambiguous.replace("status=76", "status=75"), "mutant")["pass"], False),
        ("first-candidate regression", judge(first_pick_regression, "mutant")["pass"], False),
        ("production sees mutant", judge(sole + MUTANT_MARK + "\n", "production")["pass"], False),
    ]
    failed = [name for name, got, want in cases if got != want]
    for name, got, want in cases:
        print(f"self-test {name}: {'PASS' if got == want else 'FAIL'}")
    print(f"guest_notification judge self-test: {len(cases) - len(failed)}/{len(cases)} PASS")
    return 1 if failed else 0


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--cc", default="gcc", choices=list(crun.COMPILERS))
    ap.add_argument("--self-test", action="store_true")
    args = ap.parse_args()
    if args.self_test:
        return self_test()

    BUILD_DIR.mkdir(parents=True, exist_ok=True)
    bundle, identity = build_bundle(args.cc)
    (BUILD_DIR / "bundle-identity.json").write_text(json.dumps(identity, indent=2))

    original_source = DESKTOP_RS.read_bytes()
    mutant_source = mutate_desktop(original_source)
    if git_out("status", "--porcelain", "--", str(DESKTOP_RS.relative_to(REPO))):
        raise SystemExit("Desktop source is already dirty; refusing to apply mutation")

    results: list[dict] = []
    results.append(one_run(args.cc, bundle, "production", 1))
    print(json.dumps({k: v for k, v in results[-1].items() if k != "driver"}, indent=2),
          flush=True)

    artifacts = {path: path.read_bytes() for path in (EFI, ESP, PROFILE)}
    artifact_hashes = {str(path): sha256(blob) for path, blob in artifacts.items()}
    try:
        DESKTOP_RS.write_bytes(mutant_source)
        results.append(one_run(args.cc, bundle, "mutant", 2))
        print(json.dumps({k: v for k, v in results[-1].items() if k != "driver"}, indent=2),
              flush=True)
    finally:
        DESKTOP_RS.write_bytes(original_source)
        for path, blob in artifacts.items():
            path.write_bytes(blob)
        assert DESKTOP_RS.read_bytes() == original_source
        assert all(path.read_bytes() == blob for path, blob in artifacts.items())
        assert all(sha256(path.read_bytes()) == artifact_hashes[str(path)]
                   for path in artifacts)

    passed = sum(1 for r in results if r.get("pass"))
    summary = {
        "source_commit": git_out("rev-parse", "HEAD"),
        "worktree_dirty_paths": [ln for ln in git_out("status", "--porcelain").splitlines()
                                 if ln.strip()],
        "identity": identity,
        "mutation_fixture": {
            "classification": "RED-mutation Desktop fixture; NOT the canonical artifact",
            "desktop_source": str(DESKTOP_RS),
            "production_source_sha256": sha256(original_source),
            "mutant_source_sha256": sha256(mutant_source),
            "restored": DESKTOP_RS.read_bytes() == original_source,
            "production_artifact_sha256": artifact_hashes,
        },
        "runs": len(results),
        "passed": passed,
        "failed": len(results) - passed,
        "evidence_class": "GUEST (production positive + restored mutation negative)",
        "results": results,
    }
    out = BUILD_DIR / f"summary-{args.cc}.json"
    out.write_text(json.dumps(summary, indent=2, default=str))
    print(f"Phase 16 notification guest: {passed}/{len(results)} exact PASS (summary: {out})")
    return 0 if passed == len(results) else 1


if __name__ == "__main__":
    sys.exit(main())
