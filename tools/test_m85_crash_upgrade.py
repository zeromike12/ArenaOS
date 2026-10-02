#!/usr/bin/env python3
"""Actual AINS2 and AACT4 same-platter ordered-commit kill/reboot matrix."""
import hashlib,os,shutil,subprocess
from pathlib import Path
import afs1,arena_env,mtest,package_record as rec
import test_m84_stage as t
from test_package_record import RFC_SEED,openssl_sign
ROOT=Path(__file__).resolve().parent.parent
LABEL='m85-crash-upgrade'
H=lambda b:hashlib.sha256(b).digest()
AINS=('n8-'+t.PREFIX+'-02').encode()
AACT=('v8-'+t.PREFIX+'-04').encode()
def signed(v,b):
    p=rec.signed_package(t.ID,v,b,rec.ROOT)
    return p+openssl_sign(RFC_SEED,rec.PKG_DOMAIN+p)
def boot(esp,disk,tag,feed,kill=None):
    rc,s,secs=mtest.boot(f'{LABEL}-{tag}',esp,feed,disk,kill=kill,timeout_s=150)
    (arena_env.build_dir()/f'serial-{LABEL}-{tag}.log').write_text(s)
    print(f'[{LABEL}] {tag}: rc={rc} seconds={secs:.1f}',flush=True)
    return rc,s
def main():
    env=os.environ.copy();rust=Path('/opt/rust/prefix/bin')
    if rust.is_dir():env['PATH']=str(rust)+os.pathsep+env['PATH']
    subprocess.run(['python3',str(ROOT/'tools/test_phase85_elf_fit.py')],check=True,
        stdout=(arena_env.build_dir()/f'{LABEL}-fit.log').open('w'))
    fix=ROOT/'tools/phase85-elf-probe-v2'
    subprocess.run(['cargo','build','--offline','--locked','--release','--target','x86_64-unknown-none'],cwd=fix,env=env,check=True)
    v1=(ROOT/'tools/phase85-elf-probe/target/x86_64-unknown-none/release/arena-phase85-elf-probe').read_bytes()
    v2=(fix/'target/x86_64-unknown-none/release/arena-phase85-elf-probe-v2').read_bytes()
    s1,s2=signed(7,v1),signed(8,v2)
    esp=mtest.build(LABEL);disk=arena_env.make_scratch_disk()
    rc,s=boot(esp,disk,'baseline',[(b'arena>',1,b'shutdown\r')])
    assert rc==0 and list(t.contents(disk))==[b'arena.txt']
    t.host_seed(disk,{t.STAGE1:s1,t.POLICY1:t.POLICY,t.INPUT:s2,t.INTENT:t.POLICY})
    rc,s=boot(esp,disk,'selected',[
        ((b'arena>',b'packaged READY'),1,b'pkg selectlite\r'),
        ((b'arena>',b'servicemgr: Phase 8.5 first signed ELF ran, v8-03 active'),1,b'pkg stage app.test\r'),
        ((b'arena>',b'pkg: ELIGIBLE (staged, NOT installed/active) version 8'),1,b'shutdown\r')])
    assert rc==0 and t.contents(disk)[t.STAGE2]==s2
    staged=arena_env.build_dir()/f'{LABEL}-staged.img';shutil.copyfile(disk,staged)
    installed=arena_env.build_dir()/f'{LABEL}-installed.img';shutil.copyfile(staged,installed)
    rc,s=boot(esp,installed,'installed',[((b'arena>',b'packaged READY'),1,b'pkg installtwo\r'),
        ((b'arena>',b'servicemgr: maximal signed AINS2 committed'),1,b'shutdown\r')])
    assert rc==0 and AINS in t.contents(installed)
    complete_ins=t.contents(installed)[AINS]
    phases=[('precreate',b'CREATE submitted',(None,)),
        ('empty',b'CREATE committed empty',(b'',)),
        ('submit',b'WRITE submitted',(b'',)),
        ('reply',b'WRITE reply exact',()),
        ('close',b'CLOSE completed',())]
    for kind,base,name,command in (
        ('AINS',staged,AINS,b'pkg installtwo\r'),
        ('AACT',installed,AACT,b'pkg upgradetest\r'),
    ):
        old=t.contents(base)
        for tag,suffix,legal in phases:
            platter=arena_env.build_dir()/f'{LABEL}-{kind.lower()}-{tag}.img';shutil.copyfile(base,platter)
            marker=b'packaged: '+(b'INSTALL' if kind=='AINS' else b'AACT')+b' '+suffix
            rc,serial=boot(esp,platter,f'kill-{kind.lower()}-{tag}',
                [((b'arena>',b'packaged READY'),1,command)],kill=(marker,1,0))
            now=t.contents(platter);newest=now.get(name)
            assert rc is None and marker.decode() in serial and not afs1.audit(platter)
            assert all(now.get(n)==blob for n,blob in old.items())
            if kind=='AINS':
                complete=complete_ins
            else:
                # The SHA-256 and exact signed v8/installed linkage are
                # independently checked, not guessed from a printed receipt.
                complete=newest if newest and len(newest)==512 and newest[:4]==b'AACT' \
                    and newest[8:16]==(4).to_bytes(8,'little') and newest[80:112]==H(s2) \
                    and newest[48:80]==H(complete_ins) and newest[480:]==H(newest[:480]) else None
            assert newest in (None,b'',complete),(kind,tag,newest)
            if legal:assert newest in legal or newest==complete,(kind,tag)
            assert set(now)-set(old)-{name}==set(),(kind,tag)
            expect_ready=newest in (None,complete)
            feed=[((b'arena>',b'packaged READY' if expect_ready else b'packaged: boot namespace scan refused'),1,b'shutdown\r')]
            r,reboot=boot(esp,platter,f'recover-{kind.lower()}-{tag}',feed)
            assert r==0 and '[arena ERROR halt]' not in reboot and 'm7: RESULT PASS (2/2)' in reboot
            assert ('servicemgr: packaged READY' in reboot)==expect_ready
            assert t.contents(platter).get(name)==newest and not afs1.audit(platter)
            print(f'[{LABEL}] {kind}{"2" if kind=="AINS" else "4"}/{tag}: legal prefix={"absent" if newest is None else len(newest)}; same-platter recovery exact PASS',flush=True)
    print(f'[{LABEL}] real AINS2/AACT4 CREATE->WRITE->CLOSE crash-prefix matrix PASS (AFS1 model only)',flush=True)
if __name__=='__main__':main()
