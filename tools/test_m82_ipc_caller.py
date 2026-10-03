#!/usr/bin/env python3
"""ADR-0050 real IPC dead-caller teardown, reply refusal and queue reuse.

Never treat QEMU's ResetSystem return 0 as proof: require semantic
markers, exact resource snapshots, and absence of kernel halt/error.
"""
import re
import sys
from pathlib import Path
sys.path.insert(0, str(Path(__file__).resolve().parent))
import arena_env  # noqa: E402
import mtest  # noqa: E402

LABEL = 'm82-ipc-caller'
SNAP = r'permission: resource snapshot free/records/processes (\d+)/(\d+)/(\d+)'
BAD = ('[arena ERROR halt]', '[arena ERROR ipc]', 'halting machine:',
       'PANIC', 'servicemgr: FATAL')

def check(cond: bool, msg: str) -> bool:
    print(f'[{LABEL}] {"PASS" if cond else "FAIL"}: {msg}')
    return cond

def run(esp: Path, mode: str, completion: bytes) -> tuple[int | None, str]:
    arena_env.make_scratch_disk()
    feed = [((b'permissiond READY', b'arena>',
               b'permission app reaped through held Process cap'), 1, b'perm snapshot\r'),
            (b'arena>', 2, b'perm ' + mode.encode() + b'\r'),
            (completion, 1, b'perm snapshot\r'),
            (b'arena>', 4, b'perm acquire\r'),
            (b'arena>', 5, b'shutdown\r')]
    try:
        rc, serial, secs = mtest.run_qemu(f'{LABEL}-{mode}', esp, feed=feed)
    except Exception as exc:
        print(f'[{LABEL}] {mode} harness exception: {exc}')
        rc, serial, secs = None, (arena_env.build_dir() / 'serial.log').read_text(errors='replace'), 0
    (arena_env.build_dir() / f'serial-{LABEL}-{mode}.log').write_text(serial)
    print(f'[{LABEL}] {mode}: rc={rc}, seconds={secs:.1f}')
    return rc, serial

def main() -> int:
    # The precise false-positive that hid the original kernel halt:
    # QEMU 0 plus ResetSystem(shutdown) is still FAIL if halt_machine ran.
    sample = ('m3: RESULT PASS (13/13)\nm4: RESULT PASS (9/9)\n'
              'halting via UEFI ResetSystem(shutdown)\n')
    ok = check(mtest.semantic_exit_rc(0, sample, LABEL) == 0
               and mtest.semantic_exit_rc(0, sample + '[arena ERROR halt] halting machine:', LABEL) == 97
               and mtest.semantic_exit_rc(0, sample.replace('m4: RESULT PASS (9/9)', ''), LABEL) == 97,
               'harness refuses QEMU rc=0 kernel halt or missing semantic PASS')
    esp = mtest.build(LABEL)
    for mode, completed in [
        ('probe-stall-caller-first', b'diagnostic broker replacement ready on original endpoint'),
        ('probe-stall', b'diagnostic broker replacement ready on original endpoint'),
        ('probe-bad', b'diagnostic broker replacement ready on original endpoint'),
    ]:
        rc, s = run(esp, mode, completed)
        ok &= check(rc == 0 and all(b not in s for b in BAD)
                    and 'm7: RESULT PASS (2/2)' in s
                    and '[arena INFO  m4] ADR-0050 four abandoned caller states cleared; staged caps discarded; late server reply typed STATUS_BAD_ARG; endpoint recycled; frames exact' in s
                    and 'halting via UEFI ResetSystem(shutdown)' in s,
                    f'{mode}: semantic historical PASS and clean user shutdown (no kernel halt/error)')
        snapshots = re.findall(SNAP, s)
        ok &= check(len(snapshots) == 2 and snapshots[0] == snapshots[1],
                    f'{mode}: exact frame/spawn/process accounting stable after worker and broker teardown')
        ok &= check('permission PING failed/deadline; no READY' in s
                    and completed.decode() in s
                    and 'permission: ACQUIRE denied (no ALLOW)' in s,
                    f'{mode}: failed probe did not approve; original endpoint still serves fresh PING/deny')
        if mode == 'probe-stall-caller-first':
            first = s.find('diagnostic caller-first worker stopped before broker')
            broker = s.find('permission diagnostic broker stopped; no false READY', first)
            ok &= check(first >= 0 and broker > first
                        and re.search(r'destroy pid \d+: killed 1 live thread\(s\).*1 abandoned caller slot\(s\)', s) is not None,
                        'worker died IN SYS_IPC_CALL before server death; kernel swept Delivered caller')
        elif mode == 'probe-stall':
            ok &= check('broker-first failed blocked PING; worker reaped/stopped' in s
                        and 'depcheck: permission PING incomplete or refused' in s,
                        'broker-first timeout returned typed SERVICE_GONE to still-live probe')
        elif mode == 'probe-bad':
            ok &= check('permissiond: diagnostic PING sent wrong typed reply' in s
                        and 'depcheck: permission PING incomplete or refused' in s,
                        'invalid typed reply never became authenticated readiness')
    print(f'[{LABEL}] ADR-0050: {"PASS" if ok else "FAIL"} (3 guest fault boundaries)')
    return 0 if ok else 1

if __name__ == '__main__':
    sys.exit(main())
