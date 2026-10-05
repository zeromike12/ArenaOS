#!/usr/bin/env python3
"""Phase 11.5: AFS2 in the guest (ADR-0076/0077), judged on the host.

1. Migration: an AFS1 volume with user and system files and a blank AFS2
   region. filesd formats the region and imports AFS1 read only: `user-*`
   files to /Users/user/Documents, the rest to /System/imported-afs1, the
   marker /System/afs1-import-complete committed last. The host model
   (tools/afs2.py) mounts the region, audits it and finds exactly that
   namespace with the exact bytes; the AFS1 sectors are byte-identical.
2. Remount: a second boot mounts and writes nothing.
3. Interrupted import: boots killed after the n-th AFS2 block write
   (`storaged: BLKW4K`, issued only by filesd). Each kill must have
   happened inside the import (n writes logged, no completion line), or
   the round FAILS (no retry). The crashed region is either never
   committed or mounts and audits clean without the marker; the next boot
   imports again to exactly the clean namespace; AFS1 never changes.
4. Fail closed: a committed volume with both commit records destroyed is
   refused, not repaired or formatted: its bytes do not change.
5. Wall clock: with the CMOS RTC in range timestamps are real (within a
   day of the host clock); with it out of range (1990) filesd says
   "unknown" and every timestamp is 0, never invented.
"""
import shutil
import sys
import time
import afs1
import afs2
import arena_env
import mtest

BUILD = arena_env.build_dir()
LABEL = 'm11-afs2'
BASE = arena_env.AFS2_BASE_SECTOR * 512
AFS1_BYTES = arena_env.SCRATCH_MIB * 1024 * 1024
FILES = {
    b'user-note': b'migrated user note\n',
    b'user-big': bytes((i * 7 + 3) % 256 for i in range(10000)),  # three import transfers
    b'user-empty': b'',
    b'ui10-prefs': b'UI10\x01\x01\x00',
    b'service-ledger': b'system record kept out of the user tree',
}
MARKER = b'filesd: AFS2 mounted seq '
DONE = b'filesd: AFS2 formatted; AFS1 import complete: '


