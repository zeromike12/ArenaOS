#!/usr/bin/env python3
"""Real guest font RED/GREEN control, not a synthetic PPM mutation.

Change one *production* 5x7 glyph row before building the embedded display
service, capture QMP pixels and require the real oracle to reject the exact
foreground sample. Finally restore source and shipping EFI/ESP byte-for-byte,
then boot and independently verify the restored source's QMP font pixels.
"""
import hashlib
from pathlib import Path
import subprocess
import sys

import arena_env

ROOT = Path(__file__).resolve().parent.parent
SOURCE = ROOT / 'userspace/gfxkit/src/lib.rs'
EFI = ROOT / 'build/arena-boot.efi'
ESP = ROOT / 'build/arena-esp.img'
NEEDLE = b"b'A' => [14, 17, 17, 31, 17, 17, 17],"
MUTANT = b"b'A' => [0, 17, 17, 31, 17, 17, 17], // RED only: missing top bar"
ORACLE = [sys.executable, str(ROOT / 'tools/test_m9_gop_handoff.py')]


def test(log: Path) -> int:
    with log.open('w') as f:
        return subprocess.run(ORACLE, cwd=ROOT, stdout=f,
                              stderr=subprocess.STDOUT, check=False).returncode


def main() -> int:
    original = SOURCE.read_bytes()
    assert original.count(NEEDLE) == 1 and MUTANT not in original
    assert EFI.is_file() and ESP.is_file(), 'build shipping EFI/ESP before RED'
    artifacts = {p: p.read_bytes() for p in (EFI, ESP)}
    digest = hashlib.sha256(original).hexdigest()
    out = arena_env.build_dir()
    red = False
    try:
        SOURCE.write_bytes(original.replace(NEEDLE, MUTANT))
        log = out / 'm9-font-red-guest.log'
        rc = test(log)
        observed = log.read_text()
        red = (rc != 0
               and '((21, 20), (34, 51, 85), (248, 238, 204))' in observed
               and 'AssertionError' in observed
               and '[displayd] ring3 GOP pixels ready' not in observed)
    finally:
        SOURCE.write_bytes(original)
        for p, data in artifacts.items():
            p.write_bytes(data)
    assert hashlib.sha256(SOURCE.read_bytes()).hexdigest() == digest
    assert all(p.read_bytes() == data for p, data in artifacts.items()), 'mutant EFI escaped'
    green = test(out / 'm9-font-restored-guest.log') == 0
    assert hashlib.sha256(SOURCE.read_bytes()).hexdigest() == digest
    # mtest.build rebuilds the EFI/ESP even when unchanged; restore the
    # original exact artifacts so the RED test never changes what ships.
    for p, data in artifacts.items():
        p.write_bytes(data)
    assert all(p.read_bytes() == data for p, data in artifacts.items())
    print(f'[m9-font] real QMP missing-glyph RED: {"PASS" if red else "FAIL"}; '
          f'restored exact source/EFI real QMP GREEN: {"PASS" if green else "FAIL"}')
    return 0 if red and green else 1


if __name__ == '__main__':
    sys.exit(main())
