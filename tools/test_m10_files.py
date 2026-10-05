#!/usr/bin/env python3
"""Files and Editor on AFS2 capabilities (Phase 10 invariants, re-proven).

Superseded storage (ADR-0077): the Phase-10 desktop kept user documents as
flat AFS1 `user-*` records reached through broker name scopes. Phase 11.6
retired those scopes; documents live on AFS2 and applications hold file
capabilities. Every Phase-10 invariant of this test is kept and judged on
the AFS2 bytes instead: actual select/preview, Files opening the selected
file in a real Editor (a capability offer), exact save, duplicate create
refused without overwrite, case-sensitive owned glyphs, delete, the bounded
4096-byte document and its refusal, Save As durable bytes, an unsupported
(binary) document refused without touching the open one, and a real
terminal launch. The AFS1 originals stay byte-identical (read only).
"""
import time
import afs1
import afs2
import arena_env
import mtest
import test_m84_stage as seed
from test_m10_apps import Desktop
from test_m10_desktop import crop

LABEL = 'm10-files'
BUILD = arena_env.build_dir()
BASE = arena_env.AFS2_BASE_SECTOR * 512
DOCS = '/Users/user/Documents/'
SEED = {b'user-note': b'Aa file manager', b'user-full': b'f' * 4096, b'user-bin': b'\x80binary'}


def tree(disk):
    for _ in range(50):
        try:
            vol = afs2.Volume(disk.read_bytes()[BASE:])
            afs2.check(vol)
            return afs2.walk(vol)
        except Exception:
            time.sleep(0.1)
    raise AssertionError('AFS2 region never mounted cleanly on the host')


def doc(disk, name):
    return tree(disk).get(DOCS + name)


def keys(d, *names, gap=.05):
    for n in names:
        d.q.command('input-send-event', events=[d.q._ev(n, True), d.q._ev(n, False)])
        time.sleep(gap)


def chord(d, *mods_and_key):
    *mods, key = mods_and_key
    d.q.command('input-send-event', events=[d.q._ev(m, True) for m in mods]
                + [d.q._ev(key, True), d.q._ev(key, False)] + [d.q._ev(m, False) for m in reversed(mods)])


def choose(d, disk, name):
    """Pick `name` in the trusted chooser (open on Documents)."""
    rows = sorted(k[len(DOCS):].encode() for k in tree(disk)
                  if k.startswith(DOCS) and '/' not in k[len(DOCS):])
    keys(d, *(['down'] * (rows.index(name.encode()) + 1)), 'ret')


def save_as(d, name):
    opened = d.serial().count('trusted chooser opened')
    chord(d, 'ctrl', 'shift', 's')
    d.wait(lambda: d.serial().count('trusted chooser opened') > opened, 'Save As chooser not opened')
    time.sleep(.3)
    keys(d, *(['backspace'] * 31), gap=.03)
    d.q.type_text(name, gap_s=.03)
    keys(d, 'ret')


