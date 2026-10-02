#!/usr/bin/env python3
"""Phase-9 GOP pixels and bounded SharedRegion guest smoke proof.

The framebuffer screen is captured via QMP *after* userspace displayd has
mapped its exclusive Mmio cap and painted fixed bounded bars. Its prior
SharedRegion smoke includes zeroed pages, rights, copies and mapping pins.
This is not yet a compositor, shared teardown, input or virtio-gpu proof.
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
    rc, s, elapsed = mtest.boot(LABEL, esp,
                               [(b'[displayd] ring3 GOP pixels ready', 1, capture)], disk)
    marker = re.search(r'GOP handoff: (\d+)x(\d+) pitch=(\d+) format=(\d+) phys=0x([0-9a-f]+) bytes=(\d+)', s)
    assert rc == 0 and marker is not None and 'm7: RESULT PASS (2/2)' in s
    assert '[displayd] SharedRegion guest authority/zero/copy/mapping PASS' in s
    assert '[sharedprobe] capacity/rights/zero PASS' in s
    assert 'dead-process mapping/cap sweep frame-exact; RESULT PASS (1/1)' in s
    assert '[arena ERROR halt]' not in s and 'PANIC' not in s
    w,h,pitch,fmt,phys,length = (int(x, 16) if i==4 else int(x) for i,x in enumerate(marker.groups()))
    assert (w,h,pitch,fmt) == (800,600,800,1)
    assert phys % 4096 == 0 and length == pitch*h*4 and length <= 2*1024*1024
    ppm = (arena_env.build_dir()/f'{LABEL}.ppm').read_bytes()
    header = re.match(rb'P6\s+(\d+)\s+(\d+)\s+255\s',ppm)
    assert header and (int(header[1]),int(header[2]))==(w,h)
    assert len(ppm)-header.end() == w*h*3
    def pixel(x, y):
        pos = header.end() + (y*w+x)*3
        return tuple(ppm[pos:pos+3])
    # Colors originate from userspace/displayd/src/main.rs, not OVMF's
    # firmware splash/background; exact bytes across four distant regions.
    for xy, expected in [((0,0),(0x22,0x33,0x55)),
                         ((20,20),(0x22,0x33,0x55)),
                         ((21,20),(0xf8,0xee,0xcc)),  # bitmap 'A' actual pixel
                         ((0,100),(0xe3,0x35,0x42)),
                         ((400,100),(0x2e,0xc7,0x71)),
                         ((799,599),(0x3b,0x67,0xe1))]:
        assert pixel(*xy) == expected, (xy, pixel(*xy), expected)
    no_display = arena_env.make_scratch_disk()
    rc2, s2, _ = mtest.boot(f'{LABEL}-absent', esp,
                            [(b'arena>', 1, b'shutdown\r')], no_display, video='none')
    assert rc2 == 0 and 'GOP handoff: unavailable or unsupported mode' in s2
    assert '[displayd] ring3 GOP pixels ready' not in s2
    assert '[sharedprobe] capacity/rights/zero PASS' in s2
    assert 'dead-process mapping/cap sweep frame-exact; RESULT PASS (1/1)' in s2
    assert 'm7: RESULT PASS (2/2)' in s2 and '[arena ERROR halt]' not in s2
    print(f'[{LABEL}] QMP {w}x{h} matches GOP regions and linked toolkit font foreground/background; guest SharedRegion authority/zero/capacity + exact 512-page process teardown PASS; headless boot clean; no compositor, virtio-gpu or graphics input claim',flush=True)
if __name__=='__main__':main()
