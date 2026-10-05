#!/usr/bin/env python3
"""Phase 11.8: the Files explorer on real AFS2 objects, driven by real input.

Every operation is judged on the AFS2 bytes the host reads back from the
disk, not on pixels: new folder with inline naming, copy/paste into a
folder entered by double-click, history back, cut/paste through a sidebar
place, delete to the Trash with its restore record, multi-item drag onto a
folder (a free name when taken), restore from the Trash by context menu,
rename by keyboard, open in a real Editor by double-click (capability
offer) and an exact save, empty Trash. Sorting, the grid view and a change
made by another application (seen on focus) are judged on pixels. The
AFS1 originals stay byte-identical.
"""
import time
import afs1
import afs2
import arena_env
import mtest
import test_m84_stage as seed
from test_m10_apps import Desktop
from test_m10_desktop import crop

LABEL = 'm11-explorer'
BASE = arena_env.AFS2_BASE_SECTOR * 512
HOME = '/Users/user/'
SEED = {b'user-note': b'Aa file manager', b'user-full': b'f' * 64, b'user-bin': b'\x80binary'}
# Window 0 of the cascade at (70, 60), 448x288; sidebar 116 (explorer_view).
WX, WY = 70, 60
CONTENT_X, CONTENT_Y = WX + 117, WY + 67


def tree(disk):
    for _ in range(50):
        try:
            vol = afs2.Volume(disk.read_bytes()[BASE:])
            afs2.check(vol)
            return {k[len(HOME):]: v for k, v in afs2.walk(vol).items() if k.startswith(HOME)}
        except Exception:
            time.sleep(0.1)
    raise AssertionError('AFS2 region never mounted cleanly on the host')


def row(i):
    """Screen point inside list row `i` (name column)."""
    return CONTENT_X + 40, CONTENT_Y + 22 + 20 * i + 10


def place(i):
    """Screen point on sidebar place `i` (Home, Desktop, Documents, Trash)."""
    return WX + 30, WY + 68 + 8 + 22 * i + 11


def keys(d, *names, gap=.06):
    for n in names:
        d.q.command('input-send-event', events=[d.q._ev(n, True), d.q._ev(n, False)])
        time.sleep(gap)


def chord(d, *mods_and_key):
    *mods, key = mods_and_key
    d.q.command('input-send-event', events=[d.q._ev(m, True) for m in mods]
                + [d.q._ev(key, True), d.q._ev(key, False)] + [d.q._ev(m, False) for m in reversed(mods)])
    time.sleep(.1)