def workflow(disk):
    d = Desktop(LABEL)
    try:
        d.wait(lambda: 'AFS2 file service online' in d.serial(), 'no AFS2 session')
        empty = d.shot('empty')
        files = d.launch(1, 'files')
        # Documents: user-bin, user-full, user-note (byte order). Select the
        # note: its text preview is real pixels.
        keys(d, 'down', 'down')
        d.shot('selected-preview', lambda p: crop(p, 296, 136, 190, 14) != crop(files, 296, 136, 190, 14))
        count = d.serial().count('[desktop] real application spawned;')
        d.click(345, 110)  # Open: a real Editor holding exactly this file
        editor = d.opened('files-open-editor', 1)
        assert d.serial().count('[desktop] real application spawned;') == count + 1
        # Case-sensitive bytes have different real glyphs in the owned raster.
        assert crop(editor, 114, 158, 5, 7) != crop(editor, 120, 158, 5, 7), 'upper/lower case rendered identically'
        d.q.type_text('X', gap_s=.04)
        d.click(70 + 115, 110)  # Save
        d.wait(lambda: doc(disk, 'user-note') == b'XAa file manager', 'Files-open editor did not save the file')
        d.close(1)
        d.wait(lambda: d.serial().count('[desktop] application retired:') >= 1, 'opened editor not retired')
        # New must refuse an existing name without emptying its bytes.
        d.click(105, 110)
        dialog = d.shot('create-dialog', lambda p: crop(p, 82, 96, 354, 24) != crop(files, 82, 96, 354, 24))
        d.q.type_text('\b' * 31 + 'user-note\r', gap_s=.035)
        d.shot('create-refused', lambda p: crop(p, 82, 328, 350, 12) != crop(dialog, 82, 328, 350, 12))
        assert doc(disk, 'user-note') == b'XAa file manager'
        keys(d, 'esc')
        d.click(420, 110)  # Delete the selected note
        d.wait(lambda: doc(disk, 'user-note') is None, 'Files delete did not unlink the object')
        d.close()
        d.wait(lambda: d.serial().count('[desktop] application retired:') >= 2, 'Files not retired')
        blank = d.launch(2, 'editor')
        chord(d, 'ctrl', 'o')
        d.wait(lambda: d.serial().count('trusted chooser opened') >= 1, 'chooser not opened')
        time.sleep(.3)
        choose(d, disk, 'user-full')
        full = d.shot('full-text', lambda p: crop(p, 88, 134, 330, 140) != crop(blank, 88, 134, 330, 140))
        d.q.key('q')  # a 4097th byte is refused
        d.shot('full-refused', lambda p: crop(p, 82, 328, 350, 12) != crop(full, 82, 328, 350, 12))
        assert doc(disk, 'user-full') == b'f' * 4096
        keys(d, 'delete')
        d.q.type_text('Z', gap_s=.04)
        chord(d, 'ctrl', 's')
        d.wait(lambda: doc(disk, 'user-full') == b'Z' + b'f' * 4095, '4096-byte bounded editing/save failed')
        # Save As creates real bytes through a new chooser grant.
        save_as(d, 'user-copy')
        d.wait(lambda: doc(disk, 'user-copy') == b'Z' + b'f' * 4095, 'Save As did not commit the exact document')
        # An unsupported (binary) document is refused; the open document and
        # its capability are unchanged, so a save still targets the copy.
        before_open = d.shot('before-binary-open')
        chord(d, 'ctrl', 'o')
        d.wait(lambda: d.serial().count('trusted chooser opened') >= 3, 'chooser not opened')
        time.sleep(.3)
        choose(d, disk, 'user-bin')
        d.shot('unsupported-refused', lambda p: crop(p, 82, 328, 350, 12) != crop(before_open, 82, 328, 350, 12))
        keys(d, 'backspace')  # remove the Z; the next save must land in user-copy
        chord(d, 'ctrl', 's')
        d.wait(lambda: doc(disk, 'user-copy') == b'f' * 4095, 'save after the refused open missed user-copy')
        assert doc(disk, 'user-bin') == b'\x80binary', 'refused binary document was written'
        d.close()
        d.wait(lambda: d.serial().count('[desktop] application retired:') >= 3, 'editor not retired')
        d.shot('empty-again', lambda p: crop(p, 100, 110, 300, 180) == crop(empty, 100, 110, 300, 180))
        d.launch(0, 'terminal')
        before = d.serial().count('[desktop] real application spawned;')
        d.q.type_text('launch gallery\r', gap_s=.04)
        d.opened('terminal-launched-gallery', 1)
        assert d.serial().count('[desktop] real application spawned;') == before + 1
        d.close(1)
        d.wait(lambda: d.serial().count('[desktop] application retired:') >= 4, 'terminal-launched gallery not retired')
        d.close()
        d.wait(lambda: d.serial().count('[desktop] application retired:') >= 5, 'terminal not retired')
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
                                        1, lambda: workflow(disk))], disk, pointer=True, timeout_s=180)
    assert rc == 0 and not afs1.audit(disk)
    assert disk.read_bytes()[:arena_env.SCRATCH_MIB * 1024 * 1024] == afs1_before, 'AFS1 changed'
    t = tree(disk)
    assert DOCS + 'user-note' not in t and t[DOCS + 'user-bin'] == b'\x80binary'
    assert t[DOCS + 'user-copy'] == b'f' * 4095 and t[DOCS + 'user-full'] == b'Z' + b'f' * 4095
    print('[m10-files] AFS2: Files select/preview/open-in-Editor (capability offer)/save/delete, duplicate create '
          'refused without overwrite, case-sensitive owned glyphs, bounded 4096-byte document and refusal, Save As '
          'durable bytes, binary document refused leaving the open document and its capability, terminal real launch, '
          'AFS1 originals byte-identical PASS')


if __name__ == '__main__':
    main()
