#!/usr/bin/env python3
"""Phase-9 boot-only GOP handoff probe; NOT a graphics milestone proof.

No user display service is present yet. The actual virtual screen is
captured with QMP and its geometry compared to the independently captured
UEFI scalar handoff, without pretending this boot drew application pixels.
"""
import re
import sys
from pathlib import Path
sys.path.insert(0, str(Path(__file__).resolve().parent))
import arena_env, mtest, qmp

LABEL = 'm9-gop-handoff'

def capture() -> bytes:
    sock = arena_env.build_dir() / f'qmp-{LABEL}.sock'
    path = arena_env.build_dir() / f'{LABEL}.ppm'
    conn = qmp.Qmp(str(sock), connect_timeout_s=3)
    try:
        conn.command('screendump', filename=str(path), format='ppm')
    finally:
        conn.close()
    assert path.is_file()
    return b'shutdown\r'

def main():
    esp = mtest.build(LABEL)
    disk = arena_env.make_scratch_disk()
    rc, s, elapsed = mtest.boot(LABEL, esp, [(b'arena>', 1, capture)], disk)
    marker = re.search(r'GOP handoff: (\d+)x(\d+) pitch=(\d+) format=(\d+) phys=0x([0-9a-f]+) bytes=(\d+)', s)
    assert rc == 0 and marker is not None and 'm7: RESULT PASS (2/2)' in s
    assert '[arena ERROR halt]' not in s and 'PANIC' not in s
    w,h,pitch,fmt,phys,length = (int(x, 16) if i==4 else int(x) for i,x in enumerate(marker.groups()))
    assert (w,h,pitch,fmt) == (800,600,800,1)
    assert phys % 4096 == 0 and length == pitch*h*4 and length <= 2*1024*1024
    ppm = (arena_env.build_dir()/f'{LABEL}.ppm').read_bytes()
    header = re.match(rb'P6\s+(\d+)\s+(\d+)\s+255\s',ppm)
    assert header and (int(header[1]),int(header[2]))==(w,h)
    assert len(ppm)-header.end() == w*h*3
    no_display = arena_env.make_scratch_disk()
    rc2, s2, _ = mtest.boot(f'{LABEL}-absent', esp,
                            [(b'arena>', 1, b'shutdown\r')], no_display, video='none')
    assert rc2 == 0 and 'GOP handoff: unavailable or unsupported mode' in s2
    assert 'm7: RESULT PASS (2/2)' in s2 and '[arena ERROR halt]' not in s2
    print(f'[{LABEL}] real QMP display {w}x{h} agrees with pre-EBS GOP {pitch=}, {fmt=}, phys=0x{phys:x}; no-VGA boot reports absent without fake acceleration; both guest boots clean; no compositor/pixel claim',flush=True)
if __name__=='__main__':main()
