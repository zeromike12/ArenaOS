#!/usr/bin/env python3
"""Package a qualified Phase 8 commit with its OWN bootable QEMU build.

Run after tools/run_tests.sh and tools/stability_loop.sh 100, before the
single source+artifact commit. This is NOT a milestone release; it does
not turn an incomplete 8.0 into a completed one. Reject an unrelated
EFI, an ESP with different bytes, or a partial/stale receipt. Extract
and boot the actual tarball from a formatted bundled disk before exit.
"""
import argparse
import hashlib
import os
import re
import shutil
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
FILES = ("arena-esp.img", "scratch-template.img", "edk2-x86_64-code.fd",
         "ovmf-vars-template.img", "RUNNING.md", "QUALIFICATION.txt")


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
    if not suite or int(suite.group(1)) < 21 or \
            "INITIAL-START SUBSTRATE: PASS" not in log or \
            "PRODUCTION ORDERLY-RESTART SUBSTRATE: PASS" not in log:
        raise ValueError("all 21+ historical suites + actual production restart proof required")
    return digest(efi), esp.read_bytes()


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("checkpoint", help="stable checkpoint name, e.g. phase8-initial-stack")
    parser.add_argument("suite_log", type=Path, help="output of full tools/run_tests.sh")
    args = parser.parse_args()
    if not re.fullmatch(r"[a-z0-9][a-z0-9-]*", args.checkpoint):
        parser.error("checkpoint name must contain lowercase letters/digits/hyphens")
    efi_sha, esp_bytes = verify_qualified_image(args.suite_log)
    release_dir = ROOT / "releases/checkpoints" / args.checkpoint
    release_dir.mkdir(parents=True, exist_ok=True)
    name = f"arenaos-{args.checkpoint}-qemu-x86_64.tar.gz"
    archive = release_dir / name
    stage = ROOT / "build/checkpoint-stage"
    if stage.exists():
        shutil.rmtree(stage)
    stage.mkdir(parents=True)
    (stage / "arena-esp.img").write_bytes(esp_bytes)
    shutil.copy2(arena_env.ovmf_code(), stage / "edk2-x86_64-code.fd")
    shutil.copy2(arena_env.ovmf_vars_template(), stage / "ovmf-vars-template.img")
    shutil.copy2(ROOT / "docs/RUNNING.md", stage / "RUNNING.md")
    afs1.mkfs(stage / "scratch-template.img", 8 * 1024 * 1024 // afs1.SECTOR)
    (stage / "QUALIFICATION.txt").write_text(
        f"Checkpoint: {args.checkpoint}\nEFI SHA-256: {efi_sha}\n"
        "Historical suite: all passed (see commit gate)\n"
        "Artifact-bound QEMU boots: 100/100\n"
        "Phase 8.0: INCOMPLETE; one orderly restart proven, crash and lifecycle/accounting gates open\n"
    )
    (stage / "sha256sums.txt").write_text("".join(
        f"{digest((stage / path).read_bytes())}  {path}\n" for path in FILES
    ))
    with tarfile.open(archive, "w:gz") as tar:
        for path in (*FILES, "sha256sums.txt"):
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
            allowed = set((*FILES, "sha256sums.txt"))
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
        scratch = unpacked / "scratch.img"
        shutil.copy2(unpacked / "scratch-template.img", scratch)
        old_code = os.environ.get("ARENA_OVMF_CODE")
        old_vars = os.environ.get("ARENA_OVMF_VARS")
        try:
            os.environ["ARENA_OVMF_CODE"] = str(unpacked / "edk2-x86_64-code.fd")
            os.environ["ARENA_OVMF_VARS"] = str(unpacked / "ovmf-vars-template.img")
            rc, serial, _ = mtest.boot("checkpoint-bundle", unpacked / "arena-esp.img",
                                       [(b"servicemgr: production netstackd READY pid", 1, b"stacktest\r"),
                                        (b"arena>", 2, b"shutdown\r")], scratch)
        finally:
            for key, old in (("ARENA_OVMF_CODE", old_code), ("ARENA_OVMF_VARS", old_vars)):
                if old is None:
                    os.environ.pop(key, None)
                else:
                    os.environ[key] = old
        required = ("m7: RESULT PASS (2/2)",
                    "servicemgr: production netstackd READY pid",
                    "four installed child caps audited (netd/W stack/R backoff/RW rngd/W)",
                    "m8: stacktest Process-cap stop refused to endpoint-only client",
                    "m8: stacktest observed SERVICE_GONE during child absence",
                    "m8: stacktest PASS (same endpoint; old bearer revoked; fresh ARP request on real wire)",
                    "halting via UEFI ResetSystem(shutdown)")
        if (rc != 0 or "PANIC" in serial or any(item not in serial for item in required)
                or serial.count("servicemgr: production netstackd READY pid") != 2):
            (ROOT / "build/checkpoint-bundle-failure.log").write_text(serial)
            raise ValueError("extracted bundle did not boot and shut down cleanly")

    print(f"VERIFIED checkpoint bundle: {archive.relative_to(ROOT)}")
    print(f"archive SHA-256: {archive_sha}; qualified EFI SHA-256: {efi_sha}")
    print("extracted bundle: checksums and real QEMU boot PASS")
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (OSError, ValueError) as exc:
        sys.exit(f"checkpoint bundle refused: {exc}")
