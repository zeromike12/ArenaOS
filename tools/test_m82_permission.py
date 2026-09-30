#!/usr/bin/env python3
"""Targeted Phase 8.2 VOLATILE integration guest proof, NOT phase closure.

Uses real virtio-rng, FS disk DMA, ring-3 app/manager/shell. Does not
claim persistent ALLOW or crash safety; same-disk reboot must deny.
"""
import sys
from pathlib import Path
sys.path.insert(0, str(Path(__file__).resolve().parent))
import arena_env  # noqa: E402
import mtest  # noqa: E402

LABEL = 'm82-permission-volatile'
def check(ok: bool, name: str) -> bool:
    print(f'[{LABEL}] {"PASS" if ok else "FAIL"}: {name}')
    return ok

def main() -> int:
    esp = mtest.build(LABEL)
    disk = arena_env.make_scratch_disk()
    feed = [
        ((b'permissiond READY', b'arena>'), 1, b'perm acquire\r'),
        (b'arena>', 2, b'perm allow-noauth\r'),
        (b'arena>', 3, b'perm allow-wrong\r'),
        (b'arena>', 4, b'perm allow\r'),
        ((b'permapp: PASS 512', b'arena>'), 1, b''),
        (b'arena>', 5, b'perm acquire\r'),
        (b'arena>', 6, b'perm retain\r'),
        (b'arena>', 7, b'perm acquire\r'),
        (b'arena>', 8, b'perm read-retained\r'),
        (b'arena>', 9, b'perm delegate\r'),
        (b'arena>', 10, b'perm revoke\r'),
        (b'arena>', 11, b'perm read-retained\r'),
        (b'arena>', 12, b'perm acquire\r'),
        (b'arena>', 13, b'shutdown\r'),
    ]
    rc, s, dt = mtest.boot(LABEL + '-exercise', esp, feed, disk)
    (arena_env.build_dir() / f'serial-{LABEL}-exercise.log').write_text(s)
    print(f'[{LABEL}] exercise rc={rc}, {dt:.1f}s')
    ok = check(rc == 0 and 'halting machine:' not in s and 'PANIC' not in s,
               'full-device boot completes cleanly')
    ok &= check('servicemgr: permission PING result + exit before deadline; worker reaped' in s
                and 'permissiond READY' in s and 'permission app reaped through held Process cap' in s,
                'actual two-grant worker witnesses reply and exit; manager owns app lifecycle')
    ok &= check('permapp: ACQUIRE denied (no approval)' in s
                and s.count('permission: ACQUIRE denied (no ALLOW)') >= 2
                and s.count('permission: admin marker refused by receiver') == 2,
                'request, endpoint and absent/wrong marker are not approval')
    ok &= check('permissiond: authorized VOLATILE 4' in s
                and 'permapp: PASS 512 mediated arena.txt bytes (no raw fsd cap)' in s,
                'genuine approval and fresh rngd bearer mediate exact 512-byte file')
    ok &= check(s.count('permission: ACQUIRE received 128-bit bearer') == 2
                and 'permission: independent bearer byte copy retained' in s
                and 'permission: READ verified 32 bytes from arena.txt via mediator' in s,
                'independently copied bearer survives another ACQUIRE')
    ok &= check(s.count('permapp: PASS 512 mediated') == 2
                and 'permission: delegated endpoint-only child reaped' in s,
                'intentional endpoint transfer to independent spawned client works')
    ok &= check('permissiond: authorized VOLATILE 6' in s
                and 'permission: READ refused old/missing bearer' in s,
                'revoke retires copied token bytes at receiver, not cap deletion')
    # No mkfs here: same platter as the ALLOW round.
    rc2, next_s, dt2 = mtest.boot(LABEL + '-reboot', esp,
        [(b'arena>', 1, b'perm acquire\r'), (b'arena>', 2, b'shutdown\r')], disk)
    (arena_env.build_dir() / f'serial-{LABEL}-reboot.log').write_text(next_s)
    print(f'[{LABEL}] same-disk reboot rc={rc2}, {dt2:.1f}s')
    ok &= check(rc2 == 0 and 'permissiond READY (volatile policy; default DENY)' in next_s
                and 'permission: ACQUIRE denied (no ALLOW)' in next_s,
                'VOLATILE ALLOW never claimed to survive a same-disk reboot')
    print(f'[{LABEL}] VOLATILE INTEGRATION: {"PASS" if ok else "FAIL"}; '
          'persistent/crash/ordering gates remain open')
    return 0 if ok else 1

if __name__ == '__main__':
    sys.exit(main())
