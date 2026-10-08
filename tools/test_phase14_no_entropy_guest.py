#!/usr/bin/env python3
"""RED mutation proving a signed installed PIE fails closed without seed state."""
from __future__ import annotations

import hashlib
import subprocess
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import afs2
import arena_env
import mtest
from test_phase14_negative_guest import install_and_refuse
from test_phase14_pie_guest import APP_ID, bundle_bytes

ROOT = Path(__file__).resolve().parents[1]
RNGD = ROOT / "userspace/rngd/src/main.rs"
EFI = ROOT / "build/arena-boot.efi"
ESP = ROOT / "build/arena-esp.img"
PROFILE = ROOT / "build/graphics-profile.txt"
LABEL = "phase14-no-entropy"

NEEDLE = b'''            let status = syscall6(
                SYS_ENTROPY_SEED,
                SLOT_KERNEL_SEED,
                va[1],
                0,
                0,
                0,
                0,
            );
            if status != 0 {
                log_line(|o| {
                    o.str("rngd: kernel entropy seed refused with status ");
                    o.i64(status);
                });
                fail(EXIT_SEED, "the kernel did not accept the virtio-rng seed");
            }
            core::ptr::write_bytes(va[1] as *mut u8, 0, 32);
            log("rngd: submitted a device-filled 256-bit seed to the kernel CSPRNG");'''
MUTANT = b'''            // RED mutation: deliberately withhold the genuine device bytes
            // from the kernel while allowing the manager-ready signal below.
            core::ptr::write_bytes(va[1] as *mut u8, 0, 32);
            log("rngd: TEST mutant withheld the device-filled seed from the kernel");'''


def build(label: str) -> Path:
    return mtest.build(label, desktop=True)


def main() -> None:
    original_source = RNGD.read_bytes()
    assert original_source.count(NEEDLE) == 1 and MUTANT not in original_source

    # Build and preserve a clean release-profile checkpoint before the red
    # mutation. The exact EFI/ESP/profile are restored after the guest proof.
    production_esp = build(LABEL + "-production-baseline")
    artifacts = {path: path.read_bytes() for path in (EFI, ESP, PROFILE)}
    assert production_esp == ESP
    artifact_hashes = {path: hashlib.sha256(blob).digest() for path, blob in artifacts.items()}
    mutant_result = False
    try:
        RNGD.write_bytes(original_source.replace(NEEDLE, MUTANT))
        esp = build(LABEL + "-mutant")
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
                    lambda: install_and_refuse(
                        LABEL,
                        disk,
                        bundle,
                        APP_ID,
                        "Phase14 PIE",
                        "headless-spawn-check",
                        -9,
                        False,
                    ),
                )
            ],
            disk,
            pointer=True,
            timeout_s=180,
        )
        (arena_env.build_dir() / f"serial-{LABEL}.log").write_text(serial)
        mutant_result = (
            rc == 0
            and "rngd: TEST mutant withheld the device-filled seed from the kernel" in serial
            and "rngd: submitted a device-filled 256-bit seed to the kernel CSPRNG" not in serial
            and "[desktop] launch-refusal stage/status=headless-spawn-check/18446744073709551607" in serial
            and "[arena INFO  aslr] PIE placement" not in serial
            and "halting via UEFI ResetSystem(shutdown)" in serial
        )
        if not mutant_result:
            raise AssertionError(serial[-5000:])
        print(
            f"[{LABEL}] signed APB1 install reached production spawn preflight; absent kernel "
            f"seed refused with STATUS_NO_ENTROPY and full service boot stayed live "
            f"({elapsed:.1f}s) PASS",
            flush=True,
        )
    finally:
        RNGD.write_bytes(original_source)
        # Restore the exact unmutated artifacts regardless of RED outcome.
        for path, blob in artifacts.items():
            path.write_bytes(blob)
        assert RNGD.read_bytes() == original_source
        assert all(path.read_bytes() == blob for path, blob in artifacts.items())
        assert all(hashlib.sha256(path.read_bytes()).digest() == artifact_hashes[path] for path in artifacts)

    if not mutant_result:
        raise SystemExit("missing-entropy negative did not fail closed")


if __name__ == "__main__":
    main()
