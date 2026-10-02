#!/usr/bin/env python3
"""Package a qualified Phase 8 commit with its OWN bootable QEMU build.

Run after tools/run_tests.sh and tools/stability_loop.sh 100, before the
source+artifact checkpoint commit. This is not a GitHub release;
qualification is bound to its exact EFI. Reject an unrelated EFI, an ESP
with different bytes, or a partial/stale receipt. Extract
and boot the actual tarball from a formatted bundled disk before exit.
"""
import argparse
import hashlib
import os
import re
import shutil
import subprocess
import sys
import tarfile
import tempfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import afs1  # noqa: E402
import arena_env  # noqa: E402
import mtest  # noqa: E402
from pyfatfs.PyFatFS import PyFatFS  # noqa: E402

ROOT = arena_env.REPO_ROOT
FILES = ("arena-boot.efi", "stability-receipt.txt", "arena-esp.img",
         "scratch-template.img", "edk2-x86_64-code.fd",
         "ovmf-vars-template.img", "RUNNING.md", "QUALIFICATION.txt")
# ADR-0059: an archive with the network-attached M7 guest cannot omit its
# actual host peer. ADR-0060: extracted QMP/input proof cannot import from
# this checkout. All files below are independently hashed and extracted.
PHASE9_EXTRA = ("phase9_archive_boot.py", "check_phase9_pixels.py", "qmp.py",
                "network_fixture.py", "tcp_fixture.py", "udp_dns_fixture.py")


def digest(blob: bytes) -> str:
    return hashlib.sha256(blob).hexdigest()


