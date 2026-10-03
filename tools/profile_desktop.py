#!/usr/bin/env python3
"""Interactive-latency profile of the real desktop (ARENA_PERF image).

Builds the production desktop with opt-in `[perf ...]` probes, boots it,
drives real QMP tablet/keyboard input through fixed scenarios and reports,
per scenario, the guest-measured compositor and client timings plus a
host-measured pointer motion-to-photon figure. Not a qualification gate:
TCG absolute times are inflated; compare runs on the same host only.

Usage: python3 tools/profile_desktop.py [--label NAME] [--json OUT]
"""
import argparse
import json
import os
import re
import time
import arena_env
import mtest
from test_m10_apps import Desktop

BUILD = arena_env.build_dir()
PERF = re.compile(r'\[perf (desktop|app\d)\]((?: \w+=\d+/\d+/\d+)+)')


def parse(text):
    """Aggregate `[perf]` lines: name -> {count, total_us, max_us}."""
    out = {}
    for who, body in PERF.findall(text):
        for name, n, mean, mx in re.findall(r'(\w+)=(\d+)/(\d+)/(\d+)', body):
            key = f'{who}.{name}'
            n, mean, mx = int(n), int(mean), int(mx)
            agg = out.setdefault(key, {'count': 0, 'total_us': 0, 'max_us': 0})
            agg['count'] += n
            agg['total_us'] += n * mean
            agg['max_us'] = max(agg['max_us'], mx)
    for agg in out.values():
        agg['mean_us'] = agg['total_us'] // agg['count'] if agg['count'] else 0
    return out


