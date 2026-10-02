#!/usr/bin/env python3
"""Phase 8.2: cap-space/token bounds, bogus tokens, absent entropy and file."""
import sys
from pathlib import Path
sys.path.insert(0, str(Path(__file__).resolve().parent))
import arena_env  # noqa: E402
import mtest  # noqa: E402
import afs1  # noqa: E402
from test_m82_policy_restart import disk_records  # noqa: E402

LABEL = 'm82-policy-negative'
def check(ok, what):
    print(f'[{LABEL}] {"PASS" if ok else "FAIL"}: {what}')
    return ok

def main():
    esp = mtest.build(LABEL)
    disk = arena_env.make_scratch_disk()
    feed = [
        (b'arena>', 1, b'perm forge\r'),
        (b'arena>', 2, b'perm allow\r'),
        ((b'permapp: PASS 512', b'arena>'), 1, b''),
        (b'arena>', 3, b'perm forge\r'),
        (b'arena>', 4, b'perm acquire\r'),
        (b'arena>', 5, b'perm retain\r'),
        (b'arena>', 6, b'perm acquire\r'),
        (b'arena>', 7, b'perm acquire\r'),
        (b'arena>', 8, b'perm acquire\r'),
        (b'arena>', 9, b'perm read-retained\r'),
        (b'arena>', 10, b'perm revoke\r'),
        (b'arena>', 11, b'perm read-retained\r'),
        (b'arena>', 12, b'shutdown\r'),
    ]
    rc, s, dt = mtest.boot(f'{LABEL}-capacity', esp, feed, disk)
    (arena_env.build_dir() / f'serial-{LABEL}-capacity.log').write_text(s)
    print(f'[{LABEL}] capacity rc={rc} {dt:.1f}s')
    ok = check(rc == 0 and s.count('permission: forged 128-bit bearer refused by receiver') == 2
               and 'permission: ACQUIRE NO_SPACE (four live bearers; no eviction)' in s
               and s.count('permission: ACQUIRE received 128-bit bearer') == 3
               and 'permission: READ verified 32 bytes from arena.txt via mediator' in s
               and 'permission: READ refused old/missing bearer' in s
               and s.count('permapp: audited ONLY mediator WRITE|COPY, 31 other cap slots empty') >= 2
               and 'permissiond: audited four inherited caps FS/W RNG/W mediator/R marker/R; 28 extras empty' in s
               and 'permapp: endpoint-only ALLOW/DENY/REVOKE refused by receiver' in s
               and not afs1.audit(disk) and len(disk_records(disk)) == 2,
               'real rngd issuance, four-slot bearer table full refusal, independently copied token survives, invalid after revoke')
    # No entropy function with NIC and disk still attached: historical
    # link-only M6 may run, M7 stack truthfully SKIPs rather than waiting
    # forever for a rng-backed network bearer. Manager/root cannot
    # mint its permission sources, shell lacks endpoint/approval marker.
    rc, absent, dt = mtest.run_qemu(f'{LABEL}-no-rng', esp,
        feed=[(b'arena>', 1, b'perm allow\r'),
              (b'arena>', 2, b'perm acquire\r'),
              (b'arena>', 3, b'shutdown\r')], rng=False)
    (arena_env.build_dir() / f'serial-{LABEL}-no-rng.log').write_text(absent)
    print(f'[{LABEL}] no rng rc={rc} {dt:.1f}s')
    ok &= check(rc == 0 and 'm7: RESULT SKIP' in absent
                and 'servicemgr: OFFLINE — missing or invalid boot grants; no child spawned' in absent
                and 'permissiond READY' not in absent and 'permission: transport refused -2' in absent
                and 'permission: authorized durable' not in absent,
                'NIC+disk but no rngd: no broker, no bearer, no partial authority')
    # Receiver-verified diagnostic stops the ACTUAL production fsd;
    # existing broker token bytes then fail closed, and its replacement
    # cannot claim READY until the backend is restored on same-disk boot.
    disk = arena_env.make_scratch_disk()
    rc, missing, dt = mtest.boot(f'{LABEL}-fs-absent', esp, [
        (b'arena>', 1, b'perm allow\r'),
        (b'arena>', 2, b'perm acquire\r'),
        (b'arena>', 3, b'perm retain\r'),
        (b'arena>', 4, b'perm fs-stop\r'),
        (b'arena>', 5, b'perm acquire\r'),
        (b'arena>', 6, b'perm show\r'),
        (b'arena>', 7, b'perm read-retained\r'),
        (b'arena>', 8, b'perm restart\r'),
        (b'permission broker OFFLINE after failed restart', 1, b'perm show\r'),
        (b'arena>', 10, b'shutdown\r'),
    ], disk)
    (arena_env.build_dir() / f'serial-{LABEL}-fs-absent.log').write_text(missing)
    print(f'[{LABEL}] real fsd absence rc={rc} {dt:.1f}s')
    ok &= check(rc == 0 and 'fsd: shutdown requested' in missing
                and 'fsd root exited; process reaped, endpoint orphaned; filesystem OFFLINE' in missing
                and 'permission: trusted real fsd shutdown answered; backend now absent' in missing
                and 'permissiond: ACQUIRE lost validated FS backend/policy; no grant' in missing
                and 'permission: FS backend OFFLINE; decision unavailable, no grant' in missing
                and 'permission: READ refused old/missing bearer' in missing
                and 'permission: ACQUIRE backend IO/offline; no bearer issued' in missing
                and 'permission broker OFFLINE after failed restart' in missing
                and 'permissiond READY (validated durable decision; bearer table fresh)' in missing
                and not afs1.audit(disk) and len(disk_records(disk)) == 1,
                'real production fsd absent: no stale ALLOW bearer, no new issuance/false replacement READY')
    if ok:
        rc, restored, dt = mtest.boot(f'{LABEL}-fs-restored', esp, [
            (b'arena>', 1, b'perm show\r'),
            (b'arena>', 2, b'perm acquire\r'),
            (b'arena>', 3, b'perm read\r'),
            (b'arena>', 4, b'shutdown\r'),
        ], disk)
        (arena_env.build_dir() / f'serial-{LABEL}-fs-restored.log').write_text(restored)
        print(f'[{LABEL}] same-platter fsd restored rc={rc} {dt:.1f}s')
        ok &= check(rc == 0 and 'permissiond: validated durable policy generation 1 ALLOW' in restored
                    and 'permission: READ verified 32 bytes from arena.txt via mediator' in restored
                    and not afs1.audit(disk) and len(disk_records(disk)) == 1,
                    'normal same-disk restart recovers fsd and policy but NOT old bearer bytes')
    print(f'[{LABEL}] POLICY NEGATIVE SPACE: {"PASS" if ok else "FAIL"}')
    return 0 if ok else 1

if __name__ == '__main__':
    sys.exit(main())
