#!/usr/bin/env python3
"""Guest RED/GREEN for the production SharedRegion INFO READ-cap gate.

The mutant omits only one rights check. A real ring-3 WRITE-only copy
then improperly reads the page count; the independent guest probe must
halt, while exact restored source/EFI boots and yields real QMP pixels.
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
NEEDLE = (b'    if cap.rights & crate::cap::RIGHTS_READ == 0 {\n'
          b'        return STATUS_BAD_ARG;\n'
          b'    }\n'
          b'    let Some((_, pages)) = crate::shared::backing(id) else {')
MUTANT = (b'    // RED ONLY: accept a WRITE-only cap for the size query.\n'
          b'    if false {\n'
          b'        return STATUS_BAD_ARG;\n'
          b'    }\n'
          b'    let Some((_, pages)) = crate::shared::backing(id) else {')


def build(path: Path):
    with path.open('w') as out:
        subprocess.run(['bash', 'tools/build.sh', '--image'], cwd=ROOT,
                       stdout=out, stderr=subprocess.STDOUT, check=True)


def main() -> int:
    original = SOURCE.read_bytes()
    assert original.count(NEEDLE) == 1 and MUTANT not in original
    bdir = arena_env.build_dir()
    build(bdir / 'm9-info-original-build.log')
    artifacts = {path: path.read_bytes() for path in (EFI, ESP)}
    source_hash = hashlib.sha256(original).hexdigest()
    red = False
    try:
        SOURCE.write_bytes(original.replace(NEEDLE, MUTANT))
        esp = mtest.build('m9-info-rights-red')
        disk = arena_env.make_scratch_disk()
        rc, serial, _ = mtest.boot('m9-info-rights-red', esp, [], disk, timeout_s=55)
        (bdir / 'm9-info-rights-red-serial.log').write_text(serial)
        red = (rc != 0
               and serial.count('[arena ERROR halt] halting machine: '
                                'sharedprobe: ring3 refusal/rights/zero assertion failed') == 1
               and '[sharedprobe] held SharedRegion INFO bound/refusal PASS' not in serial
               and '[displayd] ring3 GOP pixels ready' not in serial)
    finally:
        SOURCE.write_bytes(original)
        try:
            build(bdir / 'm9-info-restored-build.log')
        finally:
            for path, data in artifacts.items():
                path.write_bytes(data)
        assert hashlib.sha256(SOURCE.read_bytes()).hexdigest() == source_hash
        assert all(path.read_bytes() == data for path, data in artifacts.items())
    try:
        with (bdir / 'm9-info-green.log').open('w') as out:
            green = subprocess.run([sys.executable, str(ROOT / 'tools/test_m9_gop_handoff.py')],
                                   cwd=ROOT, stdout=out, stderr=subprocess.STDOUT).returncode == 0
    finally:
        for path, data in artifacts.items():
            path.write_bytes(data)
    assert all(path.read_bytes() == data for path, data in artifacts.items())
    print(f'[m9-shared-info] guest omitted READ gate RED: {"PASS" if red else "FAIL"}; '
          f'exact source/EFI and real QMP pixels GREEN: {"PASS" if green else "FAIL"}')
    return 0 if red and green else 1


if __name__ == '__main__':
    sys.exit(main())
