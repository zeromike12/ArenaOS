#!/usr/bin/env python3
"""Phase 11.6: file capabilities and the trusted chooser, in the real desktop
on a real AFS2 volume (ADR-0077), judged on the host.

The disk starts as AFS1 with two user documents and a signed hostile probe
application staged; the first boot migrates it to AFS2. Then:

* terminal: put, mkdir, mv inside /Users/user through its home capability,
  and requests aimed at /System (`/System/...`, `../../System/...`) that
  change nothing anywhere (malicious /System request);
* Editor + chooser: Ctrl+O, the user picks a document, edits, Ctrl+S:
  exact bytes (read-write grant); the chooser cancelled with Esc grants
  nothing;
* rename semantics: the document is renamed underneath the Editor; its
  capability follows the object, so the next save lands in the renamed
  file and the old name stays absent;
* stale after delete: the document is deleted; a save through the old
  capability is refused and creates nothing;
* read-only grant: Ctrl+Shift+O, the chosen document cannot be written
  through it (filesd refuses), its bytes stay exact;
* forged requests: a signed third-party probe is granted one document
  read-only by the user and then tries raw hostile requests (write,
  truncate, `..`, `System`, delete, rights amplification, a forged session
  page, a foreign rename destination, a forged request, a foreign revoke);
  it paints green only when filesd answered every one as expected;
* application death: closing an application retires its whole lineage in
  filesd (logged with the record count).

The AFS1 copies of the user documents stay untouched (AFS1 is read only
after migration).
"""
import re
import subprocess
import sys
import time
import afs1
import afs2
import arena_env
import mtest
import package_record as rec
import test_m84_stage as stage
from test_package_record import RFC_SEED, openssl_sign
from test_m10_apps import Desktop
from test_m10_desktop import crop

ROOT = arena_env.REPO_ROOT
BUILD = arena_env.build_dir()
LABEL = 'm11-files'
BASE = arena_env.AFS2_BASE_SECTOR * 512
AFS1_BYTES = arena_env.SCRATCH_MIB * 1024 * 1024
NOTE = b'meeting notes\n'
LETTER = b'dear reader\n'
CHOOSER = (190, 130, 420, 340)


def probe_package():
    crate = ROOT / 'tools/phase11-files-probe'
    subprocess.run(['cargo', 'build', '--offline', '--locked', '--release'], cwd=crate,
                   env=arena_env.rust_env(), check=True)
    elf = (crate / 'target/x86_64-unknown-none/release/arena-phase11-files-probe').read_bytes()
    assert 1 <= len(elf) <= 4096, len(elf)
    unsigned = rec.signed_package(stage.ID, 7, elf, rec.ROOT)
    return unsigned + openssl_sign(RFC_SEED, rec.PKG_DOMAIN + unsigned), elf


def tree(disk):
    """The committed AFS2 namespace (retried while the guest is writing)."""
    for _ in range(50):
        try:
            vol = afs2.Volume(disk.read_bytes()[BASE:])
            afs2.check(vol)
            return afs2.walk(vol)
        except Exception:
            time.sleep(0.1)
    raise AssertionError('AFS2 region never mounted cleanly on the host')


def wait_tree(d, disk, predicate, what):
    end = time.monotonic() + 15
    while time.monotonic() < end:
        t = tree(disk)
        if predicate(t):
            return t
        time.sleep(0.1)
    raise AssertionError(what + f': {sorted(k for k in tree(disk) if "/Users" in k)}')


def keys(d, *names):
    d.q.command('input-send-event', events=[d.q._ev(n, down) for n, down in names])


def press(d, name):
    keys(d, (name, True), (name, False))


def chord(d, *mods_and_key):
    *mods, key = mods_and_key
    keys(d, *[(m, True) for m in mods], (key, True), (key, False), *[(m, False) for m in reversed(mods)])


def shown(d, name, before):
    return d.shot(name, lambda p: crop(p, *CHOOSER) != crop(before, *CHOOSER))


DOCS = '/Users/user/Documents/'


