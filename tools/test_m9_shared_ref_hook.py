#!/usr/bin/env python3
"""Deliberate RED/GREEN on the actual SharedRegion cap retirement hook.

Omit ONE production cap::install drop credit. The independent capspace,
IPC escrow and PTE conservation walk must halt during the real ring-3
probe. Restore byte-identical source and shipping EFI/ESP in finally.
"""
import hashlib
import subprocess
import sys
from pathlib import Path
import arena_env
import mtest

ROOT = Path(__file__).resolve().parent.parent
SOURCE = ROOT / 'kernel/kernel/src/cap.rs'
EFI = ROOT / 'build/arena-boot.efi'
ESP = ROOT / 'build/arena-esp.img'
NEEDLE = b'            crate::shared::drop_cap(old);\n            cs.ipc_landed[slot] = false;'
MUTANT = b'            // RED ONLY: omit the real SharedRegion retirement hook.\n            cs.ipc_landed[slot] = false;'


def build(log):
    with log.open('w') as f:
        subprocess.run(['bash', 'tools/build.sh', '--image'], cwd=ROOT,env=arena_env.rust_env() | {'ARENA_GRAPHICS_FIXTURE':'phase9'},
                       stdout=f, stderr=subprocess.STDOUT, check=True)


def main():
    original = SOURCE.read_bytes()
    assert original.count(NEEDLE) == 1 and MUTANT not in original
    bdir = arena_env.build_dir()
    build(bdir / 'm9-shared-red-control-original-build.log')
    artifacts = {path: path.read_bytes() for path in (EFI, ESP)}
    digest = hashlib.sha256(original).hexdigest()
    red = False
    try:
        SOURCE.write_bytes(original.replace(NEEDLE, MUTANT))
        esp = mtest.build('m9-shared-hook-red')
        disk = arena_env.make_scratch_disk()
        rc, serial, _ = mtest.boot('m9-shared-hook-red', esp, [], disk,
                                   timeout_s=55)
        (bdir / 'm9-shared-hook-red-serial.log').write_text(serial)
        # mtest deliberately converts QEMU's 0 into a NONZERO semantic
        # result when it sees a guest fatal; RED must fail this gate.
        red = (rc != 0
               and serial.count('[arena ERROR halt] halting machine: SharedRegion ref/map conservation failure') == 1
               and '[sharedprobe] capacity/rights/zero PASS' not in serial
               and '[displayd] ring3 GOP pixels ready' not in serial)
    finally:
        SOURCE.write_bytes(original)
        try:
            build(bdir / 'm9-shared-red-control-restored-build.log')
        finally:
            for path, data in artifacts.items():
                path.write_bytes(data)
        assert hashlib.sha256(SOURCE.read_bytes()).hexdigest() == digest
        assert all(path.read_bytes() == data for path, data in artifacts.items()), 'mutant image escaped'
    try:
        proc = subprocess.run([sys.executable, str(ROOT / 'tools/test_m9_gop_handoff.py')],
                              cwd=ROOT, stdout=(bdir / 'm9-shared-hook-green.log').open('w'),
                              stderr=subprocess.STDOUT)
        green = proc.returncode == 0
    finally:
        for path, data in artifacts.items():
            path.write_bytes(data)
    assert all(path.read_bytes() == data for path, data in artifacts.items()), 'control changed EFI'
    print(f'[m9-shared-ref-hook] guest RED omitted real retirement hook: {"PASS" if red else "FAIL"}; '
          f'guest GREEN exact source/EFI and QMP pixels: {"PASS" if green else "FAIL"}', flush=True)
    return 0 if red and green else 1


if __name__ == '__main__':
    sys.exit(main())
