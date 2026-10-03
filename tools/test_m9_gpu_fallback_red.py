#!/usr/bin/env python3
"""Force rejection of GPU VERSION_1 negotiation; require real GOP QMP fallback.

Only the userspace required-features constant is mutated for the negative
build. It is restored byte-exact in finally. The separately booted restored
GPU-only image must still show pixels via the device, not GOP.
"""
import re
import sys
from pathlib import Path
sys.path.insert(0, str(Path(__file__).resolve().parent))
import arena_env, mtest, qmp

ROOT = Path(__file__).resolve().parent.parent
SRC = ROOT / 'userspace/displayd/src/gpu.rs'
OLD = b'virtio::FEATURE_VERSION_1, 1)'
NEW = b'virtio::FEATURE_VERSION_1 | 0x8000_0000, 1)'
LABEL = 'm9-gpu-fallback-reject'


def capture():
    path = arena_env.build_dir() / f'{LABEL}.ppm'
    connection = qmp.Qmp(str(arena_env.build_dir() / f'qmp-{LABEL}.sock'), connect_timeout_s=3)
    try:
        connection.command('screendump', filename=str(path), format='ppm')
    finally:
        connection.close()
    assert path.is_file()
    return b'shutdown\r'


def pixels(path):
    ppm = path.read_bytes()
    h = re.match(rb'P6\s+(\d+)\s+(\d+)\s+255\s', ppm)
    assert h and (int(h[1]), int(h[2])) == (800,600)
    assert len(ppm) - h.end() == 800*600*3
    for xy, expected in [((0,0),b'\x22\x33\x55'),
                         ((21,20),b'\xf8\xee\xcc'),
                         ((799,599),b'\x3b\x67\xe1')]:
        at = h.end() + (xy[1]*800 + xy[0])*3
        assert ppm[at:at+3] == expected, (xy,ppm[at:at+3],expected)


def main():
    original = SRC.read_bytes()
    assert original.count(OLD) == 1
    try:
        SRC.write_bytes(original.replace(OLD,NEW))
        esp = mtest.build(LABEL)
        rc, serial, _ = mtest.boot(LABEL,esp,
            [(b'[displayd] ring3 GOP pixels ready',1,capture)],
            arena_env.make_scratch_disk(),video='gpu+std')
        assert rc == 0 and 'm7: RESULT PASS (2/2)' in serial
        assert 'GPU fallback reason' in serial
        assert 'ring3 virtio-gpu 2D pixels ready' not in serial
        assert '[arena ERROR halt]' not in serial
        pixels(arena_env.build_dir() / f'{LABEL}.ppm')
    finally:
        SRC.write_bytes(original)
    assert SRC.read_bytes() == original
    # Build a new EFI from *exact* restored source; no screenshot from the
    # forced-rejection build may be mistaken for GPU pixels.
    import test_m9_gpu_pixels
    test_m9_gpu_pixels.main()
    assert SRC.read_bytes() == original
    print('[m9-gpu-fallback-reject] real QMP GOP fallback after forced pre-command VERSION_1 refusal; exact restored GPU-only device/QMP image PASS')


if __name__ == '__main__':
    main()