def press(d, x, y, button='left', down=True):
    d.q.command('input-send-event', events=[
        {'type': 'abs', 'data': {'axis': 'x', 'value': (x * 32767 + 799) // 799}},
        {'type': 'abs', 'data': {'axis': 'y', 'value': (y * 32767 + 599) // 599}},
        {'type': 'btn', 'data': {'button': button, 'down': down}}])


def click(d, x, y):
    press(d, x, y)
    press(d, x, y, down=False)
    time.sleep(.12)


def double(d, x, y):
    press(d, x, y)
    press(d, x, y, down=False)
    press(d, x, y)
    press(d, x, y, down=False)
    time.sleep(.2)


def right(d, x, y):
    press(d, x, y, 'right')
    press(d, x, y, 'right', down=False)
    time.sleep(.3)


def until(d, disk, predicate, what):
    end = time.monotonic() + 15
    while time.monotonic() < end:
        t = tree(disk)
        if predicate(t):
            return t
        time.sleep(.15)
    d.q.command('screendump', filename=str(arena_env.build_dir() / f'{LABEL}-failed.ppm'), format='ppm')
    raise AssertionError(what + ': ' + repr(sorted(tree(disk))))


def content(p):
    return crop(p, CONTENT_X, CONTENT_Y, 300, 190)


def workflow(disk):
    d = Desktop(LABEL)
    try:
        d.wait(lambda: 'AFS2 file service online' in d.serial(), 'no AFS2 session')
        d.launch(1, 'files')
        # Documents, folders first then names: user-bin, user-full, user-note.
        chord(d, 'ctrl', 'shift', 'n')
        until(d, disk, lambda t: t.get('Documents/New Folder') is None and 'Documents/New Folder' in t,
              'Ctrl+Shift+N made no folder')
        time.sleep(.3)
        d.q.type_text('Projects\r', gap_s=.05)
        until(d, disk, lambda t: 'Documents/Projects' in t and 'Documents/New Folder' not in t,
              'inline name did not rename the new folder')
        # Rows: Projects, user-bin, user-full, user-note.
        click(d, *row(3))
        chord(d, 'ctrl', 'c')
        double(d, *row(0))
        time.sleep(.3)
        chord(d, 'ctrl', 'v')
        until(d, disk, lambda t: t.get('Documents/Projects/user-note') == b'Aa file manager',
              'copy/paste into the double-clicked folder failed')
        chord(d, 'alt', 'left')
        time.sleep(.3)
        click(d, *row(2))  # user-full
        chord(d, 'ctrl', 'x')
        click(d, *place(1))  # Desktop
        time.sleep(.3)
        chord(d, 'ctrl', 'v')
        until(d, disk, lambda t: t.get('Desktop/user-full') == b'f' * 64 and 'Documents/user-full' not in t,
              'cut/paste through the Desktop place failed')
        click(d, *place(2))  # Documents: Projects, user-bin, user-note
        time.sleep(.3)
        click(d, *row(1))
        keys(d, 'delete')
        t = until(d, disk, lambda t: t.get('.Trash/user-bin') == b'\x80binary' and 'Documents/user-bin' not in t,
                  'Delete did not move to the Trash')
        assert t.get('.Trash/.restore/user-bin') == b'Documents/user-bin', t.get('.Trash/.restore/user-bin')
        # Documents: Projects, user-note. Drag the note onto Projects; the
        # copy there already holds the name, so it lands as "user-note 2".
        x, y = row(1)
        tx, ty = row(0)
        press(d, x, y)
        d.point(x + 8, y - 6)
        time.sleep(.05)
        d.point(tx, ty)
        time.sleep(.1)
        d.point(tx, ty, False)
        until(d, disk, lambda t: t.get('Documents/Projects/user-note 2') == b'Aa file manager'
              and 'Documents/user-note' not in t, 'drag onto a folder did not move')
        # Restore from the Trash by its context menu (keyboard: first item).
        click(d, *place(3))
        time.sleep(.3)
        click(d, *row(0))
        right(d, *row(0))
        keys(d, 'down', 'ret')
        until(d, disk, lambda t: t.get('Documents/user-bin') == b'\x80binary' and '.Trash/user-bin' not in t
              and '.Trash/.restore/user-bin' not in t, 'Restore did not return the item')
        # Rename by keyboard: Documents rows Projects, user-bin.
        click(d, *place(2))
        time.sleep(.3)
        click(d, *row(0))
        chord(d, 'ctrl', 'r')
        time.sleep(.2)
        d.q.type_text('Work\r', gap_s=.05)
        until(d, disk, lambda t: 'Documents/Work/user-note' in t and 'Documents/Projects' not in t,
              'Ctrl+R rename failed')
        # Open in a real Editor by double-click: a capability offer.
        double(d, *row(0))
        time.sleep(.3)
        spawned = d.serial().count('[desktop] real application spawned;')
        double(d, *row(0))  # Work: user-note, user-note 2
        d.wait(lambda: d.serial().count('[desktop] real application spawned;') == spawned + 1,
               'double-click did not open the Editor')
        d.opened('editor', 1)
        d.q.type_text('X', gap_s=.05)
        chord(d, 'ctrl', 's')
        until(d, disk, lambda t: t.get('Documents/Work/user-note') == b'XAa file manager', 'Editor save missed')
        d.close(1)
        d.wait(lambda: d.serial().count('[desktop] application retired:') >= 1, 'editor not retired')
        click(d, WX + 200, WY + 14)  # focus Files again
        time.sleep(.2)
        # Trash the second note, then empty the Trash from its menu.
        click(d, *row(1))
        keys(d, 'delete')
        until(d, disk, lambda t: '.Trash/user-note 2' in t, 'second delete failed')
        click(d, *place(3))
        time.sleep(.3)
        right(d, CONTENT_X + 150, CONTENT_Y + 150)
        keys(d, 'down', 'ret')
        until(d, disk, lambda t: not any(k.startswith('.Trash/') for k in t), 'Empty Trash left items')
        # Sorting and the grid view (pixels): Documents holds Work, user-bin.
        click(d, *place(2))
        listed = d.settled('documents', (CONTENT_X, CONTENT_Y, 300, 190))
        click(d, CONTENT_X + 230, CONTENT_Y + 10)  # Size header
        d.shot('sorted-by-size', lambda p: content(p) != content(listed))
        click(d, WX + 448 - 12 - 13, WY + 46)  # grid toggle
        d.shot('grid', lambda p: content(p) != content(listed))
        click(d, WX + 448 - 12 - 26 - 2 - 13, WY + 46)  # back to the list
        # Another application changes Desktop; Files shows it on focus.
        click(d, *place(1))
        before = d.settled('desktop-before', (CONTENT_X, CONTENT_Y, 300, 190))
        d.launch(0, 'terminal', 1)
        d.q.type_text('put Desktop/fresh.txt hello\r', gap_s=.04)
        until(d, disk, lambda t: t.get('Desktop/fresh.txt') == b'hello', 'terminal put failed')
        click(d, WX + 200, WY + 14)
        d.shot('desktop-after-focus', lambda p: content(p) != content(before))
        return b'shutdown\r'
    finally:
        d.dispose()


def main(esp=None):
    if esp is None:
        esp = mtest.build(LABEL, desktop=True)
    disk = arena_env.make_scratch_disk()
    rc, s, _ = mtest.boot(LABEL + '-seed', esp, [(b'arena>', 1, b'shutdown\r')], disk, pointer=True)
    assert rc == 0
    seed.host_seed(disk, SEED)
    with open(disk, 'r+b') as f:
        f.truncate(arena_env.AFS2_DISK_MIB * 1024 * 1024)
    afs1_before = disk.read_bytes()[:arena_env.SCRATCH_MIB * 1024 * 1024]
    rc, s, _ = mtest.boot(LABEL, esp, [((b'[desktop] real desktop frame presented', b'arena>', b'filesd: AFS2 mounted'),
                                        1, lambda: workflow(disk))], disk, pointer=True, timeout_s=240)
    assert rc == 0 and not afs1.audit(disk)
    assert disk.read_bytes()[:arena_env.SCRATCH_MIB * 1024 * 1024] == afs1_before, 'AFS1 changed'
    t = tree(disk)
    assert t['Documents/Work/user-note'] == b'XAa file manager' and t['Documents/user-bin'] == b'\x80binary'
    assert t['Desktop/user-full'] == b'f' * 64 and t['Desktop/fresh.txt'] == b'hello'
    assert not any(k.startswith('.Trash/') for k in t)
    print('[m11-explorer] AFS2: new folder named inline, copy/paste, double-click navigation, history, cut/paste '
          'via a place, delete to Trash with restore record, drag move with a free name, context-menu restore, '
          'keyboard rename, double-click open in a real Editor with exact save, empty Trash, sort and grid pixels, '
          'another application\'s change shown on focus; AFS1 originals byte-identical PASS')


if __name__ == '__main__':
    main()
