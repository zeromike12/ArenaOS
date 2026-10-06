#!/usr/bin/env python3
"""Actual boot-child sudden exit: copied-region retirement and QMP uncover.

The second client gets a real virtio-key 'x' and exits without DESTROY.
Its old SharedRegion reference does not revive it. Root reaps its process,
compositor retires focus/queue/mapping and presents the uncovered base, and
root then drops its comparator only at exact refs/pins=(1,0). Another client
continues to poll, while the historical shell can still shut down cleanly.
"""
import re
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import arena_env
import mtest
import qmp

LABEL = 'm9-client-death'
BUILD = arena_env.build_dir()


def shot(name: str) -> bytes:
    path = BUILD / f'{LABEL}-{name}.ppm'
    conn = qmp.Qmp(str(BUILD / f'qmp-{LABEL}.sock'), connect_timeout_s=4)
    try:
        conn.command('screendump', filename=str(path), format='ppm')
        if name == 'before':
            conn.key('x')
    finally:
        conn.close()
    return b'' if name == 'before' else b'\x08shutdown\r'


def pixel(name: str, x: int, y: int) -> tuple[int, int, int]:
    data = (BUILD / f'{LABEL}-{name}.ppm').read_bytes()
    h = re.match(rb'P6\s+(\d+)\s+(\d+)\s+255\s', data)
    assert h and (int(h[1]), int(h[2])) == (800, 600)
    assert len(data) - h.end() == 800 * 600 * 3
    at = h.end() + 3 * (y * 800 + x)
    return tuple(data[at:at+3])


def main():
    esp = mtest.build(LABEL)
    rc, serial, _ = mtest.boot(LABEL, esp, [
        (b'[window_b] held-cap focused surface painted', 1, lambda: shot('before')),
        ((b'[window_b] original child exiting without DESTROY',
          b'[compositord] dead original child retired and underlying pixels presented',
          b'graphics original child 1 retired:', b'arena>'), 1, lambda: shot('after')),
    ], arena_env.make_scratch_disk(), timeout_s=60)
    assert rc == 0 and 'm7: RESULT PASS (2/2)' in serial
    assert re.search(
        r'graphics original child 1 retired: Process record, region refs, mappings and comparator cap conserved; '
        r'shared 2/80 runs, 488/36864 pages, 4/128 maps; compositor caps Some\(\((?:7|8), 128\)\)',
        serial), 'root did not prove exact Process/region/map cleanup (one live A IPC landing may add a transient cap)'
    assert '[arena ERROR halt]' not in serial and 'PANIC' not in serial
    assert pixel('before', 200, 190) == (0x3d, 0xcf, 0x7a)
    assert pixel('after', 200, 190) == (0xe3, 0x35, 0x42), pixel('after', 200, 190)
    for x, y in ((60, 70), (0, 100), (193, 160), (700, 300)):
        # (193,160) was window B's bitmap glyph, so after death it must
        # revert to the underlying window A only where A actually covers;
        # y=160 is inside A and the pixel becomes client A's color.
        before, after = pixel('before', x, y), pixel('after', x, y)
        if (x, y) == (193, 160):
            assert before == (0xf8, 0xee, 0xcc) and after == (0xbd, 0x53, 0x38)
        else:
            assert before == after, ((x, y), before, after)
    print('[m9-client-death] real original-child exit, exact Process/region/map/cap retirement and independent QMP base/window uncover PASS')


if __name__ == '__main__':
    main()
