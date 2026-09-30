#!/usr/bin/env python3
"""ADR-0052: linked no_std clients in four independent real ring-3 images.

fstest and permissiond use the AFS1 client; arptest and the shell use
native net. Both clients traverse the SAME checked syscall/IPC rlib.
"""
from pathlib import Path
import sys
sys.path.insert(0, str(Path(__file__).resolve().parent))
import afs1  # noqa: E402
import arena_env  # noqa: E402
import mtest  # noqa: E402
import permission_record as policy  # noqa: E402
from test_m82_policy_restart import disk_records  # noqa: E402

LABEL = 'm83-libraries'

def check(ok, what):
    print(f'[{LABEL}] {"PASS" if ok else "FAIL"}: {what}')
    return ok

def main():
    esp = mtest.build(LABEL)
    disk = arena_env.make_scratch_disk()
    rc, s, elapsed = mtest.boot(LABEL + '-linked', esp, [
        (b'arena>', 1, b'netlib\r'),
        (b'arena>', 2, b'perm allow\r'),
        (b'arena>', 3, b'perm acquire\r'),
        (b'arena>', 4, b'perm read\r'),
        (b'arena>', 5, b'perm revoke\r'),
        (b'arena>', 6, b'shutdown\r'),
    ], disk)
    (arena_env.build_dir() / f'serial-{LABEL}-linked.log').write_text(s)
    print(f'[{LABEL}] linked guests rc={rc} {elapsed:.1f}s')
    net = s.rfind('netstackd: resolved 10.0.2.2', 0, s.find('m83: netlib PASS'))
    ok = check(rc == 0 and 'm7: RESULT PASS (2/2)' in s
               and 'm83: netlib wrong-kind Image endpoint refused by kernel' in s
               and 'arptest: PASS — native UDP API bound' in s and net >= 0
               and 'm83: netlib PASS (linked client, live gateway ARP)' in s,
               'arptest and shell separately link the native client and reach live network stack/wire')
    ok &= check('fstest: PASS (fresh)' in s
                and 'fstest: fsd reported 34 disk operation(s); storaged reported 34 completion(s)' in s
                and 'm5: RESULT PASS (6/6)' in s
                and 'permissiond: durable decision 4 generation 1' in s
                and 'permission: READ verified 32 bytes from arena.txt via mediator' in s
                and 'permissiond: durable decision 6 generation 2' in s
                and 'permission: READ refused old/missing bearer' not in s
                and policy.recover(disk_records(disk)).current.payload == b'\x01\x01\x00\x00'
                and not afs1.audit(disk),
                'fstest and broker link the FS client; real DMA, unchanged 34-operation contract, durable ALLOW/REVOKE')
    rc, offline, elapsed = mtest.run_qemu(LABEL + '-no-devices', esp,
        feed=[(b'arena>', 1, b'netlib\r'), (b'arena>', 2, b'shutdown\r')],
        net=False, rng=False, kbd=False, vcon=False, tcp_peer=False)
    (arena_env.build_dir() / f'serial-{LABEL}-no-devices.log').write_text(offline)
    print(f'[{LABEL}] absent devices rc={rc} {elapsed:.1f}s')
    ok &= check(rc == 0 and 'm83: netlib wrong-kind Image endpoint refused by kernel' in offline
                and 'm83: netlib SKIP (no stack endpoint)' in offline
                and 'm83: netlib PASS' not in offline
                and 'servicemgr: OFFLINE' in offline,
                'no stack cap yields honest SKIP, not name/pid-based or invented wire access')
    print(f'[{LABEL}] LINKED NO_STD LIBRARIES: {"PASS" if ok else "FAIL"}')
    return 0 if ok else 1

if __name__ == '__main__': sys.exit(main())
