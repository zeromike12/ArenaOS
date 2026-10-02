#!/usr/bin/env python3
"""ADR-0061 real guest service-death RED / exact-restored QMP pixel GREEN.

The test-only mutation makes a receiver-verified QMP key terminate the
compositor while inputd is IN SYS_IPC_CALL. Root must fail that call through
ordinary process teardown and halt ERROR, not leave a stale healthy shell.
The exact production bytes are restored even if a red assertion fails.
"""
import hashlib
import re
import subprocess
import sys
from pathlib import Path

import arena_env
import mtest

ROOT = Path(__file__).resolve().parent.parent
SOURCE = ROOT / 'phase9-work/compositord-main.rs'
EFI, ESP = ROOT / 'build/arena-boot.efi', ROOT / 'build/arena-esp.img'
NEEDLE = b'&& state.route_verified_key(Key { ascii, pressed }).is_ok()'
MUTANT = (b'&& { if ascii == b\'q\' { die(77) } '
          b'state.route_verified_key(Key { ascii, pressed }).is_ok() }')
MARKER = b'[window_b] held-cap focused surface painted'


def build(path: Path) -> None:
    with path.open('w') as log:
        subprocess.run(['bash', 'tools/build.sh', '--image'], cwd=ROOT,
                       stdout=log, stderr=subprocess.STDOUT, check=True)


def main() -> int:
    original = SOURCE.read_bytes()
    assert original.count(NEEDLE) == 1 and MUTANT not in original
    bdir = arena_env.build_dir()
    build(bdir / 'm9-service-death-original-build.log')
    artifacts = {p: p.read_bytes() for p in (EFI, ESP)}
    source_sha = hashlib.sha256(original).hexdigest()
    red = False
    try:
        SOURCE.write_bytes(original.replace(NEEDLE, MUTANT))
        esp = mtest.build('m9-service-death-red')
        rc, serial, elapsed = mtest.run_qemu(
            'm9-service-death-red', esp, feed=[],
            keys=mtest.DEFAULT_KEYS + [(MARKER, 1, 'q')])
        (bdir / 'm9-service-death-red-serial.log').write_text(serial)
        print(f'[m9-service-death] mutant QEMU rc={rc} elapsed={elapsed:.1f}s')
        red = (rc != 0
               and 'm7: RESULT PASS (2/2)' in serial
               and MARKER.decode() in serial
               and 'graphics: original boot service died; IPC failed; fail-closed, no restart' in serial
               and '[arena ERROR halt]' in serial
               and re.search(r'destroy pid \d+: 1 in-flight call\(s\) answered STATUS_SERVICE_GONE', serial)
               and '[window_b] real key pixel painted' not in serial)
    finally:
        SOURCE.write_bytes(original)
        try:
            build(bdir / 'm9-service-death-restored-build.log')
        finally:
            for p, data in artifacts.items():
                p.write_bytes(data)
        assert hashlib.sha256(SOURCE.read_bytes()).hexdigest() == source_sha
        assert all(p.read_bytes() == data for p, data in artifacts.items()), 'mutant EFI escaped'
    with (bdir / 'm9-service-death-green.log').open('w') as log:
        green = subprocess.run([sys.executable, 'tools/test_m9_compositor_input.py'],
                               cwd=ROOT, stdout=log, stderr=subprocess.STDOUT).returncode == 0
    for p, data in artifacts.items():
        p.write_bytes(data)
    assert all(p.read_bytes() == data for p, data in artifacts.items())
    print(f'[m9-service-death] actual boot-root IPC fail-stop RED: {"PASS" if red else "FAIL"}; '
          f'byte-exact restored source/EFI and QMP key pixel GREEN: {"PASS" if green else "FAIL"}')
    return 0 if red and green else 1


if __name__ == '__main__':
    sys.exit(main())