def main():
    signed, elf = probe_package()
    esp = mtest.build(LABEL, desktop=True)
    disk = arena_env.make_scratch_disk()
    afs1.mkfs_with_files(disk, AFS1_BYTES // afs1.SECTOR, {b'user-note': NOTE, b'user-letter': LETTER})
    rc, s, _ = mtest.boot(LABEL + '-seed', esp, [(b'arena>', 1, b'shutdown\r')], disk, pointer=True)
    assert rc == 0 and 'no AFS2 region' in s, s[-2000:]
    stage.host_seed(disk, {stage.STAGE1: signed, stage.POLICY1: stage.POLICY})
    with open(disk, 'r+b') as f:
        f.truncate(arena_env.AFS2_DISK_MIB * 1024 * 1024)
    afs1_before = disk.read_bytes()[:AFS1_BYTES]

    def workflow():
        d = Desktop(LABEL)
        try:
            d.wait(lambda: 'AFS2 file service online' in d.serial(), 'desktop never got its filesd session')
            t = tree(disk)
            assert t[DOCS + 'user-note'] == NOTE and t[DOCS + 'user-letter'] == LETTER, sorted(t)
            system_before = {k: v for k, v in t.items() if k.startswith('/System')}
            empty = d.shot('empty')

            # --- terminal through its home capability.
            d.launch(0, 'terminal')
            for line in ['cd Documents', 'put hello.txt hello world', 'mkdir Projects',
                         'mv hello.txt Projects']:
                d.q.type_text(line + '\r', gap_s=.03)
            wait_tree(d, disk, lambda t: t.get(DOCS + 'Projects/hello.txt') == b'hello world'
                      and DOCS + 'hello.txt' not in t, 'terminal put/mkdir/mv')
            # Malicious /System requests: `/` is home, `..` never climbs out.
            for line in ['put /System/evil x', 'mkdir ../../System/pwned',
                         'put ../../../System/imported-afs1/ui10-prefs HACK', 'rm /System/afs1-import-complete',
                         'mv ../../System Projects']:
                d.q.type_text(line + '\r', gap_s=.03)
            d.q.type_text('put marker.txt done\r', gap_s=.03)
            t = wait_tree(d, disk, lambda t: t.get(DOCS + 'marker.txt') == b'done', 'terminal marker')
            assert {k: v for k, v in t.items() if k.startswith('/System')} == system_before, 'System changed'
            assert not any('evil' in k or 'pwned' in k or k.startswith('/Users/user/System') for k in t), sorted(t)
            print('[m11-files] terminal put/mkdir/mv through the home capability exact; /System requests '
                  '(absolute, `..`, rm, mv) changed nothing PASS', flush=True)
            press(d, 'f8')
            d.wait(lambda: d.serial().count('[desktop] application retired:') >= 1, 'terminal not retired')
            d.wait(lambda: d.serial().count('filesd: lineage retired:') >= 1, 'terminal lineage not retired')

            # --- Editor: chooser cancel, then a read-write grant.
            press(d, 'f3')
            d.wait(lambda: d.serial().count('[desktop] real application spawned;') >= 2, 'editor not spawned')
            editor = d.opened('editor')
            chord(d, 'ctrl', 'o')
            shown(d, 'chooser-cancel', editor)
            press(d, 'esc')
            d.wait(lambda: 'trusted chooser cancelled; nothing granted' in d.serial(), 'cancel not reported')
            d.shot('chooser-closed', lambda p: crop(p, *CHOOSER) == crop(editor, *CHOOSER))
            chord(d, 'ctrl', 'o')
            shown(d, 'chooser-open', editor)
            # Documents: marker.txt, Projects/, user-letter, user-note.
            for _ in range(4):
                press(d, 'down')
            press(d, 'ret')
            d.wait(lambda: d.serial().count('trusted chooser granted one file capability') == 1, 'no grant')
            time.sleep(0.3)
            d.q.type_text('EDIT ', gap_s=.03)
            chord(d, 'ctrl', 's')
            wait_tree(d, disk, lambda t: t.get(DOCS + 'user-note') == b'EDIT ' + NOTE, 'read-write save')
            print('[m11-files] chooser cancel grants nothing; chosen document read/write save exact PASS', flush=True)

            # --- rename underneath the Editor: the capability follows the object.
            d.launch(0, 'terminal-2', 1)
            d.q.type_text('mv Documents/user-note Documents/renamed-note\r', gap_s=.03)
            wait_tree(d, disk, lambda t: DOCS + 'renamed-note' in t and DOCS + 'user-note' not in t, 'mv')
            d.click(70 + 120, 60 + 10)  # focus the Editor's title bar
            time.sleep(0.2)
            d.q.type_text('X', gap_s=.03)
            chord(d, 'ctrl', 's')
            t = wait_tree(d, disk, lambda t: t.get(DOCS + 'renamed-note') == b'XEDIT ' + NOTE, 'save after rename')
            assert DOCS + 'user-note' not in t
            print('[m11-files] rename under an open document: the capability follows the object PASS', flush=True)

            # --- delete underneath the Editor: the capability is stale.
            d.click(96 + 120, 84 + 10)  # the terminal
            d.q.type_text('rm Documents/renamed-note\r', gap_s=.03)
            before = wait_tree(d, disk, lambda t: DOCS + 'renamed-note' not in t, 'rm')
            d.click(70 + 120, 60 + 10)
            time.sleep(0.2)
            d.q.type_text('Y', gap_s=.03)
            chord(d, 'ctrl', 's')
            time.sleep(1.5)
            assert tree(disk) == before, 'a save through a stale capability changed the volume'
            print('[m11-files] save through a capability whose file was deleted is refused, nothing created PASS',
                  flush=True)

            # --- read-only grant in a second Editor.
            press(d, 'f3')
            d.wait(lambda: d.serial().count('[desktop] real application spawned;') >= 4, 'editor 2 not spawned')
            time.sleep(0.5)
            ro = d.shot('editor-2')
            chord(d, 'ctrl', 'shift', 'o')
            shown(d, 'chooser-ro', ro)
            for _ in range(3):
                press(d, 'down')  # marker.txt, Projects/, user-letter
            press(d, 'ret')
            d.wait(lambda: d.serial().count('trusted chooser granted one file capability') == 2, 'no RO grant')
            time.sleep(0.3)
            d.q.type_text('Z', gap_s=.03)
            chord(d, 'ctrl', 's')
            time.sleep(1.5)
            assert tree(disk)[DOCS + 'user-letter'] == LETTER, 'read-only grant was written'
            print('[m11-files] read-only grant: filesd refuses the save, bytes exact PASS', flush=True)

            return b'pkg graphics\r'
        finally:
            d.dispose()

    def probe():
        d = Desktop(LABEL)
        try:
            d.wait(lambda: d.serial().count('trusted chooser opened') >= 4, 'probe never asked the chooser')
            before = d.shot('probe-chooser', lambda p: True)
            for _ in range(3):
                press(d, 'down')
            press(d, 'ret')
            d.wait(lambda: d.serial().count('trusted chooser granted one file capability') == 3, 'probe grant')
            # The probe paints green only when every hostile request was
            # answered as expected.
            green = d.shot('probe-verdict', lambda p: any(
                p[(y * 800 + x) * 3:(y * 800 + x) * 3 + 3] == b'\x20\xa0\x40'
                for y in range(60, 400, 4) for x in range(60, 500, 4)))
            assert not any(green[(y * 800 + x) * 3:(y * 800 + x) * 3 + 3] == b'\xc0\x20\x20'
                           for y in range(60, 400, 2) for x in range(60, 500, 2)), 'probe painted red'
            # Application death: closing the probe retires its lineage (its
            # head, the granted document and the attenuated re-open).
            retired = d.serial().count('filesd: lineage retired:')
            press(d, 'f8')
            d.wait(lambda: d.serial().count('filesd: lineage retired:') > retired, 'probe lineage not retired')
            last = re.findall(r'filesd: lineage retired: (\d+) record', d.serial())[-1]
            assert last == '3', last
            return b'shutdown\r'
        finally:
            d.dispose()

    feed = [((b'[desktop] real desktop frame presented', b'arena>', b'filesd: AFS2 formatted'), 1, workflow),
            (b'servicemgr: real signed Image delegated for broker-owned graphical spawn', 1, probe)]
    rc, s, _ = mtest.boot(LABEL, esp, feed, disk, pointer=True, timeout_s=240)
    assert rc == 0, s[-3000:]
    assert disk.read_bytes()[:AFS1_BYTES] == afs1_before or afs1.audit(disk) == []
    d1 = afs1.Disk(disk.read_bytes())
    _, ot, _ = d1.commit()
    for name, data in ((b'user-note', NOTE), (b'user-letter', LETTER)):
        o = d1.find(name, ot)
        got = b''.join(d1.data[a * 512:(a + n) * 512] for a, n in d1.extents(o['extent_head']))[:o['size']]
        assert got == data, (name, got)
    print(f'[m11-files] signed hostile probe ({len(elf)} B) granted one read-only document: every raw '
          'request beyond it refused by filesd PASS', flush=True)
    print('[m11-files] AFS1 originals untouched after migration PASS', flush=True)


if __name__ == '__main__':
    sys.exit(main())
