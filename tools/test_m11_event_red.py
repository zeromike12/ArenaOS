#!/usr/bin/env python3
"""Phase 11.0 RED/GREEN controls for kernel event plumbing (ADR-0071/0072).

Each control removes ONE production mechanism from real kernel source,
builds the real image, boots it, and requires that the matching m11 boot
check (and only a real guest observation) reports FAIL. A build error can
never count as RED. Source and the shipped EFI/ESP are restored byte-exactly
in `finally`; a final GREEN boot of the restored bytes must report
`m11: RESULT PASS (8/8)` and reach the shell.
"""
import hashlib
from pathlib import Path
import arena_env
import mtest

ROOT = arena_env.REPO_ROOT
BUILD = arena_env.build_dir()
IPC = ROOT / 'kernel/kernel/src/ipc.rs'
TIMER = ROOT / 'kernel/kernel/src/timer.rs'
CAP = ROOT / 'kernel/kernel/src/cap.rs'
SCHED = ROOT / 'kernel/kernel/src/sched/mod.rs'
CONTROLS = [
    # A queued call never raises the bound notification.
    (IPC, b'            let signal = if parked == NO_TID { ep.bound } else { None };',
     b'            let signal: Option<Binding> = None;',
     'bound-signal', 'm11:test:bound_signal: FAIL'),
    # Destroying a notification leaves endpoints bound to its index.
    (IPC, b'                if ep.bound.is_some_and(|b| b.nid == nid) {',
     b'                if ep.bound.is_some_and(|b| b.nid == nid) && false {',
     'notif-destroy-unbind', 'm11:test:notif_destroy_unbinds: FAIL'),
    # Orphaning (server death) keeps the dead server's binding.
    (IPC, b'                // Its binding described that server\'s wait; a successor\n'
          b'                // binds its own notification (ADR-0071).\n'
          b'                if ep.bound.take().is_some() {',
     b'                // Its binding described that server\'s wait; a successor\n'
     b'                // binds its own notification (ADR-0071).\n'
     b'                if false && ep.bound.take().is_some() {',
     'orphan-unbind', 'm11:test:orphan_unbinds: FAIL'),
    # The per-process quota is not enforced.
    (TIMER, b'            if held >= MAX_TIMERS_PER_PROCESS {',
     b'            if held >= MAX_TIMERS_PER_PROCESS && false {',
     # m7's ring-3 timertest (earlier in boot) observes it first.
     'timer-quota', ('m11:test:timer_quota: FAIL',
                     'the fifth timer was not refused with STATUS_QUOTA (70)')),
    # ADR-0074: a badged cap ignores the endpoint generation (stale reuse).
    (CAP, b'            if crate::ipc::endpoint_generation(u32::from(eid)) != Some(generation) {',
     b'            if crate::ipc::endpoint_generation(u32::from(eid)).is_none() && generation == 0 {',
     'badge-generation', 'm11:test:badged_endpoint: FAIL'),
    # ADR-0074: anyone holding the endpoint may mint (serve-side check gone).
    (CAP, b'    if src.rights & RIGHTS_READ == 0 {\n        return Err("mint: only the serve side (READ) may mint");',
     b'    if false {\n        return Err("mint: only the serve side (READ) may mint");',
     'badge-mint-authority', 'm11:test:badged_endpoint: FAIL'),
    # A handoff wake is an ordinary back-of-ring wake.
    (SCHED, b'    wake_with(tid, true)\n', b'    wake_with(tid, false)\n',
     'handoff', 'm11:test:handoff_order: FAIL'),
    # A handoff overtakes the caller's own earlier wakes (the causal-order
    # rule found by the historical stackstop proof).
    (SCHED, b'            let front = handoff && !cpu.woke_others && chain_open(cpu);',
     b'            let front = handoff && chain_open(cpu);',
     'handoff-causal', 'm11:test:handoff_order: FAIL'),
    # Handoff chains are unbounded: a call/reply pair keeps the ring from
    # turning (the livelock that first kept REPLY a FIFO wake).
    (SCHED, b'    now.wrapping_sub(cpu.chain_since) < budget\n',
     b'    now.wrapping_sub(cpu.chain_since) < budget.max(u64::MAX)\n',
     'handoff-chain', 'm11:test:handoff_chain: FAIL'),
]


def boot(label, esp, green):
    disk = arena_env.make_scratch_disk()
    feed = [(b'arena>', 1, b'shutdown\r')] if green else []
    return mtest.boot(label, esp, feed, disk, timeout_s=150 if green else 90)


def main():
    sources = {p: p.read_bytes() for p in {c[0] for c in CONTROLS}}
    for path, before, after, label, _ in CONTROLS:
        assert sources[path].count(before) == 1, (label, sources[path].count(before))
    esp = mtest.build('m11-red-base')
    artifacts = {p: p.read_bytes() for p in (esp, BUILD / 'arena-boot.efi')}
    reds = []
    try:
        for path, before, after, label, expect in CONTROLS:
            original = sources[path]
            path.write_bytes(original.replace(before, after, 1))
            try:
                mutant = mtest.build(f'm11-{label}-mutant')
                rc, serial, _ = boot(f'm11-{label}-mutant', mutant, green=False)
                (BUILD / f'm11-{label}-red.log').write_text(serial)
                expects = expect if isinstance(expect, tuple) else (expect,)
                red = rc != 0 and any(e in serial for e in expects)
                assert red, f'{label}: real guest did not report {expect!r}'
                reds.append(label)
                print(f'[m11-red] {label}: guest {expect} → RED PASS', flush=True)
            finally:
                path.write_bytes(original)
    finally:
        for p, data in sources.items():
            p.write_bytes(data)
        try:
            mtest.build('m11-red-restored')
        finally:
            for p, data in artifacts.items():
                p.write_bytes(data)
    assert all(p.read_bytes() == d for p, d in sources.items()), 'source not restored'
    assert all(p.read_bytes() == d for p, d in artifacts.items()), 'artifacts not restored'
    rc, serial, _ = boot('m11-green', esp, green=True)
    assert rc == 0 and 'm11: RESULT PASS (8/8)' in serial, 'restored source is not GREEN'
    digest = hashlib.sha256(artifacts[BUILD / 'arena-boot.efi']).hexdigest()
    print(f'[m11-red] {len(reds)} production RED controls ({", ".join(reds)}); '
          f'byte-exact restore; GREEN m11 8/8 sha256={digest}', flush=True)


if __name__ == '__main__':
    main()
