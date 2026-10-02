#!/usr/bin/env python3
"""Guest SIGKILL at real INSTALL and first AACT transaction boundaries.

AFS1's documented ordered-commit prefix model only: no arbitrary sector
corruption, hostile rollback, or universal power-failure claim. After each
kill, reboot the exact same platter and refuse any partially visible newest
record. A full signed record may commit before its reply is delivered.
"""
import hashlib
import shutil
import subprocess
import sys
from pathlib import Path
import afs1, arena_env, mtest, package_record as record
import test_m84_stage as t
from test_package_record import RFC_SEED, openssl_sign

ROOT=Path(__file__).resolve().parent.parent
LABEL='m85-crash'
H=lambda b:hashlib.sha256(b).digest()
AINS=('n8-'+t.PREFIX+'-01').encode()
AACT=('v8-'+t.PREFIX+'-01').encode()

def boot(esp,disk,tag,feed,kill=None):
    rc,s,secs=mtest.boot(f'{LABEL}-{tag}',esp,feed,disk,kill=kill)
    (arena_env.build_dir()/f'serial-{LABEL}-{tag}.log').write_text(s)
    print(f'[{LABEL}] {tag}: rc={rc} seconds={secs:.1f}',flush=True)
    return rc,s

def main():
    subprocess.run([sys.executable,str(ROOT/'tools/test_phase85_elf_fit.py')],check=True,
        stdout=(arena_env.build_dir()/'m85-crash-elf-fit.log').open('w'))
    elf=(ROOT/'tools/phase85-elf-probe/target/x86_64-unknown-none/release/arena-phase85-elf-probe').read_bytes()
    assert len(elf)==648
    unsigned=record.signed_package(t.ID,7,elf,record.ROOT)
    signed=unsigned+openssl_sign(RFC_SEED,record.PKG_DOMAIN+unsigned)
    ins=bytearray(512); ins[:4]=b'AINS'; ins[4:8]=b'\x01\0\0\x02'; ins[8]=1
    ins[16:48]=t.ID; ins[48:80]=H(signed); ins[80]=7; ins[88:120]=H(elf)
    ins[120:122]=b'\x01\x01'; ins[480:]=H(ins[:480]); ins=bytes(ins)
    act=bytearray(512);act[:4]=b'AACT';act[4:8]=b'\x01\0\0\x02';act[8]=1
    act[16:48]=t.ID;act[48:80]=H(ins);act[80:112]=H(signed);act[112]=7;act[120]=1
    act[480:]=H(act[:480]);act=bytes(act)
    esp=mtest.build(LABEL)
    disk=arena_env.make_scratch_disk()
    rc,s=boot(esp,disk,'baseline',[(b'arena>',1,b'shutdown\r')])
    assert rc==0 and 'm7: RESULT PASS (2/2)' in s and list(t.contents(disk))==[b'arena.txt']
    fillers={f'hist{i:02d}'.encode():bytes([i+1]) for i in range(23)}
    t.host_seed(disk,{t.INPUT:signed,t.INTENT:t.POLICY,**fillers})
    rc,s=boot(esp,disk,'staged',[( (b'arena>',b'packaged READY'),1,b'pkg stage app.test\r'),
        (b'arena>',2,b'pkg policy app.test\r'),(b'arena>',3,b'shutdown\r')])
    assert rc==0 and t.contents(disk)[t.STAGE1]==signed and t.contents(disk)[t.POLICY1]==t.POLICY
    base=arena_env.build_dir()/f'{LABEL}-staged.img';shutil.copyfile(disk,base)
    # CREATE/WRITE/CLOSE markers arise in packaged, after the shell's last
    # request has been sent; killer observes subsequent serial only.
    phases=[('precreate',b'CREATE submitted',(None,b'')),
        ('empty',b'CREATE committed empty',(b'',)),
        ('submit',b'WRITE submitted',(b'',)),
        ('reply',b'WRITE reply exact',()),
        ('close',b'CLOSE completed',())]
    for which in ('INSTALL','AACT'):
        if which=='AACT':
            before=arena_env.build_dir()/f'{LABEL}-installed.img'
            shutil.copyfile(base,before)
            rc,s=boot(esp,before,'installed',[((b'arena>',b'packaged READY'),1,b'pkg installtest\r'),
                (b'servicemgr: Phase 8.5 signed app.test INSTALL committed',1,b'shutdown\r')])
            assert rc==0 and t.contents(before)[AINS]==ins
            base=before
        old=t.contents(base)
        name=AINS if which=='INSTALL' else AACT
        complete=ins if which=='INSTALL' else act
        command=b'pkg installtest\r' if which=='INSTALL' else b'pkg selecttest\r'
        for tag,suffix,legal in phases:
            platter=arena_env.build_dir()/f'{LABEL}-{which.lower()}-{tag}.img'
            shutil.copyfile(base,platter)
            marker=b'packaged: '+which.encode()+b' '+suffix
            rc,killed=boot(esp,platter,f'kill-{which.lower()}-{tag}',
                [((b'arena>',b'packaged READY'),1,command)],(marker,1,0))
            rec=t.contents(platter)
            newest=rec.get(name)
            assert rc is None and marker.decode() in killed and not afs1.audit(platter)
            assert all(rec.get(n)==b for n,b in old.items()), f'{which}/{tag}: predecessor changed'
            assert newest in (None,b'',complete),f'{which}/{tag}: unexpected visible prefix {len(newest or b"")}'
            if legal: assert newest in legal or newest==complete, f'{which}/{tag}: unexpected progress'
            # The manager may reach a later *complete* AACT decision before
            # SIGKILL; no malformed later decision can be called success.
            extra=set(rec)-set(old)-{name}
            assert not extra, f'{which}/{tag}: unexpected later decisions {extra}'
            if newest is None or newest==complete:
                feed=[((b'arena>',b'packaged READY'),1,b'shutdown\r')]
            else:
                feed=[((b'arena>',b'packaged: boot namespace scan refused'),1,b'shutdown\r')]
            r,recovered=boot(esp,platter,f'recover-{which.lower()}-{tag}',feed)
            assert r==0 and 'm7: RESULT PASS (2/2)' in recovered and '[arena ERROR halt]' not in recovered
            assert ('servicemgr: packaged READY' in recovered)==(newest in (None,complete))
            assert ('packaged: boot namespace scan refused' in recovered)==(newest==b'')
            assert t.contents(platter).get(name)==newest and not afs1.audit(platter)
            print(f'[{LABEL}] {which}/{tag}: legal prefix={"absent" if newest is None else len(newest)}; same-platter recovery exact PASS',flush=True)
    print(f'[{LABEL}] bounded INSTALL/AACT CREATE→WRITE→CLOSE crash-prefix proof PASS (not arbitrary corruption)',flush=True)

if __name__=='__main__':main()
