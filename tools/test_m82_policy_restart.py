#!/usr/bin/env python3
"""ADR-0048 real held-endpoint ALLOW/DENY restart + same-platter rehydration.

Read the COMMITTED disk independently, not a broker log. No token bytes are
persisted; first app/image request without the transferred endpoint fails.
"""
import re
import sys
from pathlib import Path
sys.path.insert(0, str(Path(__file__).resolve().parent))
import afs1  # noqa: E402
import arena_env  # noqa: E402
import mtest  # noqa: E402
import permission_record as policy  # noqa: E402

LABEL = 'm82-policy-restart'
SNAP = r'permission: resource snapshot free/records/processes (\d+)/(\d+)/(\d+)'

def check(ok, what):
    print(f'[{LABEL}] {"PASS" if ok else "FAIL"}: {what}')
    return ok

def disk_records(path):
    d = afs1.Disk(path.read_bytes())
    _, ot, _ = d.commit()
    result = []
    for obj in d.objects(ot):
        if obj['type'] != afs1.OBJ_FILE or not obj['name'].startswith(b'perm8-'):
            continue
        data = b''.join(d.sector(s) for base, length in d.extents(obj['extent_head'])
                        for s in range(base, base + length))[:obj['size']]
        result.append((obj['name'].decode(), data))
    return result

def boot(esp, disk, tag, feed):
    rc, serial, sec = mtest.boot(f'{LABEL}-{tag}', esp, feed, disk)
    (arena_env.build_dir() / f'serial-{LABEL}-{tag}.log').write_text(serial)
    print(f'[{LABEL}] {tag}: rc={rc} {sec:.1f}s')
    return rc, serial

def main():
    esp = mtest.build(LABEL)
    disk = arena_env.make_scratch_disk()
    rc, s = boot(esp, disk, 'restart-both', [
        (b'arena>', 1, b'perm snapshot\r'),
        (b'arena>', 2, b'perm allow\r'),
        (b'arena>', 3, b'perm acquire\r'),
        (b'arena>', 4, b'perm retain\r'),
        (b'arena>', 5, b'perm name-only\r'),
        (b'arena>', 6, b'perm restart\r'),
        ((b'permission broker reaped', b'permissiond: validated durable policy generation 1 ALLOW'), 1, b'perm read-retained\r'),
        (b'arena>', 8, b'perm acquire\r'),
        (b'arena>', 9, b'perm read\r'),
        (b'arena>', 10, b'perm deny\r'),
        (b'arena>', 11, b'perm restart\r'),
        ((b'permissiond: validated durable policy generation 2 DENY',), 1, b'perm show\r'),
        (b'arena>', 13, b'perm acquire\r'),
        (b'arena>', 14, b'perm read-retained\r'),
        (b'arena>', 15, b'perm snapshot\r'),
        (b'arena>', 16, b'shutdown\r'),
    ])
    rec = disk_records(disk)
    ok = check(rc == 0 and not afs1.audit(disk)
               and policy.recover(rec).current.payload == b'\x01\x01\x00\x00',
               'two exact independent immutable records, final DENY on valid AFS1 platter')
    ok &= check([policy.unpack(raw, i).payload for i, (_, raw) in enumerate(sorted(rec), 1)] ==
                [b'\x01\x01\x01\x00', b'\x01\x01\x00\x00'],
                'host decoded byte-exact ALLOW then DENY; no duplicate or unnamed authority')
    ok &= check('permapp: name-only request refused without mediator endpoint' in s
                and 'permission: name-only child reaped without mediator authority' in s,
                'name-only app request under ALLOW cannot call ACQUIRE without endpoint')
    ok &= check(s.count('permission broker reaped; old bearer retired; durable policy rescanned') == 2
                and 'permissiond: validated durable policy generation 1 ALLOW' in s
                and 'permissiond: validated durable policy generation 2 DENY' in s
                and s.count('permission PING result + exit before deadline; worker reaped') == 3,
                'two actual Process-cap reaps, replacements on original endpoint, PING readiness')
    ok &= check('permission: READ refused old/missing bearer' in s
                and s.count('permission: ACQUIRE received 128-bit bearer') == 2
                and 'permission: READ verified 32 bytes from arena.txt via mediator' in s
                and 'permission: ACQUIRE denied (no ALLOW)' in s,
                'old copied token revoked on restart; held endpoint reissues under ALLOW, refuses under DENY')
    snapshots = re.findall(SNAP, s)
    ok &= check(len(snapshots) == 2 and snapshots[0] == snapshots[1],
                f'after both broker restarts frames/records/processes exact: {snapshots}')
    if not ok: return 1
    rc, s = boot(esp, disk, 'reboot-deny', [
        (b'arena>', 1, b'perm show\r'), (b'arena>', 2, b'perm acquire\r'),
        (b'arena>', 3, b'perm allow\r'), (b'arena>', 4, b'perm show\r'),
        (b'arena>', 5, b'shutdown\r'),
    ])
    rec = disk_records(disk)
    ok &= check(rc == 0 and 'validated durable policy generation 2 DENY' in s
                and 'permission: ACQUIRE denied (no ALLOW)' in s
                and 'permission: request arena.txt READ; durable decision ALLOW' in s
                and policy.recover(rec).current.payload == b'\x01\x01\x01\x00'
                and len(rec) == 3 and not afs1.audit(disk),
                'same-platter DENY on reboot; genuine marker commits ALLOW generation 3')
    rc, s = boot(esp, disk, 'reboot-allow', [
        (b'arena>', 1, b'perm acquire\r'), (b'arena>', 2, b'perm read\r'),
        (b'arena>', 3, b'perm delegate\r'), (b'arena>', 4, b'perm revoke\r'),
        (b'arena>', 5, b'perm show\r'), (b'arena>', 6, b'perm acquire\r'),
        (b'arena>', 7, b'shutdown\r'),
    ])
    rec = disk_records(disk)
    ok &= check(rc == 0 and 'validated durable policy generation 3 ALLOW' in s
                and 'permission: READ verified 32 bytes from arena.txt via mediator' in s
                and 'permission: delegated endpoint-only child reaped' in s
                and 'permission: ACQUIRE denied (no ALLOW)' in s
                and policy.recover(rec).current.payload == b'\x01\x01\x00\x00'
                and len(rec) == 4 and not afs1.audit(disk),
                'same-platter ALLOW issues only fresh bearer; delegated endpoint, durable REVOKE')
    print(f'[{LABEL}] PERSISTENT RESTART: {"PASS" if ok else "FAIL"}')
    return 0 if ok else 1

if __name__ == '__main__':
    sys.exit(main())
