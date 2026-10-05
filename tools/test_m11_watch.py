#!/usr/bin/env python3
"""Phase 11: directory watches drive Files and the desktop (ADR-0079).

Files is snapped to the left half and the Terminal to the right; the
Terminal keeps the focus and changes the folders Files shows. With no
input reaching Files (no focus change, no click), its window must show:

* a file created in the watched folder;
* a rename INTO the watched folder (destination parent);
* a rename OUT OF the watched folder (source parent);
* a file whose contents changed (its size, via the parent);
* the watched folder being removed (Files falls back to home).

The desktop surface shows a file the terminal put in /Users/user/Desktop
with no desktop poll and no click. Closing Files ends its watch with its
lineage; many folder switches never exhaust filesd. Every change is
judged on AFS2 bytes as well as pixels.
"""
import time
import afs1
import afs2
import arena_env
import mtest
import test_m84_stage as seed
from test_m10_apps import Desktop
from test_m10_desktop import crop

LABEL = 'm11-watch'
BASE = arena_env.AFS2_BASE_SECTOR * 512
HOME = '/Users/user/'
SEED = {b'user-note': b'note', b'user-full': b'full'}
# Files snapped left (0..400 x 26..544), Terminal snapped right.
FILES_LIST = (120, 100, 270, 300)
TERMINAL_TITLE = (600, 40)


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


def chord(d, *keys):
    *mods, key = keys
    d.q.command('input-send-event', events=[d.q._ev(m, True) for m in mods]
                + [d.q._ev(key, True), d.q._ev(key, False)] + [d.q._ev(m, False) for m in reversed(mods)])
    time.sleep(.3)


def click(d, x, y):
    d.point(x, y, True)
    d.point(x, y, False)
    time.sleep(.3)


def files_list(p):
    return crop(p, *FILES_LIST)


def workflow(disk):
    d = Desktop(LABEL)
    try:
        d.wait(lambda: '[desktop] watching /Users/user/Desktop' in d.serial(), 'broker did not watch the Desktop folder')
        d.launch(1, 'files')
        chord(d, 'meta_l', 'left')
        # Files on the Desktop place (empty folder), then the Terminal.
        click(d, 30, 26 + 68 + 8 + 22 + 11)
        time.sleep(.5)
        d.launch(0, 'terminal', 1)
        chord(d, 'meta_l', 'right')
        empty = d.settled('desktop-folder', FILES_LIST)

        def typed(text):
            d.q.type_text(text + '\r', gap_s=.04)

        def changed(name, old):
            return d.shot(name, lambda p: files_list(p) != files_list(old))

        typed('put Desktop/w1.txt a')
        until(d, disk, lambda t: t.get('Desktop/w1.txt') == b'a', 'put failed')
        after_put = changed('watch-create', empty)
        after_put = d.settled('watch-create', FILES_LIST)
        # Rename INTO the watched folder: the destination parent.
        typed('mv Documents/user-note Desktop/user-note')
        until(d, disk, lambda t: t.get('Desktop/user-note') == b'note', 'mv into Desktop failed')
        after_in = changed('watch-rename-destination', after_put)
        after_in = d.settled('watch-rename-destination', FILES_LIST)
        # Contents change: the parent listing's size column.
        typed('put Desktop/w1.txt a much longer body of text')
        until(d, disk, lambda t: t.get('Desktop/w1.txt') == b'a much longer body of text', 'rewrite failed')
        after_write = changed('watch-contents', after_in)
        after_write = d.settled('watch-contents', FILES_LIST)
        # Rename OUT OF the watched folder: the source parent.
        typed('mv Desktop/user-note Documents/user-note')
        until(d, disk, lambda t: t.get('Documents/user-note') == b'note' and 'Desktop/user-note' not in t,
              'mv out of Desktop failed')
        changed('watch-rename-source', after_write)
        assert '[desktop] watching /Users/user/Desktop' in d.serial()
        # The watched folder removed: Files falls back to home by itself.
        typed('mkdir Desktop/sub')
        until(d, disk, lambda t: 'Desktop/sub' in t, 'mkdir failed')
        click(d, 200, 40)  # focus Files to enter the folder
        settled = d.settled('desktop-with-sub', FILES_LIST)
        d.q.type_text('s', gap_s=.05)  # type-ahead selects "sub"
        d.q.key('\r')
        inside = changed('inside-sub', settled)
        inside = d.settled('inside-sub', FILES_LIST)
        click(d, *TERMINAL_TITLE)
        typed('rmdir Desktop/sub')
        until(d, disk, lambda t: 'Desktop/sub' not in t, 'rmdir failed')
        changed('watch-folder-removed', inside)
        # Many folder switches never exhaust filesd (each moves the watch).
        click(d, 200, 40)
        for i in range(30):
            click(d, 30, 26 + 68 + 8 + 22 * (1 + i % 2) + 11)
        assert 'filesd: refused' not in d.serial(), 'filesd refused during folder switches'
        click(d, *TERMINAL_TITLE)
        before = d.settled('before-last-put', FILES_LIST)
        typed('put Documents/after-switches.txt z')
        until(d, disk, lambda t: t.get('Documents/after-switches.txt') == b'z', 'late put failed')
        changed('watch-after-switches', before)
        # Close the Terminal, then Files: its watch ends with its lineage.
        d.q.command('input-send-event', events=[d.q._ev('f8', True), d.q._ev('f8', False)])
        d.wait(lambda: d.serial().count('[desktop] application retired:') >= 1, 'terminal not retired')
        click(d, 200, 40)
        d.q.command('input-send-event', events=[d.q._ev('f8', True), d.q._ev('f8', False)])
        d.wait(lambda: d.serial().count('[desktop] application retired:') >= 2, 'Files not retired')
        d.wait(lambda: 'directory watch(es) ended with the lineage' in d.serial(), 'Files watch not ended')
        # The desktop shows w1.txt from its own watch (no poll, no click):
        # its icon is drawn in the first cells.
        d.shot('desk-from-watch', lambda p: len(set(crop(p, 20, 40, 64, 60)[i:i + 3] for i in range(0, 64 * 60 * 3, 3))) > 4)
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
    ready = (b'[desktop] real desktop frame presented', b'arena>', b'filesd: AFS2 mounted')
    rc, s, _ = mtest.boot(LABEL, esp, [(ready, 1, lambda: workflow(disk))], disk, pointer=True, timeout_s=240)
    assert rc == 0 and not afs1.audit(disk)
    print('[m11-watch] unfocused Files updated by its folder watch for create, rename destination, contents '
          'change, rename source and folder removal; the desktop updated by its watch without polling; '
          '30 folder switches without exhausting filesd; the watch ended with its lineage PASS')


if __name__ == '__main__':
    main()