def verify_qualified_image(suite_log: Path) -> tuple[str, bytes]:
    efi = (ROOT / "build/arena-boot.efi").read_bytes()
    esp = ROOT / "build/arena-esp.img"
    receipt = (ROOT / "build/stability-receipt.txt").read_text().strip().split()
    if receipt != [digest(efi), "100/100"]:
        raise ValueError("qualified 100/100 receipt does not match THIS final EFI")
    fs = PyFatFS(str(esp), read_only=True)
    try:
        inside = fs.getbytes("/EFI/BOOT/BOOTX64.EFI")
    finally:
        fs.close()
    if inside != efi:
        raise ValueError("ESP contains an EFI different from the qualified final EFI")
    log = suite_log.read_text()
    suite = re.search(r"ALL TESTS PASSED \((\d+) test suites\)", log)
    if not suite or int(suite.group(1)) < 27 or \
            "INITIAL-START SUBSTRATE: PASS" not in log or \
            "PRODUCTION ORDERLY-RESTART SUBSTRATE: PASS" not in log or \
            "REPEATED-ACCOUNTING SUBSTRATE: PASS" not in log or \
            "UNEXPECTED-CRASH SUBSTRATE: PASS" not in log or \
            "FORCED-LIVE-STOP SUBSTRATE: PASS" not in log or \
            "LIFECYCLE-AUTHORITY REFUSAL SUBSTRATE: PASS" not in log or \
            "ACTIVE DEPENDENCY PROBES: PASS" not in log or \
            "SERVICE-SIDE DIAGNOSTIC AUTHORITY: PASS" not in log:
        raise ValueError("all 27+ historical suites + service-side diagnostic authority required")
    return digest(efi), esp.read_bytes()


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("checkpoint", help="stable checkpoint name, e.g. phase8-initial-stack")
    parser.add_argument("suite_log", type=Path, help="output of full tools/run_tests.sh")
    args = parser.parse_args()
    if not re.fullmatch(r"[a-z0-9][a-z0-9-]*", args.checkpoint):
        parser.error("checkpoint name must contain lowercase letters/digits/hyphens")
    efi_sha, esp_bytes = verify_qualified_image(args.suite_log)
    if args.checkpoint.startswith("phase81-"):
        log = args.suite_log.read_text()
        total = 31 if args.checkpoint in ("phase81-transactional-core", "phase81-complete") else 29
        if f"ALL TESTS PASSED ({total} test suites)" not in log or "CONFIG-READ-BOUNDARY: PASS" not in log:
            raise ValueError(f"8.1 checkpoint requires all {total} historical suites and the guest read proof")
        if args.checkpoint in ("phase81-transactional-core", "phase81-complete") and (
            "AUTHORIZED TRANSACTION/CRASH: PASS" not in log or
            "EIGHT-SLOT EXHAUSTION: PASS" not in log):
            raise ValueError("transactional checkpoint requires updater, disk crash and bounded-table proofs")
        if args.checkpoint == "phase81-complete":
            metric = r"\(\d+, \d+, \d+\)"
            baseline = re.search(
                rf"Power-gated boot-relative frame/record/process consumption exact across skip/commit/no-op: \[({metric}), ({metric}), ({metric})\]",
                log)
            generations = re.findall(
                rf"\[m81-table\] generation ([1-8]): PASS .*boot-relative counters=({metric})",
                log)
            if (baseline is None or len(set(baseline.groups())) != 1 or
                [int(n) for n, _ in generations] != list(range(1, 9)) or
                any(counts != baseline[1] for _, counts in generations) or
                "FS allocator refused WRITE; second marked SET DEGRADED, old READ/record intact" not in log or
                "PASS: table preflight stayed bounded, not degraded" not in log):
                raise ValueError("8.1 closure requires exact numeric bounds and same-boot failure barrier")
    if args.checkpoint in ("phase82-capspace-foundation", "phase82-volatile-permission"):
        log = args.suite_log.read_text()
        required = ("ALL TESTS PASSED (33 test suites)" if args.checkpoint == "phase82-capspace-foundation"
                    else "ALL TESTS PASSED (35 test suites)",
                    "CAPSPACE32: PASS (guest last-slot copy/move/full-refusal",
                    "x86_64-unknown-none target cap layout: identical to host, all 14 fields PASS",
                    "x86_64-unknown-none IPC: CallSlot=240 Endpoint=976 Notif=24",
                    "AUTHORIZED TRANSACTION/CRASH: PASS",
                    "EIGHT-SLOT EXHAUSTION: PASS")
        if args.checkpoint == "phase82-volatile-permission":
            required += ("VOLATILE INTEGRATION: PASS", "ADR-0050: PASS (3 guest fault boundaries)",
                         "worker died IN SYS_IPC_CALL before server death",
                         "broker-first timeout returned typed SERVICE_GONE")
        if any(item not in log for item in required):
            raise ValueError("8.2 checkpoint requires its complete historical and focused guest proofs")
    if args.checkpoint == "phase84-complete":
        log = args.suite_log.read_text()
        required = ("ALL TESTS PASSED (47 test suites)",
                    "[m84-crash] PREFIX CRASH MODEL: PASS",
                    "[m84-stage] TARGETED GUEST RESULT: PASS",
                    "[m84-red-control] PASS: red mutant allows unauthorized real stage",
                    "[m84-wrong-root] PASS: guest embedded wrong root refuses",
                    "guest verifies root-signed eight-digest cumulative capacity",
                    "32/32 AFS1 objects: typed pre-CREATE NO_SPACE",
                    "same-platter reboot detects partial highest and refuses old/false stage",
                    "root-signed ZIP-215 subordinate-key alias refused")
        if any(item not in log for item in required):
            raise ValueError("8.4 bundle requires every historical suite and guest staging/crash/red/capacity proofs")
    if args.checkpoint == "phase85-complete":
        log = args.suite_log.read_text()
        total = len(list((ROOT / "tools").glob("test_m*.py"))) + 11
        required = (
            f"ALL TESTS PASSED ({total} test suites)",
            "[m85-live-cutover] old signed v7 child ALIVE at v8 PREPARE",
            "[m85-resources] guest Power snapshots baseline/live/retired=",
            "[m85-maximal]", "[m85-crash-upgrade] real AINS2/AACT4 CREATE->WRITE->CLOSE crash-prefix matrix PASS",
            "[m85-ref-hook] RED omitted real IPC reply credit: PASS; GREEN restored exact source/EFI and guest: PASS",
            "[m85-manager-destroy] direct production proc::destroy guard under LIVE manager ID HALT",
            "[m85-manager-death-live] genuinely LIVE signed v7 child and manager Image ID:",
            "[m84-stage] TARGETED GUEST RESULT: PASS",
        )
        if any(item not in log for item in required):
            raise ValueError("8.5 bundle requires all historical/real guest/crash/red/resource proofs")
    if args.checkpoint == "phase9-complete":
        log = args.suite_log.read_text()
        total = len(list((ROOT / "tools").glob("test_m*.py"))) + 11
        required = (
            f"ALL TESTS PASSED ({total} test suites)",
            "[m9-compositor-input] distinct owned windows, z-order, preserved base, bitmap title and genuine QMP key after focused delivery: PASS",
            "[m9-client-death] real original-child exit, exact Process/region/map/cap retirement and independent QMP base/window uncover PASS",
            "[m9-client-death-red] omitted Process-liveness guest stale-pixel/timeout RED: PASS; exact restored source/EFI and QMP uncovered-pixel GREEN: PASS",
            "[m9-service-death] actual boot-root IPC fail-stop RED: PASS; byte-exact restored source/EFI and QMP key pixel GREEN: PASS",
            "[m9-resources] guest Power high-water resident 16/16 then exact child retire 15/15;",
            "[m9-compositor-model] 12/12 host state/churn and typed-wire/fuzz tests, fmt/clippy, bare-metal no_std build PASS",
            "[m9-host-dns] real guest wrong-TXID RED; same-EFI restored host peer and three guest wire queries GREEN: PASS",
            "[m85-crash-upgrade] real AINS2/AACT4 CREATE->WRITE->CLOSE crash-prefix matrix PASS",
            "[m85-resources] guest Power snapshots baseline/live/retired=",
            "[m9-gpu-pixels] GPU-only guest commands + QMP 800x600 bars/font PASS",
        )
        if any(item not in log for item in required):
            raise ValueError("phase9-complete requires complete historical, renderer, injected pixels, death/RED and resource proofs")
    release_dir = ROOT / "releases/checkpoints" / args.checkpoint
    release_dir.mkdir(parents=True, exist_ok=True)
    name = f"arenaos-{args.checkpoint}-qemu-x86_64.tar.gz"
    archive = release_dir / name
    files = FILES + PHASE9_EXTRA if args.checkpoint == "phase9-complete" else FILES
    stage = ROOT / "build/checkpoint-stage"
    if stage.exists():
        shutil.rmtree(stage)
    stage.mkdir(parents=True)
    (stage / "arena-esp.img").write_bytes(esp_bytes)
    shutil.copy2(ROOT / "build/arena-boot.efi", stage / "arena-boot.efi")
    shutil.copy2(ROOT / "build/stability-receipt.txt", stage / "stability-receipt.txt")
    shutil.copy2(arena_env.ovmf_code(), stage / "edk2-x86_64-code.fd")
    shutil.copy2(arena_env.ovmf_vars_template(), stage / "ovmf-vars-template.img")
    shutil.copy2(ROOT / "docs/RUNNING.md", stage / "RUNNING.md")
    if args.checkpoint == "phase9-complete":
        for script in PHASE9_EXTRA:
            shutil.copy2(ROOT / "tools" / script, stage / script)
        (stage / "RUNNING.md").write_text(
            "# Phase-9 qualified QEMU image\n\n"
            "Verify the outer .tar.gz.sha256 and the extracted sha256sums.txt, "
            "install QEMU and Python 3, then run `python3 phase9_archive_boot.py` "
            "in this directory. The script copies fresh OVMF vars and a fresh "
            "AFS1 scratch platter, prebinds the included exact-query host "
            "UDP/TCP peers before QEMU, captures the actual display twice, "
            "injects a virtual keyboard key and checks the focused client "
            "pixel before orderly shutdown. The host DNS peer is required "
            "for the attached NIC and does not prove public/external DNS.\n")
    afs1.mkfs(stage / "scratch-template.img", 8 * 1024 * 1024 // afs1.SECTOR)
    completed_suites = ("47/47" if args.checkpoint == "phase84-complete" else
                        f"{len(list((ROOT / 'tools').glob('test_m*.py'))) + 11}/"
                        f"{len(list((ROOT / 'tools').glob('test_m*.py'))) + 11}"
                        if args.checkpoint in ("phase85-complete", "phase9-complete") else "see commit gate")
    (stage / "QUALIFICATION.txt").write_text(
        f"Checkpoint: {args.checkpoint}\nEFI SHA-256: {efi_sha}\n"
        f"Historical suite: all passed ({completed_suites})\n"
        "Artifact-bound QEMU boots: 100/100\n"
        "Phase 8.0: COMPLETE; all four exit areas plus service-side diagnostic authority proven\n"
        + ("Phase 8.1–8.5: COMPLETE with the qualified 8.5 trust boundaries. Phase 9: userspace GOP/virtio-gpu display, isolated compositor, two owned ring-3 bitmap windows, actual QMP injected keyboard/pixels, forced original-child retirement and fail-stop service death without a restart claim. Controlled host DNS peer proves virtual UDP traffic, NOT external public DNS. No Phase-10 applications, production signing keys, hostile rollback, dynamic linking, or broader concurrent dynamic children.\n"
           if args.checkpoint == "phase9-complete" else
           "Phase 8.1–8.4: COMPLETE. Phase 8.5: COMPLETE — offline TEST-root-signed AINS/AACT install/selection, actual ring-3 v7→v8 live cutover under one unretired dynamic child, held Process STOP/FINISH before old Image ID revocation and durable commit, authentic 32/32 historical AFS1 refusal, AFS1 ordered-commit crash prefixes, fail-stop/ref-pin checks. No production key custody, production general-purpose package picker, arbitrary-sector corruption, hostile rollback, dynamic linking, Secure Boot, GC or broader concurrent children.\n"
           if args.checkpoint == "phase85-complete" else
           "Phase 8.1/8.2/8.3: COMPLETE. Phase 8.4: COMPLETE — USERSPACE VERIFIED STAGING ONLY; public TEST root, signed canonical package/policy, revocation/version refusal, bounded immutable AFS1 stages, documented crash prefixes. NOT installed, activated or an Image cap. No production-key custody, Secure Boot, hostile-disk rollback defense or 8.5 installer. Eight signed revocations in guest; ninth distinct typed issuer-side refusal before signing (user-approved v1 interpretation).\n"
           if args.checkpoint == "phase84-complete" else
           "Phase 8.1: COMPLETE (unchanged transactional store); Phase 8.2: VOLATILE INTEGRATION ONLY: default-DENY mediator, marker-authorized in-memory ALLOW/DENY/REVOKE, 128-bit service bearer and mediated arena.txt READ; IPC dead-caller teardown corrected. No persisted decision or reboot/restart/crash-model permission guarantee; 8.2 INCOMPLETE\n"
           if args.checkpoint == "phase82-volatile-permission" else
           "Phase 8.1: COMPLETE (unchanged transactional store); Phase 8.2: CAP-SPACE FOUNDATION ONLY: 32 fixed slots with full-table refusal and accounting; no permissiond/CLI/approval/grant; 8.2 INCOMPLETE\n"
           if args.checkpoint == "phase82-capspace-foundation" else
           "Phase 8.1: COMPLETE; AFS1 ordered-write/atomic-sector model; visible config-record integrity; exact boot-relative resources, bounded table/disk refusal and DEGRADED barrier proven\n"
           if args.checkpoint == "phase81-complete" else
           "Phase 8.1: TRANSACTIONAL CORE; positive SET/crash/table/disk proofs; 8.1 INCOMPLETE pending resource/fault accounting\n"
           if args.checkpoint == "phase81-transactional-core" else
           "Phase 8.1: READ BOUNDARY ONLY; no authorized SET, 8.1 INCOMPLETE\n"
           if args.checkpoint.startswith("phase81-") else "")
    )
    (stage / "sha256sums.txt").write_text("".join(
        f"{digest((stage / path).read_bytes())}  {path}\n" for path in files
    ))
    with tarfile.open(archive, "w:gz") as tar:
        for path in (*files, "sha256sums.txt"):
            tar.add(stage / path, arcname=path)
    archive_sha = digest(archive.read_bytes())
    (release_dir / f"{name}.sha256").write_text(f"{archive_sha}  {name}\n")

    # Verify the distributed bytes, not the staging directory. Use the
    # extracted firmware pair and a fresh copy of the BUNDLED AFS1 disk.
    with tempfile.TemporaryDirectory(prefix="checkpoint-verify-", dir=ROOT / "build") as tmp:
        unpacked = Path(tmp)
        # Only names we generated are allowed; extractfile avoids
        # Python-version-dependent extractall filters and path traversal.
        with tarfile.open(archive, "r:gz") as tar:
            allowed = set((*files, "sha256sums.txt"))
            if {m.name for m in tar} != allowed or any(not m.isfile() for m in tar):
                raise ValueError("archive contains unexpected entries")
            for m in tar:
                src = tar.extractfile(m)
                if src is None:
                    raise ValueError(f"archive entry unreadable: {m.name}")
                (unpacked / m.name).write_bytes(src.read())
        for line in (unpacked / "sha256sums.txt").read_text().splitlines():
            sha, path = line.split(maxsplit=1)
            if digest((unpacked / path).read_bytes()) != sha:
                raise ValueError(f"archive checksum mismatch: {path}")
        if digest((unpacked / "arena-boot.efi").read_bytes()) != efi_sha or \
                (unpacked / "stability-receipt.txt").read_text().strip() != f"{efi_sha} 100/100":
            raise ValueError("extracted ELF/100-boot receipt differs from qualified image")
        scratch = unpacked / "scratch.img"
        shutil.copy2(unpacked / "scratch-template.img", scratch)
        old_code = os.environ.get("ARENA_OVMF_CODE")
        old_vars = os.environ.get("ARENA_OVMF_VARS")
        try:
            os.environ["ARENA_OVMF_CODE"] = str(unpacked / "edk2-x86_64-code.fd")
            os.environ["ARENA_OVMF_VARS"] = str(unpacked / "ovmf-vars-template.img")
            rc, serial, _ = mtest.boot("checkpoint-bundle", unpacked / "arena-esp.img",
                                       [((b"servicemgr: production netstackd READY pid", b"arena>"),
                                         1, b"stackstop\r"),
                                        (b"arena>", 2, b"shutdown\r")], scratch)
        finally:
            for key, old in (("ARENA_OVMF_CODE", old_code), ("ARENA_OVMF_VARS", old_vars)):
                if old is None:
                    os.environ.pop(key, None)
                else:
                    os.environ[key] = old
        required = ("m7: RESULT PASS (2/2)",
                    "servicemgr: production netstackd READY pid",
                    "five inherited child caps audited (netd/W stack/R backoff/RW rngd/W diag/R)",
                    "manager-owned dependency probe pid",
                    "depcheck: netd MAC answered",
                    "depcheck: production driver poison opcodes refused without marker",
                    "depcheck: rngd device completed 64 varied bytes",
                    "servicemgr: active netd MAC and rngd entropy probes passed; worker reaped",
                    "servicemgr: forcibly stopped LIVE production child through held Process cap",
                    "m8: stackstop PASS (manager mode-1 stopped live production child, new wire, resources flat)",
                    "halting via UEFI ResetSystem(shutdown)")
        if args.checkpoint in ("phase84-complete", "phase85-complete", "phase9-complete"):
            required += (("packaged: boot with exact FS/W endpoint/R STAGE/R registrar/W lifecycle/R; namespace scan verified"
                          if args.checkpoint in ("phase85-complete", "phase9-complete") else
                          "packaged: boot with exact FS/W endpoint/R marker/R; namespace scan verified"),
                         "servicemgr: packaged READY (full boot scan; exact PING + exit + deadline)",
                         "permissiond: validated durable policy generation 0 DENY",
                         "m83: returncap PASS (40 real reply caps rejected and discarded")
        if args.checkpoint.startswith("phase81-") or args.checkpoint.startswith("phase82-"):
            required += ("configread: SET absent/wrong-kind refused x20 by receiver",
                         "configread: READ UNSET",
                         "configread: ORDINARY READ BOUNDARY PASS (no fsd or marker grant)",
                         "configread: boot-root reader reaped; no update authority delegated")
        if args.checkpoint in ("phase81-transactional-core", "phase81-complete", "phase82-capspace-foundation", "phase82-volatile-permission"):
            required += ("configup: SKIP (no trusted test intent; no SET)",
                         "configup: boot-root updater reaped; marker never delegated to shell")
        if args.checkpoint.startswith("phase82-"):
            required += ("capability_spaces: 32 slots/space",
                         "m3:test:capability_spaces: PASS")
        if args.checkpoint == "phase82-volatile-permission":
            required += ("permissiond READY (volatile policy; default DENY)",
                         "servicemgr: permission PING result + exit before deadline; worker reaped",
                         "ADR-0050 four abandoned caller states cleared; staged caps discarded; late server reply typed STATUS_BAD_ARG; endpoint recycled; frames exact")
        if (rc != 0 or "PANIC" in serial or "halting machine:" in serial
                or "[arena ERROR halt]" in serial or "[arena ERROR ipc]" in serial
                or any(item not in serial for item in required)
                or serial.count("servicemgr: production netstackd READY pid") != 2
                or serial.count("servicemgr: active netd MAC and rngd entropy probes passed; worker reaped") != 2
                or "m8: stackstop FAIL" in serial or "servicemgr: OFFLINE" in serial):
            (ROOT / "build/checkpoint-bundle-failure.log").write_text(serial)
            raise ValueError("extracted bundle did not boot and shut down cleanly")
        if args.checkpoint == "phase9-complete":
            # Separate process with cwd pointing ONLY at extracted files:
            # archived peer, firmware and QMP/keyboard pixel oracle.
            subprocess.run([sys.executable, "phase9_archive_boot.py"],
                           cwd=unpacked, check=True, timeout=120)

    print(f"VERIFIED checkpoint bundle: {archive.relative_to(ROOT)}")
    print(f"archive SHA-256: {archive_sha}; qualified EFI SHA-256: {efi_sha}")
    print("extracted bundle: checksums and real QEMU boot PASS")
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (OSError, ValueError) as exc:
        sys.exit(f"checkpoint bundle refused: {exc}")
