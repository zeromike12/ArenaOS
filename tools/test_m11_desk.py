#!/usr/bin/env python3
"""Phase 11.8: the desktop surface shows /Users/user/Desktop as icons.

Judged on AFS2 bytes read back on the host: icons appear for objects the
terminal created; a dragged icon's cell is persisted in Desktop/.positions
and drawn at the same place after a fresh boot; double-click opens a text
file in a real Editor (a broker grant of exactly that file) whose save is
exact; the desktop menu creates a folder; an icon dropped on a folder icon
moves into it; Move to Trash leaves a restore record; double-clicking a
folder opens Files there; a binary file opens nothing (notice).
"""
import time
import afs1
import afs2
import arena_env
import mtest
import test_m84_stage as seed
from test_m10_apps import Desktop
from test_m10_desktop import crop

LABEL = 'm11-desk'
BASE = arena_env.AFS2_BASE_SECTOR * 512
HOME = '/Users/user/'
SEED = {b'user-note': b'note'}
# desk.rs: cells 84x74 from (10, 36), column by column.
LEFT, TOP, CW, CH = 10, 36, 84, 74


def cell(col, row):
    """Screen point on the icon glyph of cell (col, row)."""
    return LEFT + col * CW + CW // 2, TOP + row * CH + 20


def tree(disk):
    for _ in range(50):
        try:
            vol = afs2.Volume(disk.read_bytes()[BASE:])
            afs2.check(vol)
            return {k[len(HOME):]: v for k, v in afs2.walk(vol).items() if k.startswith(HOME)}
        except Exception:
            time.sleep(0.1)
    raise AssertionError('AFS2 region never mounted cleanly on the host')


def until(d, disk, predicate, what):
    end = time.monotonic() + 15
    while time.monotonic() < end:
        t = tree(disk)
        if predicate(t):
            return t
        time.sleep(.15)
    d.q.command('screendump', filename=str(arena_env.build_dir() / f'{LABEL}-failed.ppm'), format='ppm')
    raise AssertionError(what + ': ' + repr(sorted(tree(disk))))


