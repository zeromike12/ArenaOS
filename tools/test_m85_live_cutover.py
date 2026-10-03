#!/usr/bin/env python3
"""Real same-boot signed v7/v8 PREPARE; a live old child is stopped before COMMIT."""
import hashlib
import os
import subprocess
from pathlib import Path
import arena_env, mtest, afs1, package_record as rec
import test_m84_stage as t
from test_package_record import RFC_SEED, openssl_sign

ROOT=Path(__file__).resolve().parent.parent
LABEL='m85-live-cutover'
H=lambda x:hashlib.sha256(x).digest()
def sign(v,elf):
    p=rec.signed_package(t.ID,v,elf,rec.ROOT)
    return p+openssl_sign(RFC_SEED,rec.PKG_DOMAIN+p)
def boot(esp,disk,tag,feed):
    rc,s,sec=mtest.boot(f'{LABEL}-{tag}',esp,feed,disk,timeout_s=120)
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
        binary=crate/'target/x86_64-unknown-none/release'/('arena-'+name)
        payload.append(binary.read_bytes())
    v1,v2=payload
    assert v1!=v2 and all(1<=len(x)<=4096 for x in payload)
    validator=arena_env.build_dir()/'m85-live-cutover-validator'
    subprocess.run(['rustc','--edition=2024',str(ROOT/'tools/phase85-elf-probe/validate.rs'),'-o',str(validator)],cwd=ROOT,env=env,check=True)
    for i,payload in enumerate((v1,v2)):
        file=arena_env.build_dir()/f'm85-live-cutover-{i}.elf';file.write_bytes(payload)
        subprocess.run([str(validator),str(file)],check=True)
    s1,s2=sign(7,v1),sign(8,v2)
    esp=mtest.build(LABEL); disk=arena_env.make_scratch_disk()
    boot(esp,disk,'baseline',[(b'arena>',1,b'shutdown\r')])
    t.host_seed(disk,{t.STAGE1:s1,t.POLICY1:t.POLICY,t.INPUT:s2,t.INTENT:t.POLICY})
    before=t.contents(disk)
    s=boot(esp,disk,'cutover',[
        ((b'arena>',b'packaged READY'),1,b'pkg selectlite\r'),
        ((b'arena>',b'servicemgr: Phase 8.5 first signed ELF ran, v8-03 active'),1,b'pkg stage app.test\r'),
        ((b'arena>',b'pkg: ELIGIBLE (staged, NOT installed/active) version 8'),1,b'pkg resources\r'),
        (b'pkg: observed frames=',1,b'pkg oldlive\r'),
        ((b'arena>',b'phase85-hold: old signed v7 child ALIVE until manager Process-cap STOP'),1,b'pkg resources\r'),
        (b'pkg: observed frames=',2,b'pkg upgradetest\r'),
        ((b'arena>',b'servicemgr: Phase 8.5 second distinct signed ELF version 8 installed, selected and ran in ring 3'),1,b'pkg resources\r'),
        (b'pkg: observed frames=',3,b'shutdown\r')])
    assert s.count('phase85-hold: first signed v7 launch exited normally')>=1
    assert s.count('phase85-hold: old signed v7 child ALIVE until manager Process-cap STOP')>=1
    assert 'servicemgr: four repeated signed-child STOP/FINISH cycles and BUSY refusals PASS' in s
    assert 'servicemgr: genuinely LIVE v7 child stopped and reaped by held Process cap before v8 COMMIT' in s
    assert 'servicemgr: distinct signed v7/v8 registry slots live together at PREPARE; old copied ID revoked before COMMIT' in s
    assert 'phase85-v2: version-eight image queried signed stage via inherited endpoint' in s
    assert 'UPGRADETEST refused' not in s and 'SELECTTEST refused' not in s
    import re
    samples=[tuple(map(int,x)) for x in re.findall(r'pkg: observed frames=(\d+) records=(\d+) processes=(\d+)',s)]
    assert len(samples)==3 and samples[0]==samples[2] and samples[1][0]<samples[0][0]
    assert samples[1][1:]==(samples[0][1]+1,samples[0][2]+1),samples
    assert 'servicemgr: full fixture notification budget 19/19; twentieth refused' in s
    mgr={name:int(n) for name,n in re.findall(r'servicemgr: observed cap occupancy ([\w-]+)=(\d+)',s)}
    assert all(0<mgr[k]<=32 for k in ('baseline','two-live-images','unretired-child','after-finish')),mgr
    assert mgr['two-live-images']>mgr['unretired-child']>=mgr['baseline'] and mgr['after-finish']<mgr['two-live-images'],mgr
    peaks=[int(n) for n in re.findall(r'packaged: observed cap high-water (\d+)',s)]
    assert peaks and max(peaks)<=32 and max(peaks)>=7,peaks
    print(f'[{LABEL}] measured frames/records/processes={samples}; manager caps={mgr}; packaged cap peak={max(peaks)}',flush=True)
    now=t.contents(disk)
    assert all(now[k]==v for k,v in before.items()) and now[t.STAGE2]==s2
    n1=('n8-'+t.PREFIX+'-01').encode();n2=('n8-'+t.PREFIX+'-02').encode()
    acts=[('v8-'+t.PREFIX+f'-{i:02}').encode() for i in range(1,5)]
    assert all(k in now for k in (n1,n2,*acts)) and now[acts[3]][80:112]==H(s2)
    assert not afs1.audit(disk)
    print(f'[{LABEL}] old signed v7 child ALIVE at v8 PREPARE; second child BUSY; held Process-cap STOP/reap and full-ID revoke before durable AACT4; distinct signed v8 ELF ran PASS',flush=True)
if __name__=='__main__':main()
