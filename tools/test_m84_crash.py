#!/usr/bin/env python3
"""ADR-0053 SIGKILL at real fsd CREATE/2xWRITE/CLOSE/verify/ACK boundaries.

No anti-rollback claim: AFS1 coherent metadata/atomic 512-B commit and
ordered device completion only. Partial visible highest is CORRUPT, never
silently downgraded to stage1. No private key enters the guest platter.
"""
import hashlib
import shutil
import sys
from pathlib import Path
sys.path.insert(0,str(Path(__file__).resolve().parent))
import afs1, arena_env, mtest
import package_record as record
from test_package_record import RFC_SEED,openssl_sign
import test_m84_stage as t

LABEL='m84-crash'
def check(cond,why):
    print(f'[{LABEL}] {"PASS" if cond else "FAIL"}: {why}',flush=True)
    return cond

def boot(esp,disk,tag,commands,kill=None):
    feed=[((b'arena>',b'packaged READY') if n==1 else b'arena>', n,cmd+b'\r')
          for n,cmd in enumerate(commands,1)]
    rc,s,secs=mtest.boot(f'{LABEL}-{tag}',esp,feed,disk,kill=kill)
    (arena_env.build_dir()/f'serial-{LABEL}-{tag}.log').write_text(s)
    print(f'[{LABEL}] {tag}: rc={rc} {secs:.1f}s',flush=True)
    return rc,s

def recovery(esp,disk,tag):
    rc,s,secs=mtest.boot(f'{LABEL}-recover-{tag}',esp,
        [(b'arena>',1,b'pkg query app.test\r'),(b'arena>',2,b'shutdown\r')],disk)
    (arena_env.build_dir()/f'serial-{LABEL}-recover-{tag}.log').write_text(s)
    return rc,s

