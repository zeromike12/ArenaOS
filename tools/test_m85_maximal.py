#!/usr/bin/env python3
"""Guest-real signed maximal historical AFS1 platter and pre-CREATE fourth refusal."""
import hashlib
import os
import subprocess
from pathlib import Path
import afs1, arena_env, mtest, config_record, permission_record
import package_record as rec
import test_m84_stage as t
from test_package_record import RFC_SEED,openssl_sign

ROOT=Path(__file__).resolve().parent.parent
LABEL='m85-maximal'
H=lambda b:hashlib.sha256(b).digest()
def sign(v,elf):
    unsigned=rec.signed_package(t.ID,v,elf,rec.ROOT)
    return unsigned+openssl_sign(RFC_SEED,rec.PKG_DOMAIN+unsigned)
def policy(n):
    if n==1:return t.POLICY
    unsigned=rec.signed_policy(t.ID,n,t.SUB_PUB,7,1,())
    return unsigned+openssl_sign(RFC_SEED,rec.POL_DOMAIN+unsigned)
def boot(esp,disk,tag,feed):
    rc,s,sec=mtest.boot(f'{LABEL}-{tag}',esp,feed,disk,timeout_s=150)
    (arena_env.build_dir()/f'serial-{LABEL}-{tag}.log').write_text(s)
    print(f'[{LABEL}] {tag}: rc={rc} seconds={sec:.1f}',flush=True)
    assert rc==0 and 'm7: RESULT PASS (2/2)' in s and '[arena ERROR halt]' not in s and 'PANIC' not in s
    return s
def main():
    env=os.environ.copy();rust=Path('/opt/rust/prefix/bin')
    if rust.is_dir():env['PATH']=str(rust)+os.pathsep+env['PATH']
    payload=[]
    for name in ('phase85-elf-probe-hold','phase85-elf-probe-v2'):
        crate=ROOT/'tools'/name
        subprocess.run(['cargo','build','--offline','--locked','--release','--target','x86_64-unknown-none'],cwd=crate,env=env,check=True)
        payload.append((crate/'target/x86_64-unknown-none/release'/('arena-'+name)).read_bytes())
    s1,s2=sign(7,payload[0]),sign(8,payload[1])
    esp=mtest.build(LABEL);disk=arena_env.make_scratch_disk()
    boot(esp,disk,'baseline',[(b'arena>',1,b'shutdown\r')])
    historic={**{config_record.name(i).encode():config_record.pack(i,b'guest-v1' if i%2 else b'guest-v2') for i in range(1,9)},
              **{permission_record.name(i).encode():permission_record.pack(i,b'\x01\x01\x01\x00' if i%2 else b'\x01\x01\x00\x00') for i in range(1,9)},
              b'cfg-intent-one':b'x',
              b'approval-intent':b'previous-permission-intent'}
    assert len(historic)==18
    staging={t.INPUT:s2,t.INTENT:policy(4),t.STAGE1:s1,
             **{('p8-'+t.PREFIX+f'-{i:02}').encode():policy(i) for i in range(1,5)}}
    t.host_seed(disk,{**historic,**staging})
    assert len(t.contents(disk))==26
    before={k:v for k,v in t.contents(disk).items() if k not in (t.INPUT,t.INTENT)}
    def snapshot():
        prior.append(disk.read_bytes())
        return b'pkg fourthtest\r'
    prior=[]
    s=boot(esp,disk,'maximal',[
        ((b'arena>',b'packaged READY'),1,b'pkg maximalselect\r'),
        ((b'arena>',b'servicemgr: Phase 8.5 first signed ELF ran, v8-03 active'),1,b'pkg stage app.test\r'),
        ((b'arena>',b'pkg: ELIGIBLE (staged, NOT installed/active) version 8'),1,b'pkg installtwo\r'),
        ((b'arena>',b'servicemgr: maximal signed AINS2 committed without Image cap'),1,snapshot),
        ((b'arena>',b'servicemgr: maximal platter fourth refusal preserved usable signed v7 LAUNCH'),1,b'shutdown\r')])
    now=t.contents(disk)
    assert len(now)==32 and len(prior)==1 and disk.read_bytes()==prior[0]
    assert all(now[k]==v for k,v in before.items())
    assert now[t.STAGE1]==s1 and now[t.STAGE2]==s2
    assert all(('n8-'+t.PREFIX+f'-{i:02}').encode() in now for i in (1,2))
    acts=[('v8-'+t.PREFIX+f'-{i:02}').encode() for i in range(1,5)]
    assert all(k in now for k in acts[:3]) and acts[3] not in now
    assert 'configread: READ CORRUPT' not in s and 'permissiond: visible policy record CORRUPT' not in s
    assert 'servicemgr: maximal AACT4 typed NO_SPACE before CREATE' in s
    assert s.count('packaged: AACT CREATE submitted')==3
    assert 'servicemgr: maximal platter fourth refusal or prior LAUNCH FAILED' not in s
    assert 'phase85-hold: old signed v7 child ALIVE until manager Process-cap STOP' in s
    assert not afs1.audit(disk)
    print(f'[{LABEL}] historical 19 + staging 8 + real guest AINS2/AACT3=32/32; fourth typed NO_SPACE before CREATE, full platter byte-exact and prior signed v7 relaunched PASS',flush=True)
if __name__=='__main__':main()
