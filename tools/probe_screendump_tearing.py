#!/usr/bin/env python3
"""Probe: are QMP screendumps atomic w.r.t. the running guest? (evidence tool)

Launch the Gallery (a window at 70,60). Repeatedly put the cursor inside the
window (A), wait until it is drawn, move it to the desktop (B), then take ONE
immediate screendump. Classify each immediate frame:
  both   - arrow at A and at B (stale-looking)
  b_only - arrow at B only (correct)
  a_only - arrow at A only (move not presented yet)
For every 'both' frame, re-dump 0.3 s later with no input: if A's arrow is gone,
the earlier frame was a torn capture of a correct framebuffer.
"""
import sys
import time
sys.path.insert(0, str(__import__('pathlib').Path(__file__).resolve().parent))
import arena_env
import mtest
from test_m10_apps import Desktop
from test_m10_desktop import crop

BUILD = arena_env.build_dir()
N = int(sys.argv[1]) if len(sys.argv) > 1 else 300
stats = {'both': 0, 'b_only': 0, 'a_only': 0, 'neither': 0, 'both_resolved': 0, 'both_persistent': 0}


def arrow(p, x, y):
    px = lambda a, b: p[(b * 800 + a) * 3:(b * 800 + a) * 3 + 3]
    return any(px(a, b) == px(a, b + 6) != px(a + 6, b + 6) for a in range(x - 1, x + 2) for b in range(y - 1, y + 2))


def workflow():
    d = Desktop('tear')
    try:
        d.launch(5, 'tear-gallery')
        clean = d.shot('tear-clean')
        for i in range(N):
            ax, ay = 120 + (i % 3) * 15, 170
            bx, by = 700 + (i % 2) * 20, 420 + (i % 2) * 20
            d.point(ax, ay)
            d.shot('tear-a', lambda p: arrow(p, ax, ay))
            d.point(bx, by)
            d.q.command('screendump', filename=str(BUILD / 'tear-now.ppm'), format='ppm')
            from test_m10_desktop import ppm
            p = ppm(BUILD / 'tear-now.ppm')
            a, b = arrow(p, ax, ay), arrow(p, bx, by)
            kind = 'both' if a and b else 'b_only' if b else 'a_only' if a else 'neither'
            stats[kind] += 1
            if kind == 'both':
                time.sleep(.3)
                q = d.shot('tear-later')
                if arrow(q, ax, ay):
                    stats['both_persistent'] += 1
                    d.q.command('screendump', filename=str(BUILD / f'tear-persistent-{i}.ppm'), format='ppm')
                else:
                    stats['both_resolved'] += 1
        print('[tear-probe]', stats, flush=True)
        return b'shutdown\r'
    finally:
        d.dispose()


esp = mtest.build('tear', desktop=True)
rc, s, _ = mtest.boot('tear', esp, [((b'[desktop] real desktop frame presented', b'arena>'), 1, workflow)],
                      arena_env.make_scratch_disk(), pointer=True, timeout_s=1800)
print('[tear-probe] rc', rc, stats)
