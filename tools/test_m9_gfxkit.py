#!/usr/bin/env python3
"""Host and bare-metal linked-toolkit contract + visual oracle RED/GREEN.

This is a static/no_std host gate, not proof that a compositor exists.
Real pixels on QEMU are asserted separately by test_m9_gop_handoff.py.
"""
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / 'tools'))
from check_phase9_pixels import verify, SAMPLES  # noqa: E402


def main():
    manifest = ROOT / 'userspace/gfxkit/Cargo.toml'
    for command in [
        ['cargo', 'test', '--offline', '--locked', '--manifest-path', str(manifest),
         '--lib', '--target', 'x86_64-unknown-linux-gnu'],
        ['cargo', 'build', '--offline', '--locked', '--manifest-path', str(manifest),
         '--release', '--target', 'x86_64-unknown-none'],
        ['rustfmt', '--edition', '2024', '--check', str(ROOT / 'userspace/gfxkit/src/lib.rs')],
    ]:
        subprocess.run(command, check=True, cwd=ROOT)
    w, h = 800, 600
    ppm = bytearray(f'P6\n{w} {h}\n255\n'.encode()) + bytearray(w*h*3)
    header = len(ppm) - w*h*3
    for (x, y), rgb in SAMPLES:
        offset = header + 3*(y*w+x)
        ppm[offset:offset+3] = bytes(rgb)
    assert verify(ppm)[:2] == (800, 600)
    x, y = SAMPLES[-1][0]
    ppm[header+3*(y*w+x)+2] ^= 1  # deliberate one-channel one-pixel RED
    try:
        verify(ppm)
    except ValueError as exc:
        assert 'wrong actual pixel' in str(exc)
    else:
        raise AssertionError('mutant PPM was accepted')
    print('[m9-gfxkit] 3/3 host contracts, bare-metal no_std build, pixel oracle GREEN and deliberate one-channel RED PASS')


if __name__ == '__main__':
    main()
