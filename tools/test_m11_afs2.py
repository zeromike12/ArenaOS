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
   day of the host clock); with no usable clock (injected at the kernel's
   RTC reader) filesd says "unknown" and every timestamp is 0, never
   invented.
"""
import shutil
import sys
import time
import afs1
import afs2
import arena_env
import mtest
import test_m84_stage as stage

BUILD = arena_env.build_dir()
LABEL = 'm11-afs2'
BASE = arena_env.AFS2_BASE_SECTOR * 512
AFS1_BYTES = arena_env.SCRATCH_MIB * 1024 * 1024
FILES = {
    b'user-note': b'migrated user note\n',
    b'user-big': bytes((i * 7 + 3) % 256 for i in range(10000)),  # three import transfers
    b'ui10-prefs': b'UI10\x01\x01\x00',
    b'service-ledger': b'system record kept out of the user tree',
}
MARKER = b'filesd: AFS2 mounted seq '
DONE = b'filesd: AFS2 formatted; AFS1 import complete: '
INTERRUPTED = 'filesd: AFS1 import was interrupted'


def seeded(esp):
    """A real AFS1 volume (its seed boot leaves the boot-time fs self-test
    file and whatever services record), plus FILES, then the blank AFS2
    region appended."""
    disk = arena_env.make_scratch_disk()
    rc, s, _ = mtest.boot(f'{LABEL}-seed', esp, [(b'arena>', 1, b'shutdown\r')], disk, pointer=True)
    assert rc == 0 and 'no AFS2 region' in s, s[-2000:]
    stage.host_seed(disk, FILES)
    with open(disk, 'r+b') as f:
        f.truncate(arena_env.AFS2_DISK_MIB * 1024 * 1024)
    assert afs1.audit(disk) == []
    return disk


def region(disk):
    return disk.read_bytes()[BASE:]


def expected(afs1_files):
    """The migration of exactly these AFS1 files."""
    # Phase 12 format-time infrastructure directories are present on every
    # freshly formatted AFS2 volume, whether or not a package was installed.
    tree = {'/': None, '/System': None, '/System/.apb1-staging': None,
            '/System/Applications': None, '/System/imported-afs1': None,
            '/System/afs1-import-complete': b'', '/Users': None, '/Users/user': None,
            '/Users/user/Desktop': None, '/Users/user/Documents': None, '/Users/user/.Trash': None}
    for name, data in afs1_files.items():
        where = '/Users/user/Documents/' if name.startswith(b'user-') else '/System/imported-afs1/'
        tree[where + name.decode()] = data
    return tree


def mounted(disk):
    vol = afs2.Volume(region(disk))
    afs2.check(vol)
    return vol, afs2.walk(vol)


def boot(label, esp, disk, kill=None, extra=(), ready=MARKER):
    # Shut down only once filesd has reported its mount (or `ready`).
    feed = [] if kill else [((ready, b'arena>'), 1, b'shutdown\r')]
    return mtest.boot(label, esp, feed, disk, kill=kill, pointer=True, extra_args=list(extra))


def times(vol):
    out = []
    for path in afs2.walk(vol):
        st = vol.stat(vol.resolve(path))
        out += [st['ctime'], st['mtime']]
    return out


def main():
    global AFS1
    esp = mtest.build(LABEL, desktop=True)
    disk = seeded(esp)
    AFS1 = stage.contents(disk)
    users = sum(n.startswith(b'user-') for n in AFS1)
    afs1_before = disk.read_bytes()[:AFS1_BYTES]
    pristine = BUILD / f'{LABEL}-seed.img'
    shutil.copyfile(disk, pristine)

    # 1. Migration.
    rc, s, _ = boot(f'{LABEL}-migrate', esp, disk)
    assert rc == 0, s[-3000:]
    assert (f'AFS1 import complete: {users} user file(s) to /Users/user/Documents, '
            f'{len(AFS1) - users} system record(s)') in s, s[-3000:]
    assert 'wall clock from RTC' in s
    writes = s.count('storaged: BLKW4K')
    assert disk.read_bytes()[:AFS1_BYTES] == afs1_before, 'AFS1 mutated by the migration'
    vol, tree = mounted(disk)
    assert tree == expected(AFS1), sorted(tree)
    host = time.time() * 1e6
    stamps = times(vol)
    assert all(abs(t - host) < 86400e6 for t in stamps), ('timestamps not from the RTC', stamps[:4], host)
    migrated = region(disk)
    print(f'[m11-afs2] migration: {len(AFS1)} AFS1 files imported, {writes} AFS2 block writes, '
          'host-audited namespace and bytes exact, AFS1 byte-identical, RTC timestamps PASS', flush=True)

    # 2. Remount writes nothing.
    rc, s, _ = boot(f'{LABEL}-remount', esp, disk)
    assert rc == 0 and MARKER.decode() in s and DONE.decode() not in s and INTERRUPTED not in s, s[-3000:]
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
        assert (INTERRUPTED in s) == (state != 'never committed'), (label, state)
        assert mounted(disk)[1] == expected(AFS1)
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
    rc, s, _ = boot(f'{LABEL}-corrupt', esp, disk, ready=b'file service offline')
    assert rc == 0 and 'refused to mount (CORRUPT) - fail closed, never repaired' in s, s[-3000:]
    assert region(disk) == damaged and s.count('storaged: BLKW4K') == 0
    print('[m11-afs2] damaged committed volume refused, never repaired or formatted PASS', flush=True)

    # 5. No usable clock: unknown, never invented. The firmware rewrites an
    # out-of-range CMOS date (QEMU -rtc base=1990 boots as 2090), so the
    # absent clock is injected at the kernel's one RTC reader instead; the
    # source and artifacts are restored byte-exactly afterwards.
    rtc = arena_env.REPO_ROOT / 'kernel/kernel/src/rtc.rs'
    original = rtc.read_bytes()
    needle = b'pub fn read_unix_seconds() -> Option<u64> {\n'
    assert original.count(needle) == 1
    artifacts = {p: p.read_bytes() for p in (esp, BUILD / 'arena-boot.efi')}
    try:
        rtc.write_bytes(original.replace(needle, needle + b'    if true { return None; }\n', 1))
        mutant = mtest.build(f'{LABEL}-no-clock', desktop=True)
        disk = seeded(mutant)
        AFS1 = stage.contents(disk)
        rc, s, _ = boot(f'{LABEL}-no-clock', mutant, disk)
    finally:
        rtc.write_bytes(original)
        mtest.build(f'{LABEL}-restored', desktop=True)
    assert rtc.read_bytes() == original
    # The kernel image is deterministic (kernel/.cargo/config.toml): the
    # restored source rebuilds the identical EFI. The ESP's FAT entry
    # carries the file time, so its original bytes are put back.
    assert (BUILD / 'arena-boot.efi').read_bytes() == artifacts[BUILD / 'arena-boot.efi'], 'restored EFI differs'
    esp.write_bytes(artifacts[esp])
    assert rc == 0 and 'wall clock unknown' in s, s[-3000:]
    vol, tree = mounted(disk)
    assert tree == expected(AFS1) and set(times(vol)) == {0}
    print('[m11-afs2] no clock: "unknown" and zero timestamps, migration still exact; source/EFI restored '
          'byte-exact PASS', flush=True)

if __name__ == '__main__':
    sys.exit(main())
