#!/usr/bin/env python3
"""Real ring-3 virtio-gpu 2D scanout, captured from QEMU's virtual display.

QMP pixels, not a serial readiness line or host-generated image, are the
oracle. QEMU advertises 800x600 so the 2-MiB SharedRegion bound is respected.
This is a transport milestone, NOT compositor/input qualification.
"""
import re
import sys
from pathlib import Path
sys.path.insert(0, str(Path(__file__).resolve().parent))
import arena_env, mtest, qmp

LABEL = 'm9-gpu-pixels'


def capture():
    path = arena_env.build_dir() / f'{LABEL}.ppm'
    session = qmp.Qmp(str(arena_env.build_dir() / f'qmp-{LABEL}.sock'), connect_timeout_s=3)
    try:
        session.command('screendump', filename=str(path), format='ppm')
    finally:
        session.close()
    assert path.is_file()
    return b'shutdown\r'


def main():
    esp = mtest.build(LABEL)
    rc, serial, _ = mtest.boot(LABEL, esp,
        [(b'[arena INFO  m9] displayprobe: cap-bearing MODE/PRESENT guest', 1, capture)],
        arena_env.make_scratch_disk(), video='gpu')
    assert rc == 0 and 'm7: RESULT PASS (2/2)' in serial
    assert '[displayd] SharedRegion guest authority/zero/copy/mapping PASS' in serial
    assert '[displayprobe] ring3 MODE/cap-refusal/PRESENT 80x drain PASS' in serial
    assert 'displayprobe: cap-bearing MODE/PRESENT guest; own record retired; shared/cap/PTE accounting conserved PASS' in serial
    assert re.search(r'displayd resources at parked boundary: shared 2/32 runs, 471/20480 pages, 2/64 maps; caps 6/64; free frames \d+; live processes \d+', serial)
    assert 'GOP handoff: unavailable or unsupported mode' in serial
    assert '[arena ERROR halt]' not in serial and 'PANIC' not in serial
    data = (arena_env.build_dir() / f'{LABEL}.ppm').read_bytes()
    header = re.match(rb'P6\s+(\d+)\s+(\d+)\s+255\s', data)
    assert header is not None and (int(header[1]), int(header[2])) == (800, 600)
    assert len(data) - header.end() == 800 * 600 * 3
    def pixel(x, y):
        at = header.end() + (y*800 + x)*3
        return tuple(data[at:at+3])
    for xy, expected in [((0,0),(0x22,0x33,0x55)),
                         ((20,20),(0x22,0x33,0x55)),
                         ((21,20),(0xf8,0xee,0xcc)),
                         ((0,100),(0xe3,0x35,0x42)),
                         ((400,100),(0x2e,0xc7,0x71)),
                         ((700,300),(0xa2,0x51,0xf4)),
                         ((799,599),(0x3b,0x67,0xe1))]:
        assert pixel(*xy) == expected, (xy,pixel(*xy),expected)
    # The same EFI against QEMU's default 1280x800 mode (4,096,000 bytes)
    # must reject the real device response; the registry caps a run at 512
    # pages. Never treat QEMU rc=0 for a kernel halt as a PASS.
    rc2, oversized, _ = mtest.boot(f'{LABEL}-oversize', esp,
        [(b'arena>', 1, b'shutdown\r')], arena_env.make_scratch_disk(),
        video='gpu-big')
    assert rc2 != 0 and '[arena ERROR halt]' in oversized
    assert 'GPU display-info malformed or exceeds bounded scanout' in oversized
    assert '[displayd] ring3 virtio-gpu 2D pixels ready' not in oversized
    print('[m9-gpu-pixels] GPU-only guest commands + QMP 800x600 bars/font PASS; real default 1280x800 refuses bounded scanout; compositor/input/final stability pending')


if __name__ == '__main__':
    main()