def press(d, x, y, button='left', down=True):
    d.q.command('input-send-event', events=[
        {'type': 'abs', 'data': {'axis': 'x', 'value': (x * 32767 + 799) // 799}},
        {'type': 'abs', 'data': {'axis': 'y', 'value': (y * 32767 + 599) // 599}},
        {'type': 'btn', 'data': {'button': button, 'down': down}}])


def click(d, x, y, button='left'):
    press(d, x, y, button)
    press(d, x, y, button, down=False)
    time.sleep(.25)


def double(d, x, y):
    press(d, x, y)
    press(d, x, y, down=False)
    press(d, x, y)
    press(d, x, y, down=False)
    time.sleep(.3)


def drag(d, x, y, tx, ty):
    press(d, x, y)
    d.point(x + 8, y + 8)
    time.sleep(.05)
    d.point((x + tx) // 2, (y + ty) // 2)
    time.sleep(.05)
    d.point(tx, ty)
    time.sleep(.1)
    d.point(tx, ty, False)
    time.sleep(.3)


def icons(p):
    return crop(p, 0, 30, 360, 500)


def workflow(disk, shots):
    d = Desktop(LABEL)
    try:
        d.wait(lambda: '[desktop] desktop surface shows /Users/user/Desktop' in d.serial(), 'desktop surface not loaded')
        empty = d.settled('empty', (0, 30, 360, 500))
        d.launch(0, 'terminal')
        d.q.type_text('mkdir Desktop/Folder\r', gap_s=.04)
        d.q.type_text('put Desktop/hello.txt hi\r', gap_s=.04)
        d.q.type_text('put Desktop/keep.txt keep\r', gap_s=.04)
        d.q.type_text('put Desktop/photo.png pixels\r', gap_s=.04)
        until(d, disk, lambda t: t.get('Desktop/keep.txt') == b'keep' and 'Desktop/photo.png' in t, 'terminal setup failed')
        d.close()
        d.wait(lambda: d.serial().count('[desktop] application retired:') >= 1, 'terminal not retired')
        # Folders first, then names: Folder, hello.txt, keep.txt, photo.png.
        shown = d.shot('icons-shown', lambda p: icons(p) != icons(empty))
        shown = d.settled('icons-shown', (0, 30, 360, 500))
        # Drag keep.txt (0,2) to cell (3,2): persisted.
        drag(d, *cell(0, 2), *cell(3, 2))
        until(d, disk, lambda t: b'3 2 keep.txt' in (t.get('Desktop/.positions') or b''), 'cell not persisted')
        shots['moved'] = d.settled('moved', (0, 30, 360, 500))
        # Double-click hello.txt (0,1): a real Editor holding that file.
        spawned = d.serial().count('[desktop] real application spawned;')
        double(d, *cell(0, 1))
        d.wait(lambda: d.serial().count('[desktop] real application spawned;') == spawned + 1, 'no Editor')
        d.opened('editor')
        d.q.type_text('X', gap_s=.05)
        d.q.command('input-send-event', events=[d.q._ev('ctrl', True), d.q._ev('s', True), d.q._ev('s', False), d.q._ev('ctrl', False)])
        until(d, disk, lambda t: t.get('Desktop/hello.txt') == b'Xhi', 'Editor save from a desktop open missed')
        d.close()
        d.wait(lambda: d.serial().count('[desktop] application retired:') >= 2, 'editor not retired')
        # A kind nothing opens (an image) opens nothing.
        before = d.serial().count('[desktop] real application spawned;')
        double(d, *cell(0, 3))
        time.sleep(.5)
        assert d.serial().count('[desktop] real application spawned;') == before, 'an image opened an app'
        # Desktop menu: New Folder (first item), on bare desktop.
        click(d, 500, 460, 'right')
        click(d, 520, 460 + 4 + 12)
        until(d, disk, lambda t: 'Desktop/New Folder' in t, 'menu New Folder failed')
        # Drop hello.txt onto Folder (0,0).
        time.sleep(1.2)
        drag(d, *cell(0, 1), *cell(0, 0))
        until(d, disk, lambda t: t.get('Desktop/Folder/hello.txt') == b'Xhi' and 'Desktop/hello.txt' not in t,
              'drop onto a folder icon did not move')
        # Move Folder to the Trash from its icon menu (second item).
        click(d, *cell(0, 0), 'right')
        x, y = cell(0, 0)
        click(d, x + 20, y + 4 + 24 + 12)
        until(d, disk, lambda t: '.Trash/Folder/hello.txt' in t and t.get('.Trash/.restore/Folder') == b'Desktop/Folder',
              'Move to Trash failed')
        # Double-click New Folder: Files opens there.
        time.sleep(1.2)
        spawned = d.serial().count('[desktop] real application spawned;')
        layout = tree(disk).get('Desktop/.positions') or b''
        row = next(int(l.split(b' ')[1]) for l in layout.split(b'\n') if l.endswith(b' New Folder'))
        col = next(int(l.split(b' ')[0]) for l in layout.split(b'\n') if l.endswith(b' New Folder'))
        double(d, *cell(col, row))
        d.wait(lambda: d.serial().count('[desktop] real application spawned;') == spawned + 1, 'no Files')
        d.opened('files-at-folder')
        d.close()
        d.wait(lambda: d.serial().count('[desktop] application retired:') >= 3, 'files not retired')
        click(d, 500, 330)  # bare desktop: nothing selected, as after a boot
        shots['before-reboot'] = d.settled('before-reboot', (0, 30, 360, 500))
        return b'shutdown\r'
    finally:
        d.dispose()


def reboot(shots):
    d = Desktop(LABEL + '-again')
    try:
        d.wait(lambda: '[desktop] desktop surface shows /Users/user/Desktop' in d.serial(), 'desktop surface not loaded')
        d.shot('same-icons', lambda p: icons(p) == icons(shots['before-reboot']))
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
    shots = {}
    ready = (b'[desktop] real desktop frame presented', b'arena>', b'filesd: AFS2 mounted')
    rc, s, _ = mtest.boot(LABEL, esp, [(ready, 1, lambda: workflow(disk, shots))], disk, pointer=True, timeout_s=240)
    assert rc == 0 and not afs1.audit(disk)
    rc, s, _ = mtest.boot(LABEL + '-again', esp, [(ready, 1, lambda: reboot(shots))], disk, pointer=True)
    assert rc == 0
    t = tree(disk)
    assert t['Desktop/keep.txt'] == b'keep' and b'3 2 keep.txt' in t['Desktop/.positions']
    print('[m11-desk] desktop icons for real AFS2 objects; dragged cell persisted and redrawn after a fresh boot; '
          'double-click text opens a granted Editor with exact save; binary opens nothing; menu New Folder; drop '
          'onto a folder icon moves; Move to Trash with restore record; folder opens in Files PASS')


if __name__ == '__main__':
    main()
