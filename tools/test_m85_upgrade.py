#!/usr/bin/env python3
"""Two distinct root-test-signed ELF payloads: durable v7→v8 cutover.

Both execute in real ring 3 on separate boots; no LIVE overlap or running-
child revocation claim. Phase 8.5 remains unqualified.
"""
import hashlib
import os
import subprocess
import sys
from pathlib import Path
import arena_env, mtest, afs1, package_record as record
import test_m84_stage as t
from test_package_record import RFC_SEED, openssl_sign

ROOT=Path(__file__).resolve().parent.parent
FIX=ROOT/'tools/phase85-elf-probe-v2'
LABEL='m85-upgrade'
H=lambda x: hashlib.sha256(x).digest()

def boot(esp,disk,tag,feed):
    rc,s,sec=mtest.boot(f'{LABEL}-{tag}',esp,feed,disk)
    (arena_env.build_dir()/f'serial-{LABEL}-{tag}.log').write_text(s)
    print(f'[{LABEL}] {tag}: rc={rc} seconds={sec:.1f}',flush=True)
    assert rc==0 and 'm7: RESULT PASS (2/2)' in s and '[arena ERROR halt]' not in s
    return s

def sign(version,elf):
    unsigned=record.signed_package(t.ID,version,elf,record.ROOT)
    signed=unsigned+openssl_sign(RFC_SEED,record.PKG_DOMAIN+unsigned)
    assert record.parse_package(signed).payload==elf and len(signed)<=4288
    return signed

def main():
    env=os.environ.copy()
    rust=Path('/opt/rust/prefix/bin')
    if rust.is_dir():env['PATH']=str(rust)+os.pathsep+env['PATH']
    subprocess.run([sys.executable,str(ROOT/'tools/test_phase85_elf_fit.py')],check=True,
        stdout=(arena_env.build_dir()/'m85-upgrade-v1-fit.log').open('w'))
    subprocess.run(['cargo','build','--offline','--locked','--release','--target','x86_64-unknown-none'],
        cwd=FIX,env=env,check=True,stdout=(arena_env.build_dir()/'m85-upgrade-v2-build.log').open('w'))
    elf1=(ROOT/'tools/phase85-elf-probe/target/x86_64-unknown-none/release/arena-phase85-elf-probe').read_bytes()
    elf2=(FIX/'target/x86_64-unknown-none/release/arena-phase85-elf-probe-v2').read_bytes()
    assert len(elf1)==648 and 1<=len(elf2)<=4096 and elf1!=elf2
    validator=arena_env.build_dir()/'m85-upgrade-production-elf-validator'
    subprocess.run(['rustc','--edition=2024',str(ROOT/'tools/phase85-elf-probe/validate.rs'),'-o',str(validator)],
        cwd=ROOT,env=env,check=True)
    subprocess.run([str(validator),str(FIX/'target/x86_64-unknown-none/release/arena-phase85-elf-probe-v2')],
        cwd=ROOT,check=True,stdout=(arena_env.build_dir()/'m85-upgrade-v2-validation.log').open('w'))
    s1,s2=sign(7,elf1),sign(8,elf2)
    esp=mtest.build(LABEL)
    disk=arena_env.make_scratch_disk()
    boot(esp,disk,'baseline',[(b'arena>',1,b'shutdown\r')])
    assert list(t.contents(disk))==[b'arena.txt']
    t.host_seed(disk,{t.INPUT:s1,t.INTENT:t.POLICY})
    s=boot(esp,disk,'first',[((b'arena>',b'packaged READY'),1,b'pkg stage app.test\r'),
        (b'arena>',2,b'pkg policy app.test\r'),(b'arena>',3,b'pkg installtest\r'),
        ((b'arena>',b'servicemgr: Phase 8.5 signed app.test INSTALL committed'),1,b'pkg selectlite\r'),
        (b'servicemgr: Phase 8.5 first signed ELF ran, v8-03 active',1,b'shutdown\r')])
    assert 'phase85: bounded image queried signed stage via inherited endpoint' in s
    old=t.contents(disk); assert t.STAGE1 in old and old[t.STAGE1]==s1
    assert all(('v8-'+t.PREFIX+f'-{n:02d}').encode() in old for n in (1,2,3))
    t.host_seed(disk,{t.INPUT:s2}) # offline replacement of unsigned candidate only
    s=boot(esp,disk,'second',[((b'arena>',b'packaged READY'),1,b'pkg stage app.test\r'),
        (b'arena>',2,b'pkg upgradetest\r'),
        (b'servicemgr: Phase 8.5 second distinct signed ELF version 8 installed, selected and ran in ring 3',1,b'shutdown\r')])
    assert 'phase85-v2: version-eight image queried signed stage via inherited endpoint' in s
    assert 'servicemgr: UPGRADETEST refused' not in s
    now=t.contents(disk)
    n2=('n8-'+t.PREFIX+'-02').encode();v4=('v8-'+t.PREFIX+'-04').encode()
    assert now[t.STAGE1]==s1 and now[t.STAGE2]==s2 and now[t.POLICY1]==t.POLICY
    assert all(now[n]==b for n,b in old.items() if n!=t.INPUT)
    assert n2 in now and v4 in now and not afs1.audit(disk)
    ins=now[n2]; act=now[v4]
    assert len(ins)==512 and ins[:4]==b'AINS' and ins[8:16]==(2).to_bytes(8,'little')
    assert ins[16:48]==t.ID and ins[48:80]==H(s2) and ins[80:88]==(8).to_bytes(8,'little')
    assert ins[88:120]==H(elf2) and ins[120]==2 and ins[128:160]==H(old[('n8-'+t.PREFIX+'-01').encode()])
    assert ins[480:]==H(ins[:480])
    assert len(act)==512 and act[:4]==b'AACT' and act[8:16]==(4).to_bytes(8,'little')
    assert act[16:48]==t.ID and act[48:80]==H(ins) and act[80:112]==H(s2)
    assert act[112:120]==(8).to_bytes(8,'little') and act[120]==1
    assert act[128:160]==H(old[('v8-'+t.PREFIX+'-03').encode()]) and act[480:]==H(act[:480])
    before=disk.read_bytes()
    s=boot(esp,disk,'reboot',[((b'arena>',b'packaged READY'),1,b'shutdown\r')])
    assert 'phase85-v2:' not in s and disk.read_bytes()==before
    print(f'[{LABEL}] two distinct ELF payloads {H(elf1).hex()} / {H(elf2).hex()}, v7/v8 signed and actually executed; durable fourth select hash={H(act).hex()} PASS (no live overlap)',flush=True)

if __name__=='__main__':main()
