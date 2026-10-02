#!/usr/bin/env python3
"""ADR-0048: kill real guest at ALLOW, DENY and REVOKE policy-write boundaries.

Audit the COMMITTED platter BEFORE recovery. No false assertion of
commit-sector-corruption or rollback resistance (AFS1 model only).
"""
import shutil
import sys
from pathlib import Path
sys.path.insert(0, str(Path(__file__).resolve().parent))
import afs1  # noqa: E402
import arena_env  # noqa: E402
import mtest  # noqa: E402
import permission_record as policy  # noqa: E402
from test_m82_policy_restart import disk_records  # noqa: E402

LABEL = 'm82-policy-crash'
ALLOW = b'\x01\x01\x01\x00'
DENY = b'\x01\x01\x00\x00'

def check(ok, what):
    print(f'[{LABEL}] {"PASS" if ok else "FAIL"}: {what}')
    return ok

def boot(esp, disk, tag, feed=(), kill=None):
    rc, s, elapsed = mtest.boot(f'{LABEL}-{tag}', esp, list(feed), disk, kill=kill)
    (arena_env.build_dir() / f'serial-{LABEL}-{tag}.log').write_text(s)
    print(f'[{LABEL}] {tag}: rc={rc}, {elapsed:.1f}s')
    return rc, s

def run_scenario(esp, scenario, old_cmd, new_cmd, old_payload, new_payload):
    # Each transition gets its own guest-created original decision and an
    # independently audited baseline platter. Never synthesize the old bytes.
    disk = arena_env.make_scratch_disk()
    rc, s = boot(esp, disk, f'{scenario}-baseline', [(b'arena>', 1, b'perm ' + old_cmd + b'\r'),
                                         (b'arena>', 2, b'shutdown\r')])
    ok = check(rc == 0 and f'permissiond: durable decision {4 if old_cmd == b"allow" else 5} generation 1' in s
               and policy.recover(disk_records(disk)).current.payload == old_payload
               and not afs1.audit(disk), f'{scenario}: independently audited, guest-committed old decision before fault')
    if not ok: return False
    baseline = arena_env.build_dir() / f'm82-policy-crash-{scenario}-base.img'
    shutil.copyfile(disk, baseline)
    markers = [
        ('precreate', b'permissiond: POLICY CREATE submitted', (None, b'', new_payload)),
        ('empty', b'permissiond: POLICY CREATE committed empty generation', (b'', new_payload)),
        ('prewrite', b'permissiond: POLICY WRITE submitted', (b'', new_payload)),
        ('write-reply', b'permissiond: POLICY WRITE committed (fsd replied)', (new_payload,)),
        ('close', b'permissiond: POLICY CLOSE completed', (new_payload,)),
        ('rescan', b'permissiond: POLICY exact committed decision verified before reply', (new_payload,)),
        ('ack', b'permission: authorized durable decision generation 2', (new_payload,)),
    ]
    for tag, marker, legal in markers:
        scratch = arena_env.build_dir() / f'm82-policy-crash-{scenario}-{tag}.img'
        shutil.copyfile(baseline, scratch)
        rc, killed = boot(esp, scratch, f'kill-{scenario}-{tag}', [(b'arena>', 1, b'perm ' + new_cmd + b'\r')],
                          kill=(marker, 1, 0.0))
        records = dict(disk_records(scratch))
        recovered = policy.recover(list(records.items()))
        new_raw = records.get('perm8-02')
        got = (None if new_raw is None else b'' if not new_raw
               else policy.unpack(new_raw, 2).payload)
        ok &= check(rc is None and marker.decode() in killed and not afs1.audit(scratch)
                    and records['perm8-01'] == policy.pack(1, old_payload)
                    and got in legal
                    and recovered.current.payload == (new_payload if got == new_payload else old_payload),
                    f'{scenario}/{tag}: SIGKILL at actual boundary; committed disk audits and no old-byte mutation')
        if not ok: break
        expected = 'ALLOW' if (new_payload if got == new_payload else old_payload) == ALLOW else 'DENY'
        rc, booted = boot(esp, scratch, f'recover-{scenario}-{tag}', [
            (b'arena>', 1, b'perm show\r'),
            (b'arena>', 2, b'perm acquire\r'),
            (b'arena>', 3, b'shutdown\r'),
        ])
        allowed = (b'permission: ACQUIRE received 128-bit bearer' if expected == 'ALLOW'
                   else b'permission: ACQUIRE denied (no ALLOW)')
        ok &= check(rc == 0 and ('validated durable policy generation ' +
                                  ('2 ' + expected if got == new_payload else '1 ' + expected)) in booted
                    and f'permission: request arena.txt READ; durable decision {expected}' in booted
                    and allowed.decode() in booted
                    and not afs1.audit(scratch)
                    and dict(disk_records(scratch)) == records,
                    f'{scenario}/{tag}: same-platter recovery uses only visible committed decision; no invented retry')
        if not ok: break
    return ok

def main():
    esp = mtest.build(LABEL)
    ok = True
    for scenario, old_cmd, new_cmd, old_payload, new_payload in (
        ('revoke', b'allow', b'revoke', ALLOW, DENY),
        ('deny', b'allow', b'deny', ALLOW, DENY),
        ('allow', b'deny', b'allow', DENY, ALLOW),
    ):
        ok &= run_scenario(esp, scenario, old_cmd, new_cmd, old_payload, new_payload)
        if not ok: break
    print(f'[{LABEL}] ALLOW/DENY/REVOKE POLICY CRASH MODEL: {"PASS" if ok else "FAIL"}')
    return 0 if ok else 1

if __name__ == '__main__':
    sys.exit(main())
