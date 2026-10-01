#!/usr/bin/env python3
"""Fail-stop before IPC/process sweep when manager owns a LIVE signed child."""
import os,shutil,subprocess
from pathlib import Path
import afs1,arena_env,mtest,package_record as rec
import test_m84_stage as t
from test_package_record import RFC_SEED,openssl_sign
ROOT=Path(__file__).resolve().parent.parent
LABEL='m85-manager-death-live'
def boot(esp,disk,tag,feed):
    rc,s,sec=mtest.boot(f'{LABEL}-{tag}',esp,feed,disk,timeout_s=120)
    (arena_env.build_dir()/f'serial-{LABEL}-{tag}.log').write_text(s)
    print(f'[{LABEL}] {tag}: rc={rc} seconds={sec:.1f}',flush=True)
    return rc,s
def signed(v,elf):
    p=rec.signed_package(t.ID,v,elf,rec.ROOT)
    return p+openssl_sign(RFC_SEED,rec.PKG_DOMAIN+p)
def main():
    env=os.environ.copy();rust=Path('/opt/rust/prefix/bin')
    if rust.is_dir():env['PATH']=str(rust)+os.pathsep+env['PATH']
    payload=[]
    for name in ('phase85-elf-probe-hold','phase85-elf-probe-v2'):
        crate=ROOT/'tools'/name
        subprocess.run(['cargo','build','--offline','--locked','--release','--target','x86_64-unknown-none'],cwd=crate,env=env,check=True)
        payload.append((crate/'target/x86_64-unknown-none/release'/('arena-'+name)).read_bytes())
    assert payload[0]!=payload[1]
    s1,s2=signed(7,payload[0]),signed(8,payload[1])
    esp=mtest.build(LABEL);disk=arena_env.make_scratch_disk()
    rc,s=boot(esp,disk,'baseline',[(b'arena>',1,b'shutdown\r')])
    assert rc==0 and list(t.contents(disk))==[b'arena.txt']
    t.host_seed(disk,{t.STAGE1:s1,t.POLICY1:t.POLICY,t.INPUT:s2,t.INTENT:t.POLICY})
    fault=arena_env.build_dir()/f'{LABEL}-fault.img';shutil.copyfile(disk,fault)
    expected='[arena ERROR halt] halting machine: ADR-0055: manager death with dynamic authority/child'
    for tag,platter,command,marker in (
        ('exit',disk,b'pkg deathtest\r','deliberate manager last-thread exit with genuinely LIVE signed v7 child/ID'),
        ('fault',fault,b'pkg deathfault\r','deliberate manager ring-3 #UD with genuinely LIVE signed v7 child/ID'),
    ):
        rc,s=boot(esp,platter,tag,[
            ((b'arena>',b'packaged READY'),1,b'pkg selectlite\r'),
            ((b'arena>',b'servicemgr: Phase 8.5 first signed ELF ran, v8-03 active'),1,b'pkg stage app.test\r'),
            ((b'arena>',b'pkg: ELIGIBLE (staged, NOT installed/active) version 8'),1,b'pkg oldlive\r'),
            ((b'arena>',b'phase85-hold: old signed v7 child ALIVE until manager Process-cap STOP'),1,command)])
        assert rc==97 and s.count(expected)==1 and marker in s,tag
        assert 'servicemgr: old signed v7 dynamic child spawned, Process cap held' in s
        assert 'FATAL live-child death fixture returned' not in s and 'shutting down...' not in s
        items=t.contents(platter)
        assert items[t.STAGE1]==s1 and items[t.STAGE2]==s2
        assert ('v8-'+t.PREFIX+'-03').encode() in items and ('v8-'+t.PREFIX+'-04').encode() not in items
        assert not afs1.audit(platter)
    print(f'[{LABEL}] genuinely LIVE signed v7 child and manager Image ID: last-thread exit and ring-3 #UD both HALT before bookkeeping/IPC sweep PASS',flush=True)
if __name__=='__main__':main()
