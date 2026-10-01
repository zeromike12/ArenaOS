#!/usr/bin/env python3
"""Guest fatal negative: manager exits with LIVE provisional ID before AACT.

QEMU rc=0 on UEFI fatal ResetSystem is NOT success. This test only passes
when the exact kernel fail-stop ERROR occurs before normal teardown; the
signed/staged/installed platter must have no activation decision.
"""
import shutil
import subprocess
import sys
from pathlib import Path
import arena_env, mtest, package_record as record, afs1
import test_m84_stage as t
from test_package_record import RFC_SEED, openssl_sign

ROOT=Path(__file__).resolve().parent.parent
LABEL='m85-manager-death'

def boot(esp,disk,tag,feed):
    rc,s,elapsed=mtest.boot(f'{LABEL}-{tag}',esp,feed,disk)
    (arena_env.build_dir()/f'serial-{LABEL}-{tag}.log').write_text(s)
    print(f'[{LABEL}] {tag}: rc={rc} seconds={elapsed:.1f}',flush=True)
    return rc,s

def main():
    subprocess.run([sys.executable,str(ROOT/'tools/test_phase85_elf_fit.py')],check=True,
        stdout=(arena_env.build_dir()/'m85-manager-death-elf-fit.log').open('w'))
    elf=(ROOT/'tools/phase85-elf-probe/target/x86_64-unknown-none/release/arena-phase85-elf-probe').read_bytes()
    assert len(elf)==648
    unsigned=record.signed_package(t.ID,7,elf,record.ROOT)
    signed=unsigned+openssl_sign(RFC_SEED,record.PKG_DOMAIN+unsigned)
    esp=mtest.build(LABEL)
    disk=arena_env.make_scratch_disk()
    rc,s=boot(esp,disk,'baseline',[(b'arena>',1,b'shutdown\r')])
    assert rc==0 and 'm7: RESULT PASS (2/2)' in s and list(t.contents(disk))==[b'arena.txt']
    t.host_seed(disk,{t.INPUT:signed,t.INTENT:t.POLICY})
    fault=arena_env.build_dir()/f'{LABEL}-fault.img';shutil.copyfile(disk,fault)
    expected='[arena ERROR halt] halting machine: ADR-0055: manager death with dynamic authority/child'
    for tag,platter,command,marker in (
        ('fatal-exit',disk,b'pkg deathtest\r','deliberate manager last-thread exit with PREPARED LIVE ID'),
        ('fatal-fault',fault,b'pkg deathfault\r','deliberate manager ring-3 #UD fault with PREPARED LIVE ID'),
    ):
        rc,s=boot(esp,platter,tag,[((b'arena>',b'packaged READY'),1,b'pkg stage app.test\r'),
            (b'arena>',2,b'pkg policy app.test\r'),(b'arena>',3,b'pkg installtest\r'),
            ((b'arena>',b'servicemgr: Phase 8.5 signed app.test INSTALL committed'),1,command)])
        assert rc==97 and s.count(expected)==1 and 'm7: RESULT PASS (2/2)' in s,tag
        assert marker in s and 'shutting down...' not in s
        assert 'servicemgr: FATAL death fixture exit returned' not in s
        items=t.contents(platter)
        assert ('n8-'+t.PREFIX+'-01').encode() in items and not any(n.startswith(b'v8-') for n in items)
        assert items[t.STAGE1]==signed and not afs1.audit(platter)
    print(f'[{LABEL}] expected fatal exit and genuine ring-3 #UD manager-death halts before bookkeeping/IPC sweep; no durable selection PASS',flush=True)

if __name__=='__main__':main()
