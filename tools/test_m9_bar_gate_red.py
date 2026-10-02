#!/usr/bin/env python3
"""Guest RED/GREEN for virtio device-info held-MMIO-cap coverage.

Omit only the physical *extent* check, retain the exact BAR base and
READ right, and demonstrate a real short-lived ring-3 probe improperly
receiving the full device record for its one-page cap to a four-page BAR.
Restore source/EFI/ESP byte-for-byte, then verify green QMP pixels.
"""
import hashlib
from pathlib import Path
import subprocess
import sys

import arena_env
import mtest

ROOT = Path(__file__).resolve().parent.parent
SOURCE = ROOT / 'kernel/kernel/src/arch/x86_64/syscall.rs'
EFI = ROOT / 'build/arena-boot.efi'
ESP = ROOT / 'build/arena-esp.img'
NEEDLE = b'if phys == base && u64::from(pages) * 4096 >= last_byte)'
MUTANT = b'if phys == base && pages != 0) // RED ONLY: ignore BAR extent'


def build(path: Path):
    with path.open('w') as out:
        subprocess.run(['bash', 'tools/build.sh', '--image'], cwd=ROOT,
                       stdout=out, stderr=subprocess.STDOUT, check=True)


def main() -> int:
    original = SOURCE.read_bytes()
    assert original.count(NEEDLE) == 1 and MUTANT not in original
    bdir = arena_env.build_dir()
    build(bdir / 'm9-bar-gate-original-build.log')
    artifacts = {path: path.read_bytes() for path in (EFI, ESP)}
    digest = hashlib.sha256(original).hexdigest()
    red = False
    try:
        SOURCE.write_bytes(original.replace(NEEDLE, MUTANT))
        esp = mtest.build('m9-bar-extent-red')
        disk = arena_env.make_scratch_disk()
        rc, serial, _ = mtest.boot('m9-bar-extent-red', esp, [], disk, timeout_s=55)
        (bdir / 'm9-bar-extent-red-serial.log').write_text(serial)
        red = (rc != 0
               and serial.count('[arena ERROR halt] halting machine: '
                                'sharedprobe: ring3 refusal/rights/zero assertion failed') == 1
               and '[sharedprobe] truncated virtio BAR device-info refused PASS' not in serial
               and '[displayd] ring3 GOP pixels ready' not in serial)
    finally:
        SOURCE.write_bytes(original)
        try:
            build(bdir / 'm9-bar-gate-restored-build.log')
        finally:
            for path, data in artifacts.items():
                path.write_bytes(data)
        assert hashlib.sha256(SOURCE.read_bytes()).hexdigest() == digest
        assert all(path.read_bytes() == data for path, data in artifacts.items()), 'mutant EFI escaped'
    try:
        with (bdir / 'm9-bar-gate-green.log').open('w') as out:
            green = subprocess.run([sys.executable, str(ROOT / 'tools/test_m9_gop_handoff.py')],
                                   cwd=ROOT, stdout=out,
                                   stderr=subprocess.STDOUT).returncode == 0
    finally:
        for path, data in artifacts.items():
            path.write_bytes(data)
    assert hashlib.sha256(SOURCE.read_bytes()).hexdigest() == digest
    assert all(path.read_bytes() == data for path, data in artifacts.items())
    print(f'[m9-bar-gate] guest truncated-device-cap RED: {"PASS" if red else "FAIL"}; '
          f'exact source/EFI and real QMP pixels GREEN: {"PASS" if green else "FAIL"}')
    return 0 if red and green else 1


if __name__ == '__main__':
    sys.exit(main())
