#!/usr/bin/env python3
"""Phase-11 standalone witness: AFS2 through the Desktop and Files.

Runs on a live desktop after the Phase-10 pixel witness (the desktop is
empty again). Only Python's standard library, `qmp.py` and `afs2.py`:

1. The desktop menu's New Folder creates /Users/user/Desktop/New Folder
   (read back from the AFS2 region of the guest's disk) and its icon is
   drawn in the first desktop cell.
2. Double-clicking that icon spawns a real Files process showing the
   folder (pixels change where its window opens).
3. Ctrl+Shift+N in Files creates "New Folder" inside it; Enter keeps the
   name. The disk holds Desktop/New Folder/New Folder.
4. F8 closes Files: its process is retired and the window uncovers.
"""
import re
import sys
import time
from pathlib import Path

import afs2
import qmp

AFS2_BASE = 16384 * 512
HOME = '/Users/user/'
# desk.rs: cells 84x74 from (10, 36), column by column.
LEFT, TOP, CW, CH = 10, 36, 84, 74


def capture(sock, serial, image, disk, timeout):
    deadline = time.monotonic() + timeout
    conn = qmp.Qmp(str(sock), connect_timeout_s=4)

    def log():
        return serial.read_bytes() if serial.is_file() else b''

    def wait(predicate, message):
        while time.monotonic() < deadline:
            if predicate():
                return
            time.sleep(.1)
        raise TimeoutError(message)

    def tree():
        try:
            vol = afs2.Volume(Path(disk).read_bytes()[AFS2_BASE:])
            afs2.check(vol)
            return {k[len(HOME):]: v for k, v in afs2.walk(vol).items() if k.startswith(HOME)}
        except Exception:
            return {}

    def shot():
        conn.command('screendump', filename=str(image), format='ppm')
        raw = image.read_bytes()
        head = re.match(rb'P6\s+(\d+)\s+(\d+)\s+255\s', raw)
        if not head or (int(head[1]), int(head[2])) != (800, 600):
            raise ValueError('desktop image is not the tested 800x600 P6 scanout')
        return raw[head.end():]

    def crop(p, x, y, w, h):
        return b''.join(p[((y + r) * 800 + x) * 3:((y + r) * 800 + x + w) * 3] for r in range(h))

    def until_pixels(predicate, message):
        while time.monotonic() < deadline:
            p = shot()
            if predicate(p):
                return p
            time.sleep(.05)
        raise TimeoutError(message)

    def press(x, y, button='left', down=True):
        conn.command('input-send-event', events=[
            {'type': 'abs', 'data': {'axis': 'x', 'value': (x * 32767 + 799) // 799}},
            {'type': 'abs', 'data': {'axis': 'y', 'value': (y * 32767 + 599) // 599}},
            {'type': 'btn', 'data': {'button': button, 'down': down}}])

    def click(x, y, button='left'):
        press(x, y, button)
        press(x, y, button, down=False)
        time.sleep(.25)

    def keys(*names):
        conn.command('input-send-event', events=[conn._ev(n, True) for n in names]
                     + [conn._ev(n, False) for n in reversed(names)])
        time.sleep(.3)

    try:
        first = (LEFT, TOP, CW, CH)
        empty = shot()
        # 1. Desktop menu: New Folder (first item), on bare desktop.
        click(500, 460, 'right')
        click(520, 460 + 4 + 12)
        wait(lambda: 'Desktop/New Folder' in tree(), 'desktop menu New Folder not on AFS2')
        icon = until_pixels(lambda p: crop(p, *first) != crop(empty, *first), 'desktop icon not drawn')
        # 2. Double-click the icon: a real Files process opens the folder.
        spawned = log().count(b'[desktop] real application spawned;')
        x, y = LEFT + CW // 2, TOP + 20
        for _ in range(2):
            press(x, y)
            press(x, y, down=False)
        wait(lambda: log().count(b'[desktop] real application spawned;') == spawned + 1, 'Files not spawned')
        opened = until_pixels(lambda p: crop(p, 200, 150, 300, 200) != crop(icon, 200, 150, 300, 200),
                              'Files window not drawn')
        time.sleep(1.0)
        opened = shot()
        # 3. Ctrl+Shift+N: New Folder inside it, Enter keeps the name.
        keys('ctrl', 'shift', 'n')
        keys('ret')
        wait(lambda: 'Desktop/New Folder/New Folder' in tree(), 'Files New Folder not on AFS2')
        until_pixels(lambda p: p != opened, 'Files listing did not change')
        # 4. F8 closes Files: retired, window uncovered.
        retired = log().count(b'[desktop] application retired:')
        keys('f8')
        wait(lambda: log().count(b'[desktop] application retired:') == retired + 1, 'Files not retired')
        until_pixels(lambda p: crop(p, 200, 150, 300, 200) == crop(icon, 200, 150, 300, 200),
                     'Files window did not uncover')
    finally:
        conn.close()
    return 'DESKTOP-MENU-FOLDER FILES-OPEN FILES-NEW-FOLDER FILES-CLOSE'


def main():
    if len(sys.argv) != 6:
        raise SystemExit('usage: check_phase11_files.py QMP_SOCKET SERIAL PPM DISK TIMEOUT_S')
    sock, serial, image, disk = map(Path, sys.argv[1:5])
    try:
        print('[phase11-files]', capture(sock, serial, image, disk, float(sys.argv[5])), 'PASS', flush=True)
    except Exception as error:
        print(f'[phase11-files] FAIL: {error}', file=sys.stderr)
        return 1
    return 0


if __name__ == '__main__':
    sys.exit(main())
