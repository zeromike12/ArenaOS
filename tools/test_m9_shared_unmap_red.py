#!/usr/bin/env python3
"""Guest RED/GREEN for exact own SharedRegion unmap retirement.

Omit only the real mapping-pin removal *after* PTE removal; the
independent stable-boundary registry/PTE walk must catch the stale map.
Restore the byte-exact source and EFI/ESP, then boot and verify QMP pixels.
"""
import hashlib
import subprocess
import sys
from pathlib import Path
sys.path.insert(0, str(Path(__file__).resolve().parent))
import arena_env, mtest

ROOT = Path(__file__).resolve().parent.parent
SOURCE = ROOT / 'kernel/kernel/src/arch/x86_64/syscall.rs'
EFI = ROOT / 'build/arena-boot.efi'
ESP = ROOT / 'build/arena-esp.img'
NEEDLE = b'    crate::shared::unpin_own_mapping(pid, va, id);\n    STATUS_OK'
MUTANT = b'    // RED ONLY: leave the mapping pin after clearing the PTE.\n    let _ = (pid, va, id);\n    STATUS_OK'


def build(path):
    with path.open('w') as output:
        subprocess.run(['bash', 'tools/build.sh', '--image'], cwd=ROOT,
                       stdout=output, stderr=subprocess.STDOUT, check=True)


def main():
    original = SOURCE.read_bytes()
    assert original.count(NEEDLE) == 1 and MUTANT not in original
    bdir = arena_env.build_dir()
    build(bdir / 'm9-unmap-original-build.log')
    artifacts = {p:p.read_bytes() for p in (EFI,ESP)}
    digest = hashlib.sha256(original).hexdigest()
    red = False
    try:
        SOURCE.write_bytes(original.replace(NEEDLE,MUTANT))
        esp = mtest.build('m9-unmap-omitted-pin')
        rc, serial, _ = mtest.boot('m9-unmap-omitted-pin',esp,[],
                                   arena_env.make_scratch_disk(),timeout_s=55)
        (bdir / 'm9-unmap-red-serial.log').write_text(serial)
        red = (rc != 0
               and 'SharedRegion map PTE missing' in serial
               and '[sharedprobe] exact own SharedRegion UNMAP 48x capless churn PASS' not in serial
               and '[displayd] ring3 GOP pixels ready' not in serial)
    finally:
        SOURCE.write_bytes(original)
        try:
            build(bdir / 'm9-unmap-restored-build.log')
        finally:
            for p,data in artifacts.items(): p.write_bytes(data)
        assert hashlib.sha256(SOURCE.read_bytes()).hexdigest() == digest
        assert all(p.read_bytes()==data for p,data in artifacts.items())
    try:
        with (bdir / 'm9-unmap-green.log').open('w') as output:
            green = subprocess.run([sys.executable,'tools/test_m9_gop_handoff.py'],
                                   cwd=ROOT,stdout=output,stderr=subprocess.STDOUT).returncode == 0
    finally:
        for p,data in artifacts.items(): p.write_bytes(data)
    assert all(p.read_bytes()==data for p,data in artifacts.items())
    print(f'[m9-unmap-hook] omitted-pin guest RED: {"PASS" if red else "FAIL"}; '
          f'exact restored source/EFI GOP QMP GREEN: {"PASS" if green else "FAIL"}')
    return 0 if red and green else 1


if __name__ == '__main__':
    sys.exit(main())
