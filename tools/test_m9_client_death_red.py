#!/usr/bin/env python3
"""Omit original-client liveness check: real guest RED, restored QMP GREEN.

An exited child still has a copied surface cap in the compositor. If the
Process-cap liveness check is disabled, root MUST fail-stop when that stale
copy/map cannot be retired within the bounded deadline. A serial PASS or a
QEMU exit zero never counts as a successful lifecycle proof.
"""
import hashlib
import re
import subprocess
import sys
from pathlib import Path

import arena_env
import mtest
import qmp

ROOT = Path(__file__).resolve().parent.parent
SOURCE = ROOT / 'phase9-work/compositord-main.rs'
EFI, ESP = ROOT / 'build/arena-boot.efi', ROOT / 'build/arena-esp.img'
NEEDLE = b'let life = unsafe { syscall1(SYS_PROC_LIVE, process) };'
MUTANT = b'let life = 1; // RED ONLY: omit original Process/READ witness'
LABEL = 'm9-client-death-red'


def build(log: Path) -> None:
    with log.open('w') as f:
        subprocess.run(['bash', 'tools/build.sh', '--image'], cwd=ROOT,
                       stdout=f, stderr=subprocess.STDOUT, check=True)


def red_picture() -> bytes:
    path = arena_env.build_dir() / f'{LABEL}.ppm'
    conn = qmp.Qmp(str(arena_env.build_dir() / f'qmp-{LABEL}.sock'))
    try:
        conn.command('screendump', filename=str(path), format='ppm')
    finally:
        conn.close()
    return b''


def still_stale() -> bool:
    data = (arena_env.build_dir() / f'{LABEL}.ppm').read_bytes()
    h = re.match(rb'P6\s+800\s+600\s+255\s', data)
    if not h or len(data) - h.end() != 800 * 600 * 3:
        return False
    offset = h.end() + 3 * (190 * 800 + 200)
    return tuple(data[offset:offset+3]) == (0x3d, 0xcf, 0x7a)


def main() -> int:
    original = SOURCE.read_bytes()
    assert original.count(NEEDLE) == 1 and MUTANT not in original
    bdir = arena_env.build_dir()
    build(bdir / f'{LABEL}-original-build.log')
    artifacts = {p: p.read_bytes() for p in (EFI, ESP)}
    sha = hashlib.sha256(original).hexdigest()
    red = False
    try:
        SOURCE.write_bytes(original.replace(NEEDLE, MUTANT))
        esp = mtest.build(LABEL)
        rc, serial, _ = mtest.run_qemu(
            LABEL, esp,
            feed=[(b'[window_b] original child exiting without DESTROY', 1, red_picture)],
            keys=mtest.DEFAULT_KEYS + [(b'[window_b] held-cap focused surface painted', 1, 'x')])
        (bdir / f'{LABEL}-serial.log').write_text(serial)
        red = (rc != 0 and 'm7: RESULT PASS (2/2)' in serial
               and still_stale()
               and 'graphics: dead original child never retired its copied region' in serial
               and '[arena ERROR halt]' in serial
               and 'graphics original child 1 retired:' not in serial
               and '[compositord] dead original child retired' not in serial)
    finally:
        SOURCE.write_bytes(original)
        try:
            build(bdir / f'{LABEL}-restored-build.log')
        finally:
            for p, data in artifacts.items():
                p.write_bytes(data)
        assert hashlib.sha256(SOURCE.read_bytes()).hexdigest() == sha
        assert all(p.read_bytes() == data for p, data in artifacts.items())
    with (bdir / f'{LABEL}-green.log').open('w') as log:
        green = subprocess.run([sys.executable, 'tools/test_m9_client_death.py'],
                               cwd=ROOT, stdout=log, stderr=subprocess.STDOUT).returncode == 0
    for p, data in artifacts.items():
        p.write_bytes(data)
    assert all(p.read_bytes() == data for p, data in artifacts.items())
    print(f'[m9-client-death-red] omitted Process-liveness guest stale-pixel/timeout RED: {"PASS" if red else "FAIL"}; '
          f'exact restored source/EFI and QMP uncovered-pixel GREEN: {"PASS" if green else "FAIL"}')
    return 0 if red and green else 1


if __name__ == '__main__':
    sys.exit(main())
