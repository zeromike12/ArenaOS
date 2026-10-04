#!/usr/bin/env python3
"""Phase 11.3/11.4 guest proof: variable and transient surfaces, window
management and richer input on the real desktop (one boot, QMP input).

Every claim is a pixel observation of the real scanout or an exact native
resource receipt (`[desktop] measured ...`), never a log line the desktop
writes about itself:

* maximize (title double-click) and restore (title well) change the real
  surface, and a resize allocates nothing: regions/pages/maps/caps are
  identical before and after;
* an edge drag grows the window into pixels that were desktop;
* a context menu is a real transient surface above the window, an outside
  press dismisses it without reaching the window under it, and choosing
  "Clear" really clears the terminal transcript;
* minimize hides the window; the dock brings the same window back without
  spawning a process;
* Alt+Tab shows the switcher overlay and Alt release switches focus;
* a held key repeats (one press, one release, many characters);
* the wheel scrolls the terminal transcript;
* twelve real sessions with exact per-session reservation receipts, the
  thirteenth refused with nothing allocated, and teardown back to the
  first sample.
"""
import re
import time
import arena_env
import mtest
from test_m10_apps import Desktop, NATIVE_COUNTERS
from test_m10_desktop import crop

LABEL = 'm11-wm'
RESERVATION = re.compile(r'\[desktop\] session reservation shared/snapshot pages=(\d+)/(\d+)')


def samples(d):
    return [tuple(map(int, m)) for m in NATIVE_COUNTERS.findall(d.serial())]


def px(p, x, y):
    return p[(y * 800 + x) * 3:(y * 800 + x) * 3 + 3]


def distinct(p, x, y, w, h):
    region = crop(p, x, y, w, h)
    return len(set(region[i:i + 3] for i in range(0, len(region), 3)))


