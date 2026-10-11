#!/usr/bin/env python3
"""RED mutation: revoke a verified APB1 Image before Desktop receives it."""
from __future__ import annotations

import hashlib
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import afs2
import arena_env
import mtest
from test_phase14_negative_guest import install_and_refuse
from test_phase14_pie_guest import APP_ID, bundle_bytes

ROOT = Path(__file__).resolve().parents[1]
PACKAGED = ROOT / "userspace/packaged/src/main.rs"
DESKTOP_PACKAGE = ROOT / "userspace/desktop/src/package.rs"
EFI = ROOT / "build/arena-boot.efi"
ESP = ROOT / "build/arena-esp.img"
PROFILE = ROOT / "build/graphics-profile.txt"
LABEL = "phase14-stale-image"

NEEDLE = b'''        Ok((image_id, _image_info)) => {
            answer[..32].copy_from_slice(&digest);'''
MUTANT = b'''        Ok((image_id, _image_info)) => {
            // RED mutation: deliver a valid capability whose Image ID was revoked.
            if unsafe { syscall6(SYS_IMAGE_REVOKE, REGISTRAR, image_id as u64, 0, 0, 0, 0) } != 0 {
                fail("test mutation could not revoke verified Image");
            }
            say("packaged: TEST mutant revoked verified Image before transfer");
            answer[..32].copy_from_slice(&digest);'''
INFO_NEEDLE = b'''    let valid = unsafe { syscall2(SYS_CAP_DESCRIBE, image, desc.as_mut_ptr() as u64) } == 0
        && desc[0] == 1
        && desc[2] & (RIGHTS_READ | RIGHTS_COPY | RIGHTS_DESTROY)
            == RIGHTS_READ | RIGHTS_COPY | RIGHTS_DESTROY
        && unsafe { syscall6(SYS_IMAGE_INFO, image, info.as_mut_ptr() as u64, 0, 0, 0, 0) } == 0
        && info[0] >= info[1]
        && (1..=NATIVE_IMAGE_BYTES_MAX as u64).contains(&info[2]);'''
INFO_MUTANT = b'''    let valid = {
        // RED mutation: keep the launch moving after the broker delivered
        // the deliberately revoked Image. The kernel spawn preflight remains
        // the authority check under test; these are the fixture's known
        // validated link-time values, not a randomized placement.
        info = [0x400160, 0x400000, 23256];
        true
    };'''


def main() -> None:
    original_source = PACKAGED.read_bytes()
    original_package_client = DESKTOP_PACKAGE.read_bytes()
    assert original_source.count(NEEDLE) == 1 and MUTANT not in original_source
    assert original_package_client.count(INFO_NEEDLE) == 1 and INFO_MUTANT not in original_package_client

    # Preserve a clean production build and restore it exactly after the
    # test-only trust-path mutation, including the graphics profile.
    production_esp = mtest.build(LABEL + "-production-baseline", desktop=True)
    artifacts = {path: path.read_bytes() for path in (EFI, ESP, PROFILE)}
    assert production_esp == ESP
    artifact_hashes = {path: hashlib.sha256(blob).digest() for path, blob in artifacts.items()}
    mutation_passed = False
    try:
        PACKAGED.write_bytes(original_source.replace(NEEDLE, MUTANT))
        DESKTOP_PACKAGE.write_bytes(original_package_client.replace(INFO_NEEDLE, INFO_MUTANT))
        esp = mtest.build(LABEL + "-mutant", desktop=True)
        disk = arena_env.make_scratch_disk(afs2=True)
        rc, serial, _ = mtest.boot(
            LABEL + "-afs2-seed",
            esp,
            [(b"filesd: AFS2 mounted", 1, b"shutdown\r")],
            disk,
            pointer=True,
            timeout_s=180,
        )
        assert rc == 0 and "filesd: AFS2 mounted" in serial, serial[-3000:]

        bundle = bundle_bytes()
        from test_phase14_pie_guest import seed_bundle

        seed_bundle(disk, bundle)
        rc, serial, elapsed = mtest.boot(
            LABEL,
            esp,
            [
                (
                    (b"[desktop] real desktop frame presented", b"filesd: AFS2 mounted"),
                    1,
                    lambda: install_and_check(disk, bundle),
                )
            ],
            disk,
            pointer=True,
            timeout_s=180,
        )
        (arena_env.build_dir() / f"serial-{LABEL}.log").write_text(serial)
        mutation_passed = (
            rc == 0
            and "packaged: TEST mutant revoked verified Image before transfer" in serial
            and "[desktop] launch-refusal stage/status=headless-spawn-check/18446744073709551614" in serial
            and "[phase14-pie] PASS" not in serial
            and "halting via UEFI ResetSystem(shutdown)" in serial
        )
        if not mutation_passed:
            raise AssertionError(serial[-5000:])
        print(
            f"[{LABEL}] signed APB1 install delivered revoked Image authority; kernel spawn "
            f"preflight refused it with STATUS_BAD_ARG and ownership receipts returned "
            f"({elapsed:.1f}s) PASS",
            flush=True,
        )
    finally:
        PACKAGED.write_bytes(original_source)
        DESKTOP_PACKAGE.write_bytes(original_package_client)
        for path, blob in artifacts.items():
            path.write_bytes(blob)
        assert PACKAGED.read_bytes() == original_source
        assert DESKTOP_PACKAGE.read_bytes() == original_package_client
        assert all(path.read_bytes() == blob for path, blob in artifacts.items())
        assert all(hashlib.sha256(path.read_bytes()).digest() == artifact_hashes[path] for path in artifacts)

    if not mutation_passed:
        raise SystemExit("revoked Image authority was not rejected")


def install_and_check(disk: Path, bundle: bytes) -> bytes:
    serial = install_and_refuse(
        LABEL,
        disk,
        bundle,
        APP_ID,
        "Phase14 PIE",
        "headless-spawn-check",
        -2,
        False,
            "packaged: TEST mutant revoked verified Image before transfer",
    )
    return serial


if __name__ == "__main__":
    main()
