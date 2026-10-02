#!/usr/bin/env python3
"""ADR-0060 live guest: virtio-gpu-only owned windows and injected QMP key."""
import re
import sys
from pathlib import Path
sys.path.insert(0, str(Path(__file__).resolve().parent.parent / 'tools'))
import arena_env
import mtest
import qmp

LABEL = 'm9-gpu-compositor-input'
BUILD = arena_env.build_dir()

def picture(name: str) -> bytes:
    path = BUILD / f'{LABEL}-{name}.ppm'
    conn = qmp.Qmp(str(BUILD / f'qmp-{LABEL}.sock'), connect_timeout_s=4)
    try:
        conn.command('screendump', filename=str(path), format='ppm')
        if name == 'before':
            conn.key('q')  # real virtio-input device, NOT serial/host pixel edit
    finally:
        conn.close()
    return b'' if name == 'before' else b'\x08shutdown\r'

def pixels(name: str):
    data = (BUILD / f'{LABEL}-{name}.ppm').read_bytes()
    h = re.match(rb'P6\s+(\d+)\s+(\d+)\s+255\s', data)
    assert h and (int(h[1]),int(h[2])) == (800,600)
    assert len(data)-h.end() == 800*600*3
    return lambda x,y: tuple(data[h.end()+(y*800+x)*3:h.end()+(y*800+x)*3+3])

def main():
    esp=mtest.build(LABEL)
    rc,serial,_=mtest.boot(LABEL,esp,[
        (b'[window_b] held-cap focused surface painted',1,lambda:picture('before')),
        ((b'[window_b] real key pixel painted',
          b'[window_a] forged input token refused',b'arena>'),1,
         lambda:picture('after')),
    ],arena_env.make_scratch_disk(), video='gpu')
    assert rc==0 and 'm7: RESULT PASS (2/2)' in serial
    assert '[displayd] ring3 virtio-gpu 2D pixels ready' in serial
    assert 'GOP handoff: unavailable or unsupported mode' in serial
    assert '[window_a] forged input token refused' in serial
    before,after=pixels('before'),pixels('after')
    assert before(60,70)==(0xbd,0x53,0x38) and before(190,160)==(0x3d,0xcf,0x7a)
    assert before(200,190)==(0x3d,0xcf,0x7a)
    # The owned 5x7 bitmap S in SECOND starts with 0b01111, not text drawn by QEMU.
    assert before(192,160)==(0x3d,0xcf,0x7a)
    assert before(193,160)==(0xf8,0xee,0xcc)
    assert after(200,190)==(0xff,0xbb,0x11)
    for xy in ((0,100),(700,300),(799,599),(60,70)):
        assert before(*xy)==after(*xy),(xy,before(*xy),after(*xy))
    print('[m9-gpu-compositor-input] virtio-gpu-only owned windows, z-order, preserved base, bitmap title and genuine QMP key after focused delivery: PASS')

if __name__=='__main__': main()