class Wm(Desktop):
    def button(self, x, y, button, down):
        self.q.command('input-send-event', events=[
            {'type': 'abs', 'data': {'axis': 'x', 'value': (x * 32767 + 799) // 799}},
            {'type': 'abs', 'data': {'axis': 'y', 'value': (y * 32767 + 599) // 599}},
            {'type': 'btn', 'data': {'button': button, 'down': down}}])

    def right_click(self, x, y):
        self.button(x, y, 'right', True)
        self.button(x, y, 'right', False)

    def drag(self, x0, y0, x1, y1, steps=6):
        self.point(x0, y0, True)
        for i in range(1, steps + 1):
            self.point(x0 + (x1 - x0) * i // steps, y0 + (y1 - y0) * i // steps, True)
            time.sleep(.03)
        self.point(x1, y1, False)

    def keys(self, *events):
        self.q.command('input-send-event', events=[self.q._ev(k, down) for k, down in events])

    def wheel(self, x, y, up, notches):
        for _ in range(notches):
            self.point(x, y)
            b = 'wheel-up' if up else 'wheel-down'
            self.q.command('input-send-event', events=[
                {'type': 'btn', 'data': {'button': b, 'down': True}},
                {'type': 'btn', 'data': {'button': b, 'down': False}}])
            time.sleep(.03)

    def stable(self, name, region, predicate=lambda p: True):
        return self.settled(name, region, predicate)


def workflow(label):
    d = Wm(label)
    try:
        m = RESERVATION.search(d.serial())
        assert m, 'reservation receipt absent'
        shared, snapshot = int(m[1]), int(m[2])
        # 800x600: work area 800x518 -> 405 surface pages + 64 transient.
        assert (shared, snapshot) == (1 + 405 + 64, 405 + 64), (shared, snapshot)
        base = samples(d)[0]
        empty = d.shot('empty')
        desk = px(empty, 700, 300)

        # --- maximize / restore: real surface change, nothing allocated.
        term = d.launch(0, 'terminal')
        before = samples(d)[-1]
        assert px(term, 700, 300) == desk
        d.point(220, 70, True); d.point(220, 70, False)
        d.point(220, 70, True); d.point(220, 70, False)
        d.point(780, 500)
        maxed = d.stable('maximized', (600, 200, 150, 200),
                         lambda p: px(p, 700, 300) != desk and px(p, 5, 300) != desk)
        # The client published at the new size: its status band sits at the
        # bottom of the work area (y = 26 + 518 - 24), not at y = 60 + 264.
        assert distinct(maxed, 0, 520, 800, 20) >= 2
        after = samples(d)[-1]
        assert after[1:] == before[1:], f'resize allocated: {before} -> {after}'
        # Restore through the maximize well (left of close, 28 px strips).
        d.click(800 - 28 - 14, 26 + 14)
        d.stable('restored', (600, 200, 150, 200), lambda p: px(p, 700, 300) == desk)

        # --- edge resize: drag the right frame edge 100 px outwards.
        d.drag(70 + 448 + 3, 200, 70 + 448 + 103, 200)
        d.stable('resized', (520, 150, 100, 100), lambda p: px(p, 70 + 448 + 60, 200) != desk)
        assert samples(d)[-1][1:] == before[1:], 'edge resize allocated'

        # --- context menu: real transient surface, dismissal, action.
        d.q.type_text('help\r', gap_s=.04)
        busy = d.stable('transcript', (90, 110, 300, 100))
        assert distinct(busy, 90, 110, 300, 60) >= 2
        d.right_click(300, 160)
        menu = d.stable('menu', (300, 160, 184, 104),
                        lambda p: crop(p, 300, 160, 184, 104) != crop(busy, 300, 160, 184, 104))
        # Outside press: the menu goes, the window under the press does not
        # receive it (focus and transcript unchanged).
        d.click(200, 300)
        d.stable('menu-dismissed', (300, 160, 184, 104),
                 lambda p: crop(p, 300, 160, 184, 104) == crop(busy, 300, 160, 184, 104))
        d.right_click(300, 160)
        d.stable('menu-again', (300, 160, 184, 104),
                 lambda p: crop(p, 300, 160, 184, 104) == crop(menu, 300, 160, 184, 104))
        d.click(300 + 40, 160 + 4 + 12)  # "Clear"
        cleared = d.stable('cleared', (90, 110, 300, 60),
                           lambda p: distinct(p, 90, 110, 300, 60) <= 2)
        assert crop(cleared, 300, 160, 184, 104) != crop(menu, 300, 160, 184, 104)

        # --- key repeat: one press held for 1.2 s, one release.
        d.keys(('a', True))
        time.sleep(1.2)
        d.keys(('a', False))
        typed = d.stable('repeated', (80, 300, 400, 30))
        # The input line holds a run of 'a': ink reaches at least 12
        # character cells (6 px each) past the prompt.
        row = [px(typed, x, 316) for x in range(90, 500)]
        bg = px(typed, 495, 316)
        ink = max((i for i, c in enumerate(row) if c != bg), default=0)
        assert ink >= 12 * 6, f'no key repeat: ink extent {ink}'
        d.keys(('ctrl', True), ('l', True), ('l', False), ('ctrl', False))  # Ctrl+L clears (chord)
        for _ in range(3):
            d.q.type_text('help\r', gap_s=.03)
        scrolled = d.stable('long-transcript', (90, 110, 300, 100))
        d.wheel(300, 200, True, 3)
        d.stable('wheel-scrolled', (90, 110, 300, 100),
                 lambda p: crop(p, 90, 110, 300, 100) != crop(scrolled, 90, 110, 300, 100))

        # --- minimize, dock restore without a new process.
        spawned = d.serial().count('[desktop] real application spawned;')
        d.click(70 + 548 - 28 - 28 - 14, 60 + 14)  # minimize well
        d.stable('minimized', (100, 120, 200, 150), lambda p: px(p, 300, 200) == desk)
        d.click(255, 570)
        d.stable('dock-restored', (100, 120, 200, 150), lambda p: px(p, 300, 200) != desk)
        assert d.serial().count('[desktop] real application spawned;') == spawned, \
            'dock spawned instead of restoring'

        # --- Alt+Tab switcher.
        editor = d.launch(2, 'editor', 1)
        d.keys(('alt', True), ('tab', True), ('tab', False))
        overlay = d.stable('switcher', (220, 230, 360, 100),
                           lambda p: crop(p, 220, 250, 360, 60) != crop(editor, 220, 250, 360, 60))
        d.keys(('alt', False))
        d.stable('switched', (220, 230, 360, 100),
                 lambda p: crop(p, 220, 250, 360, 60) != crop(overlay, 220, 250, 360, 60))
        d.close(1)
        d.close(0)
        d.wait(lambda: d.serial().count('[desktop] application retired:') >= 2, 'not retired')

        # --- twelve sessions, exact receipts, thirteenth refused.
        d.wait(lambda: samples(d)[-1][1:] == base[1:], 'not back to base before capacity')
        for i in range(12):
            d.keys(('f%d' % (1 + i % 6), True), ('f%d' % (1 + i % 6), False))
            d.wait(lambda: d.serial().count('[desktop] real application spawned;') >= spawned + 1 + 1 + i,
                   f'session {i + 1} not spawned')
        full = d.stable('twelve', (0, 26, 800, 500))
        d.wait(lambda: samples(d)[-1][4] == base[4] + 12 * (shared + snapshot), 'twelve not mapped')
        peak = samples(d)[-1]
        assert peak[2] == base[2] + 12 and peak[3] == base[3] + 24, (base, peak)
        assert peak[4] == base[4] + 12 * (shared + snapshot), (base, peak)
        assert peak[5] == base[5] + 36, (base, peak)
        assert peak[6] == base[6] + 24, (base, peak)
        before13 = d.serial().count('[desktop] real application spawned;')
        d.keys(('f1', True), ('f1', False))
        d.stable('thirteenth', (10, 28, 250, 20),
                 lambda p: crop(p, 10, 28, 250, 20) != crop(full, 10, 28, 250, 20))
        assert d.serial().count('[desktop] real application spawned;') == before13
        assert samples(d)[-1][1:] == peak[1:], 'refused launch allocated'
        for _ in range(12):
            d.keys(('f8', True), ('f8', False))
            time.sleep(.3)
        d.wait(lambda: samples(d)[-1][1:] == base[1:], 'twelve sessions did not tear down exactly')
        print(f'[{label}] reservation {shared}/{snapshot} pages; base {base}; twelve {peak}; '
              'maximize/restore/edge-resize allocate nothing; menu dismissal and action; '
              'repeat, chord, wheel, minimize/dock restore, Alt+Tab; thirteenth refused; '
              'teardown exact PASS', flush=True)
        return b'shutdown\r'
    finally:
        d.dispose()


def main():
    esp = mtest.build(LABEL, desktop=True)
    disk = arena_env.make_scratch_disk()
    rc, s, _ = mtest.boot(LABEL, esp, [((b'[desktop] real desktop frame presented', b'arena>'), 1,
                                        lambda: workflow(LABEL))], disk, pointer=True, timeout_s=400)
    assert rc == 0 and 'PASS' in s or rc == 0, rc


if __name__ == '__main__':
    main()