def workflow(label, results):
    d = Desktop(label)
    serial = BUILD / f'serial-{label}.log'
    def mark():
        return len(serial.read_bytes())
    def phase(name, body):
        time.sleep(1.3)  # flush the previous reporting window
        start, t0 = mark(), time.monotonic()
        info = body() or {}
        host_s = time.monotonic() - t0
        time.sleep(1.3)
        text = serial.read_bytes()[start:].decode('utf-8', 'replace')
        results[name] = {'host_s': round(host_s, 3), **info, 'guest': parse(text)}
        print(f'[profile] {name}: host {host_s:.2f}s', flush=True)
    def sweep(n, y0, y1):
        # Back-to-back absolute moves, as fast as QMP accepts them.
        for i in range(n):
            x = 120 + (i * 37) % 560
            y = y0 + (i * 23) % (y1 - y0)
            d.point(x, y)
        d.point(780, 500)
        return {'events': n}
    def cursor_latency(trials):
        # Host motion-to-photon: move, then dump the scanout back-to-back
        # (no sleep) until the arrow tip is drawn at the target over the
        # uniform empty desktop. Resolution is one screendump.
        dump = BUILD / f'{label}-dump.ppm'
        def pixels():
            d.q.command('screendump', filename=str(dump), format='ppm')
            raw = dump.read_bytes()
            return raw[re.match(rb'P6\s+\d+\s+\d+\s+255\s', raw).end():]
        samples = []
        for i in range(trials):
            x, y = 640 + (i % 2) * 40, 420 + (i % 3) * 30
            t0 = time.monotonic()
            d.point(x, y)
            while True:
                p = pixels()
                px = lambda a, b: p[(b * 800 + a) * 3:(b * 800 + a) * 3 + 3]
                if px(x - 1, y - 1) == px(x - 1, y + 5) != px(x + 5, y + 5):
                    break
                if time.monotonic() - t0 > 5:
                    raise AssertionError('cursor never drawn')
            samples.append((time.monotonic() - t0) * 1000)
            time.sleep(0.05)
        t0 = time.monotonic()
        for _ in range(trials):
            pixels()
        dump_ms = (time.monotonic() - t0) * 1000 / trials
        samples.sort()
        return {'motion_to_photon_ms': {'median': round(samples[len(samples) // 2], 1),
                                        'max': round(samples[-1], 1),
                                        'screendump_ms': round(dump_ms, 1)}}
    def typing_latency(trials):
        # Host key-to-photon in the focused Terminal: type one character
        # and dump until the input line's raster changes.
        dump = BUILD / f'{label}-dump.ppm'
        def region():
            d.q.command('screendump', filename=str(dump), format='ppm')
            raw = dump.read_bytes()
            p = raw[re.match(rb'P6\s+\d+\s+\d+\s+255\s', raw).end():]
            # Terminal at (70,60): input line local y 240..264.
            return b''.join(p[((60 + 244 + r) * 800 + 82) * 3:((60 + 244 + r) * 800 + 482) * 3] for r in range(16))
        samples = []
        for i in range(trials):
            before = region()
            t0 = time.monotonic()
            d.q.key('abcdefghij'[i % 10])
            while region() == before:
                if time.monotonic() - t0 > 5:
                    raise AssertionError('typed glyph never drawn')
            samples.append((time.monotonic() - t0) * 1000)
            time.sleep(0.05)
        samples.sort()
        return {'key_to_photon_ms': {'median': round(samples[len(samples) // 2], 1),
                                     'max': round(samples[-1], 1)}}
    try:
        phase('idle-empty', lambda: time.sleep(3))
        phase('cursor-latency', lambda: cursor_latency(15))
        phase('pointer-empty', lambda: sweep(150, 60, 520))
        phase('launch-terminal', lambda: (d.launch(0, 'p-terminal'), {})[1])
        phase('typing-terminal', lambda: (d.q.type_text('echo the quick brown fox jumps\r', gap_s=.03), {'events': 31})[1])
        phase('key-latency', lambda: typing_latency(15))
        def launch_rest():
            for kind in range(1, 6):
                d.launch(kind, f'p-app-{kind}', kind)
        phase('launch-five-more', launch_rest)
        phase('idle-six-apps', lambda: time.sleep(3))
        phase('pointer-six-apps', lambda: sweep(150, 60, 520))
        def drag():
            # Gallery (top, index 5) title strip at (200+20, 180+10).
            d.point(220, 190, True)
            for i in range(60):
                d.point(220 - i * 2, 190 - i)
            d.point(100, 130, False)
            d.point(780, 500)
            return {'events': 62}
        phase('drag-six-apps', drag)
        for n in range(6):
            d.q.command('input-send-event', events=[d.q._ev('f8', True), d.q._ev('f8', False)])
            d.wait(lambda: d.serial().count('[desktop] application retired:') >= n + 1,
                   'F8 did not retire the focused application')
        return b'shutdown\r'
    finally:
        d.dispose()


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--label', default='profile')
    ap.add_argument('--json')
    args = ap.parse_args()
    os.environ['ARENA_PERF'] = '1'
    try:
        esp = mtest.build(args.label, desktop=True)
    finally:
        os.environ.pop('ARENA_PERF', None)
    results = {}
    rc, _, _ = mtest.boot(args.label, esp,
                          [((b'[desktop] real desktop frame presented', b'arena>'), 1,
                            lambda: workflow(args.label, results))],
                          arena_env.make_scratch_disk(), pointer=True, timeout_s=300)
    assert rc == 0, rc
    text = json.dumps(results, indent=1)
    if args.json:
        open(args.json, 'w').write(text + '\n')
    for name, r in results.items():
        g = r['guest']
        pick = lambda k: g.get(k, {})
        print(f"{name:18} host={r['host_s']:6.2f}s "
              + ' '.join(f"{k.split('.')[1]}={pick(k).get('count', 0)}x{pick(k).get('mean_us', 0)}us(max{pick(k).get('max_us', 0)})"
                         for k in ('desktop.render', 'desktop.present', 'desktop.input2frame', 'desktop.damage')
                         if k in g)
              + (f" m2p={r['motion_to_photon_ms']}" if 'motion_to_photon_ms' in r else '')
              + (f" k2p={r['key_to_photon_ms']}" if 'key_to_photon_ms' in r else ''))
        apps = {k: v for k, v in g.items() if k.startswith('app')}
        for k in sorted(apps):
            if k.endswith(('paint', 'event2damage')):
                print(f"{'':18}   {k}={apps[k]['count']}x{apps[k]['mean_us']}us(max{apps[k]['max_us']})")
    print('[profile] PASS (measurement only; not a qualification gate)')


if __name__ == '__main__':
    main()