def main():
    esp=mtest.build(LABEL)
    disk=arena_env.make_scratch_disk()
    rc,s=boot(esp,disk,'baseline',[b'shutdown'])
    if not check(rc==0 and 'packaged READY' in s,'fresh fixture READY'):return 1
    t.host_seed(disk,{t.INPUT:t.ROOT_FILE,t.INTENT:t.POLICY})
    rc,s=boot(esp,disk,'stage-one',[b'pkg stage app.test',b'pkg policy app.test',b'shutdown'])
    if not check(rc==0 and t.contents(disk).get(t.STAGE1)==t.ROOT_FILE
                 and t.contents(disk).get(t.POLICY1)==t.POLICY
                 and not afs1.audit(disk),'guest committed baseline stage1 + root policy'):return 1
    original=arena_env.build_dir()/'m84-crash-policy-baseline.img'
    shutil.copyfile(disk,original)
    unsigned=record.signed_package(t.ID,8,bytes((n*17+3)&255 for n in range(4096)),record.ROOT)
    new=unsigned+openssl_sign(RFC_SEED,record.PKG_DOMAIN+unsigned)
    assert len(new)==4288
    t.host_seed(disk,{t.INPUT:new})
    assert t.contents(disk)[t.INPUT]==new and not afs1.audit(disk)
    base=arena_env.build_dir()/'m84-crash-baseline.img'
    shutil.copyfile(disk,base)
    # Marker observed after the shell sent the LAST request; a SIGKILL
    # after emission can race fsd's commit. The legal disk states are
    # enumerated by their actually visible bytes, not guessed ACKs.
    markers=[('precreate',b'packaged: STAGE CREATE submitted',1,(None,b'')),
             ('empty',b'packaged: STAGE CREATE committed empty',1,(b'',)),
             ('first-submit',b'packaged: STAGE WRITE submitted',1,(b'',new[:3584])),
             ('first-reply',b'packaged: STAGE WRITE reply exact',1,(new[:3584],)),
             ('second-submit',b'packaged: STAGE WRITE submitted',2,(new[:3584],new)),
             ('second-reply',b'packaged: STAGE WRITE reply exact',2,(new,)),
             ('close',b'packaged: STAGE CLOSE completed',1,(new,)),
             ('rescan',b'packaged: STAGE exact persisted bytes verified before acknowledgement',1,(new,)),
             ('ack',b'pkg: ELIGIBLE (staged, NOT installed/active) version 8',1,(new,))]
    ok=True
    for tag,marker,nth,legal in markers:
        platter=arena_env.build_dir()/f'm84-crash-{tag}.img'
        shutil.copyfile(base,platter)
        rc,killed=boot(esp,platter,'kill-'+tag,[b'pkg stage app.test'],(marker,nth,0.0))
        rec=t.contents(platter)
        newest=rec.get(t.STAGE2)
        ok &= check(rc is None and marker.decode() in killed and newest in legal
                    and rec.get(t.STAGE1)==t.ROOT_FILE and rec.get(t.POLICY1)==t.POLICY
                    and not afs1.audit(platter),
                    f'{tag}: observed guest boundary; disk within legal AFS1 prefix, predecessor exact')
        if not ok:break
        r,after=recovery(esp,platter,tag)
        if newest is None:
            expected=('packaged READY' in after and
                      'ELIGIBLE (staged, NOT installed/active) version 7' in after)
        elif newest==new:
            expected=('packaged READY' in after and
                      'ELIGIBLE (staged, NOT installed/active) version 8 digest '+
                       hashlib.sha256(new).hexdigest() in after)
        else:
            expected=('packaged READY' not in after and
                      'boot namespace scan refused' in after and
                      'pkg: transport refused' in after)
        ok &= check(r==0 and expected and t.contents(platter).get(t.STAGE2)==newest
                    and not afs1.audit(platter),
                    f'{tag}: same-platter reboot accepts only complete newest; partial blocks READY')
        if not ok:break
    if ok:
        # Also kill the independently root-signed POLICY transaction at
        # its own immutable CREATE/WRITE/CLOSE/reverify/ACK boundaries.
        header=record.signed_policy(t.ID,2,t.SUB_PUB,7,1,
                                    (hashlib.sha256(t.ROOT_FILE).digest(),))
        policy=header+openssl_sign(RFC_SEED,record.POL_DOMAIN+header)
        t.host_seed(original,{t.INTENT:policy})
        policy_markers=[('precreate',b'packaged: POLICY CREATE submitted',(None,b'')),
            ('empty',b'packaged: POLICY CREATE committed empty',(b'',)),
            ('submit',b'packaged: POLICY WRITE submitted',(b'',policy)),
            ('reply',b'packaged: POLICY WRITE reply exact',(policy,)),
            ('close',b'packaged: POLICY CLOSE completed',(policy,)),
            ('rescan',b'packaged: POLICY exact persisted bytes verified before acknowledgement',(policy,)),
            ('ack',b'pkg: POLICY committed generation 2',(policy,))]
        name=('p8-'+t.PREFIX+'-02').encode()
        for tag,marker,legal in policy_markers:
            platter=arena_env.build_dir()/f'm84-crash-policy-{tag}.img'
            shutil.copyfile(original,platter)
            rc,killed=boot(esp,platter,'policy-kill-'+tag,[b'pkg policy app.test'],(marker,1,0.0))
            rec=t.contents(platter)
            new_policy=rec.get(name)
            ok &= check(rc is None and marker.decode() in killed and new_policy in legal
                        and rec.get(t.STAGE1)==t.ROOT_FILE and rec.get(t.POLICY1)==t.POLICY
                        and not afs1.audit(platter),
                        f'policy/{tag}: observed root-signed write boundary; exact AFS1 prefix')
            if not ok:break
            r,after=recovery(esp,platter,'policy-'+tag)
            if new_policy is None:
                expected=('packaged READY' in after and
                          'ELIGIBLE (staged, NOT installed/active) version 7' in after)
            elif new_policy==policy:
                expected=('packaged READY' in after and 'pkg: INELIGIBLE' in after)
            else:
                expected=('packaged READY' not in after and
                          'boot namespace scan refused' in after and
                          'pkg: transport refused' in after)
            ok &= check(r==0 and expected and t.contents(platter).get(name)==new_policy
                        and not afs1.audit(platter),
                        f'policy/{tag}: same-platter rollback of uncommitted prefix only; no old ALLOW')
            if not ok:break
    print(f'[{LABEL}] PREFIX CRASH MODEL: {"PASS" if ok else "FAIL"}; no arbitrary corruption/rollback promise')
    return 0 if ok else 1

if __name__=='__main__':sys.exit(main())