def seeded():
    disk = arena_env.make_scratch_disk(afs2=True)
    afs1.mkfs_with_files(disk, AFS1_BYTES // afs1.SECTOR, FILES)
    with open(disk, 'r+b') as f:
        f.truncate(arena_env.AFS2_DISK_MIB * 1024 * 1024)
    assert afs1.audit(disk) == []
    return disk


def region(disk):
    return disk.read_bytes()[BASE:]


def expected():
    tree = {'/': None, '/System': None, '/System/imported-afs1': None,
            '/System/afs1-import-complete': b'', '/Users': None, '/Users/user': None,
            '/Users/user/Desktop': None, '/Users/user/Documents': None, '/Users/user/.Trash': None}
    for name, data in FILES.items():
        where = '/Users/user/Documents/' if name.startswith(b'user-') else '/System/imported-afs1/'
        tree[where + name.decode()] = data
    return tree


def mounted(disk):
    vol = afs2.Volume(region(disk))
    afs2.check(vol)
    return vol, afs2.walk(vol)


def boot(label, esp, disk, kill=None, extra=()):
    # Shut down only once filesd has reported its mount.
    feed = [] if kill else [((MARKER, b'arena>'), 1, b'shutdown\r')]
    return mtest.boot(label, esp, feed, disk, kill=kill, pointer=True, extra_args=list(extra))


def times(vol):
    out = []
    for path in afs2.walk(vol):
        st = vol.stat(vol.resolve(path))
        out += [st['ctime'], st['mtime']]
    return out


def main():
    esp = mtest.build(LABEL, desktop=True)
    disk = seeded()
    afs1_before = disk.read_bytes()[:AFS1_BYTES]
    pristine = BUILD / f'{LABEL}-seed.img'
    shutil.copyfile(disk, pristine)

    # 1. Migration.
    rc, s, _ = boot(f'{LABEL}-migrate', esp, disk)
    assert rc == 0, s[-3000:]
    assert 'AFS1 import complete: 3 user file(s) to /Users/user/Documents, 2 system record(s)' in s, s[-3000:]
    assert 'wall clock from RTC' in s
    writes = s.count('storaged: BLKW4K')
    assert disk.read_bytes()[:AFS1_BYTES] == afs1_before, 'AFS1 mutated by the migration'
    vol, tree = mounted(disk)
    assert tree == expected(), sorted(tree)
    host = time.time() * 1e6
    stamps = times(vol)
    assert all(abs(t - host) < 86400e6 for t in stamps), ('timestamps not from the RTC', stamps[:4], host)
    migrated = region(disk)
    print(f'[m11-afs2] migration: {len(FILES)} AFS1 files imported, {writes} AFS2 block writes, '
          'host-audited namespace and bytes exact, AFS1 byte-identical, RTC timestamps PASS', flush=True)

    # 2. Remount writes nothing.
    rc, s, _ = boot(f'{LABEL}-remount', esp, disk)
    assert rc == 0 and MARKER.decode() in s and DONE.decode() not in s and 'interrupted' not in s, s[-3000:]
    assert s.count('storaged: BLKW4K') == 0 and region(disk) == migrated
    assert disk.read_bytes()[:AFS1_BYTES] == afs1_before
    print('[m11-afs2] second boot mounts the committed volume with zero writes PASS', flush=True)

    # 3. Interrupted import at several points of its write sequence.
    points = sorted({1, 2, 3, writes // 4, writes // 2, (writes * 3) // 4})
    assert writes >= 16 and points[-1] < writes - 2, (writes, points)
    for n in points:
        shutil.copyfile(pristine, disk)
        label = f'{LABEL}-crash-{n}'
        rc, s, _ = boot(label, esp, disk, kill=(b'storaged: BLKW4K', n, 0.0))
        logged = s.count('storaged: BLKW4K')
        # The intended trigger: killed inside the import, after >= n writes.
        assert rc is None and logged >= n and DONE.decode() not in s, (label, rc, logged)
        assert disk.read_bytes()[:AFS1_BYTES] == afs1_before
        img = region(disk)
        if afs2.never_committed(img):
            state = 'never committed'
        else:
            vol, tree = mounted(disk)
            assert '/System/afs1-import-complete' not in tree, (label, sorted(tree))
            state = f'committed seq {vol.seq} without marker'
        rc, s, _ = boot(f'{label}-recover', esp, disk)
        assert rc == 0 and DONE.decode() in s, s[-3000:]
        assert ('import was interrupted' in s) == (state != 'never committed'), (label, state)
        assert mounted(disk)[1] == expected()
        assert disk.read_bytes()[:AFS1_BYTES] == afs1_before
        print(f'[m11-afs2] crash after BLKW4K #{n} (logged {logged}/{writes}): {state}; '
              'next boot re-imported to the exact namespace PASS', flush=True)

    # 4. Fail closed: destroy both commit records of the committed volume.
    shutil.copyfile(pristine, disk)
    with open(disk, 'r+b') as f:
        f.seek(BASE)
        f.write(migrated)
        for slot in (1, 2):
            f.seek(BASE + slot * afs2.BLOCK)
            f.write(b'\xee' * 64)
    damaged = region(disk)
    assert not afs2.never_committed(damaged)
    rc, s, _ = boot(f'{LABEL}-corrupt', esp, disk)
    assert rc == 0 and 'refused to mount (CORRUPT) - fail closed, never repaired' in s, s[-3000:]
    assert region(disk) == damaged and s.count('storaged: BLKW4K') == 0
    print('[m11-afs2] damaged committed volume refused, never repaired or formatted PASS', flush=True)

    # 5. Out-of-range RTC: unknown, never invented.
    disk = seeded()
    rc, s, _ = boot(f'{LABEL}-no-clock', esp, disk, extra=['-rtc', 'base=1990-06-01T00:00:00'])
    assert rc == 0 and 'wall clock unknown' in s, s[-3000:]
    vol, tree = mounted(disk)
    assert tree == expected() and set(times(vol)) == {0}
    print('[m11-afs2] RTC out of range: "unknown" and zero timestamps, migration still exact PASS', flush=True)


if __name__ == '__main__':
    sys.exit(main())
