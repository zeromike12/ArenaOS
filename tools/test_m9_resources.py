#!/usr/bin/env python3
"""Real guest Phase-9 resource high-water and original-child teardown.

The Power-only shell measures live frame/record/process counters before and
after the focused second window exits, while the compositor/root separately
report bounded SharedRegion refs, maps, pages and occupied capability slots.
No source-file estimate is substituted for a guest snapshot.
"""
import re
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import arena_env
import mtest
import qmp

LABEL = 'm9-resources'


def kill_second() -> bytes:
    conn = qmp.Qmp(str(arena_env.build_dir() / f'qmp-{LABEL}.sock'))
    try:
        conn.key('x')
    finally:
        conn.close()
    return b''


def main():
    esp = mtest.build(LABEL)
    feed = [
        ((b'[window_b] held-cap focused surface painted',
          b'[window_a] forged input token refused',
          b'servicemgr: packaged READY (full boot scan; exact PING + exit + deadline)',
          b'arena>'), 1, b'pkg resources\r'),
        (b'pkg: observed frames=', 1, kill_second),
        ((b'graphics original child 1 retired:',
          b'[compositord] dead original child retired and underlying pixels presented'),
         1, b'\x08pkg resources\r'),
        (b'pkg: observed frames=', 2, b'shutdown\r'),
    ]
    rc, serial, _ = mtest.boot(LABEL, esp, feed, arena_env.make_scratch_disk(), timeout_s=60)
    samples = [tuple(map(int, row)) for row in re.findall(
        r'pkg: observed frames=(\d+) records=(\d+) processes=(\d+)', serial)]
    assert rc == 0 and 'm7: RESULT PASS (2/2)' in serial and len(samples) == 2, samples
    before, after = samples
    assert before[1:] == (16, 16) and after[1:] == (15, 15), samples
    assert after[0] > before[0] + 19, samples  # freed region plus PTEs/stack
    assert re.search(r'compositor: live processes 15; .*shared 3/32, pages 507/20480, maps 6/64; free frames \d+', serial)
    assert re.search(r'graphics original child 1 retired: .*shared 2/32 runs, 488/20480 pages, 4/64 maps; compositor caps Some\(\((?:7|8), 64\)\)', serial)
    assert '[arena ERROR halt]' not in serial and 'PANIC' not in serial
    print(f'[m9-resources] guest Power high-water resident 16/16 then exact child retire 15/15; free frames {before[0]}->{after[0]}; '
          'shared 3/32->2/32, 507->488 pages, 6->4 maps, compositor caps 10/64->7..8/64 PASS')


if __name__ == '__main__':
    main()
