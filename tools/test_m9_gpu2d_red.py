#!/usr/bin/env python3
"""Host-only GPU command ACK matcher RED/GREEN; no device claim.

Omit a real production response type check, require its exact unit test to
reject the mutant, then restore source and check the full no_std codec gate.
The independent guest GPU service and QMP pixels remain unimplemented.
"""
import hashlib
from pathlib import Path
import subprocess
import sys

import arena_env

ROOT = Path(__file__).resolve().parent.parent
SRC = ROOT / 'userspace/gpu2d/src/lib.rs'
MANIFEST = ROOT / 'userspace/gpu2d/Cargo.toml'
NEEDLE = b'    if get32(input, 0) != expected_kind {\n        return Err(WireError::WrongResponse);\n    }'
MUTANT = b'    let _ = expected_kind; // RED: accept an unexpected device ACK'


def main() -> int:
    original = SRC.read_bytes()
    assert original.count(NEEDLE) == 1 and MUTANT not in original
    source_hash = hashlib.sha256(original).hexdigest()
    artifacts = [ROOT / 'build/arena-boot.efi', ROOT / 'build/arena-esp.img']
    artifact_hashes = {p: hashlib.sha256(p.read_bytes()).hexdigest()
                       for p in artifacts if p.exists()}
    bdir = arena_env.build_dir()
    log = bdir / 'm9-gpu2d-ack-red.log'
    red = False
    try:
        SRC.write_bytes(original.replace(NEEDLE, MUTANT))
        with log.open('w') as f:
            rc = subprocess.run([
                'cargo', 'test', '--offline', '--locked', '--manifest-path', str(MANIFEST),
                '--lib', '--target', 'x86_64-unknown-linux-gnu',
                'hostile_geometry_and_stale_response_lengths_refuse',
            ], cwd=ROOT, env=arena_env.rust_env(), stdout=f,
                stderr=subprocess.STDOUT, check=False).returncode
        output = log.read_text()
        red = (rc != 0 and 'left: Ok(())' in output
               and 'right: Err(WrongResponse)' in output
               and 'hostile_geometry_and_stale_response_lengths_refuse' in output)
    finally:
        SRC.write_bytes(original)
    assert hashlib.sha256(SRC.read_bytes()).hexdigest() == source_hash
    with (bdir / 'm9-gpu2d-ack-green.log').open('w') as f:
        green = subprocess.run([sys.executable, str(ROOT / 'tools/test_m9_gpu2d_wire.py')],
                               cwd=ROOT, stdout=f, stderr=subprocess.STDOUT).returncode == 0
    assert hashlib.sha256(SRC.read_bytes()).hexdigest() == source_hash
    assert all(hashlib.sha256(p.read_bytes()).hexdigest() == digest
               for p, digest in artifact_hashes.items()), 'host control altered EFI/ESP'
    print(f'[m9-gpu2d-ack] production ACK matcher RED host control: {"PASS" if red else "FAIL"}; '
          f'exact source/no_std codec GREEN: {"PASS" if green else "FAIL"}; no GPU guest claim')
    return 0 if red and green else 1


if __name__ == '__main__':
    sys.exit(main())
