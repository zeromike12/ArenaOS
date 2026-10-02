#!/usr/bin/env python3
"""ADR-0048 real bounded generation, allocator failure and corrupt-namespace gates."""
import shutil
import sys
from pathlib import Path
sys.path.insert(0, str(Path(__file__).resolve().parent))
import afs1  # noqa: E402
import arena_env  # noqa: E402
import mtest  # noqa: E402
import permission_record as policy  # noqa: E402
from test_m82_policy_restart import disk_records  # noqa: E402

LABEL = 'm82-policy-refusal'
ALLOW = b'\x01\x01\x01\x00'
DENY = b'\x01\x01\x00\x00'

def check(ok, why):
    print(f'[{LABEL}] {"PASS" if ok else "FAIL"}: {why}')
    return ok

def boot(esp, disk, tag, commands):
    rc, serial, dt = mtest.boot(f'{LABEL}-{tag}', esp, commands, disk)
    (arena_env.build_dir() / f'serial-{LABEL}-{tag}.log').write_text(serial)
    print(f'[{LABEL}] {tag}: rc={rc} {dt:.1f}s')
    return rc, serial

def main():
    esp = mtest.build(LABEL)
    disk = arena_env.make_scratch_disk()
    rc, serial = boot(esp, disk, 'eight', [
        (b'arena>', i, b'perm ' + (b'allow' if i % 2 else b'deny') + b'\r')
        for i in range(1, 9)] + [
            (b'arena>', 9, b'perm allow\r'),
            (b'arena>', 10, b'perm show\r'),
            (b'arena>', 11, b'shutdown\r'),
        ])
    rec = sorted(disk_records(disk))
    ok = check(rc == 0 and not afs1.audit(disk) and len(rec) == 8
               and rec == [(policy.name(i), policy.pack(i, ALLOW if i % 2 else DENY))
                           for i in range(1, 9)]
               and serial.count('permission: authorized durable decision generation') == 8
               and 'permission: admin NO_SPACE (no durable acknowledgement)' in serial
               and 'permission: request arena.txt READ; durable decision DENY' in serial
               and not any(name.startswith('perm8-0') and name not in [policy.name(i) for i in range(1, 9)]
                           for name, _ in rec),
               'eight guest-issued immutable decisions exact; ninth refuses without minting a record')
    if not ok: return 1
    table = arena_env.build_dir() / 'm82-policy-eight-complete.img'
    shutil.copyfile(disk, table)
    # A valid AFS1 host-prepared allocator-full fixture (no forged driver
    # status). fsd can commit an empty CREATE after reclaim, but cannot
    # allocate a data sector. Previous validated ALLOW remains byte-exact.
    base = arena_env.make_scratch_disk()
    rc, s = boot(esp, base, 'disk-base', [(b'arena>', 1, b'perm allow\r'),
                                         (b'arena>', 2, b'shutdown\r')])
    ok &= check(rc == 0 and policy.recover(disk_records(base)).current.payload == ALLOW,
                'guest created real ALLOW before allocator exhaustion')
    if not ok: return 1
    full = arena_env.build_dir() / 'm82-policy-allocator-full.img'
    shutil.copyfile(base, full)
    raw = bytearray(full.read_bytes())
    _, _, bitmap = afs1.Disk(bytes(raw)).commit()
    offset = bitmap * afs1.SECTOR
    raw[offset:offset + afs1.BITMAP_SECTORS * afs1.SECTOR] = b'\xff' * (afs1.BITMAP_SECTORS * afs1.SECTOR)
    full.write_bytes(raw)
    ok &= check(not afs1.audit(full), 'allocator-full platter audits at AFS1 level before guest boot')
    rc, s = boot(esp, full, 'disk-full', [
        (b'arena>', 1, b'perm acquire\r'),
        (b'arena>', 2, b'perm retain\r'),
        (b'arena>', 3, b'perm revoke\r'),
        (b'arena>', 4, b'perm read-retained\r'),
        (b'arena>', 5, b'perm allow\r'),
        (b'arena>', 6, b'perm show\r'),
        (b'arena>', 7, b'shutdown\r'),
    ])
    records = dict(disk_records(full))
    ok &= check(rc == 0 and 'permission: admin NO_SPACE (no durable acknowledgement)' in s
                and 'permission: admin DEGRADED (no durable acknowledgement)' in s
                and 'permission: policy DEGRADED; no grant' in s
                and 'permission: READ refused old/missing bearer' in s
                and 'permissiond: durable decision 6 generation 2' not in s
                and records['perm8-01'] == policy.pack(1, ALLOW)
                and records.get('perm8-02') in (None, b'') and not afs1.audit(full),
                'real disk-full WRITE: typed NO_SPACE then DEGRADED; in-RAM fail-close and byte-exact old ALLOW')
    if not ok: return 1
    # Mutate the newest *data* sector offline, not the AFS1 commit.
    # Old ALLOW at seq1 must NEVER be selected when seq2 is visible bad.
    damaged = arena_env.build_dir() / 'm82-policy-visible-corrupt.img'
    shutil.copyfile(table, damaged)
    raw = bytearray(damaged.read_bytes())
    d = afs1.Disk(bytes(raw))
    _, ot, _ = d.commit()
    newest = d.find(b'perm8-08', ot)
    assert newest is not None and newest['size'] == 512
    block = d.extents(newest['extent_head'])[0][0]
    raw[block * 512 + 34] ^= 0x02
    damaged.write_bytes(raw)
    try:
        policy.recover(disk_records(damaged))
        valid = True
    except policy.PolicyCorrupt:
        valid = False
    ok &= check(not afs1.audit(damaged) and not valid,
                'visible record corruption is outside AFS1 metadata integrity, not an invisible commit rollback')
    rc, s = boot(esp, damaged, 'visible-corrupt', [
        (b'arena>', 1, b'perm show\r'), (b'arena>', 2, b'shutdown\r'),
    ])
    ok &= check(rc == 0 and 'permissiond: visible policy record CORRUPT; no READY, no old ALLOW' in s
                and 'permissiond READY' not in s
                and 'permission: broker OFFLINE/SHOW transport refused' in s
                and not afs1.audit(damaged),
                'broker refuses visible malformed newest record rather than falling back to older ALLOW')
    print(f'[{LABEL}] POLICY BOUNDS/FAIL-CLOSED: {"PASS" if ok else "FAIL"}')
    return 0 if ok else 1

if __name__ == '__main__':
    sys.exit(main())
