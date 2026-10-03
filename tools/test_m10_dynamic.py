#!/usr/bin/env python3
"""Actual signed dynamic Image receives own graphics, bounded four-child model."""
import hashlib
import re
import subprocess
import arena_env
import mtest
import package_record as rec
import test_m84_stage as stage
from test_package_record import RFC_SEED,openssl_sign
from test_m10_apps import Desktop
from test_m10_desktop import crop

ROOT=arena_env.REPO_ROOT;BUILD=arena_env.build_dir();LABEL='m10-dynamic'
def main():
    crate=ROOT/'tools/phase10-graphical-probe'
    subprocess.run(['cargo','build','--offline','--locked','--release'],cwd=crate,env=arena_env.rust_env(),check=True)
    elf=(crate/'target/x86_64-unknown-none/release/arena-phase10-graphical-probe').read_bytes()
    assert 1<=len(elf)<=4096
    validator=BUILD/'m10-dynamic-validator'
    subprocess.run(['rustc','--edition=2024',str(ROOT/'tools/phase85-elf-probe/validate.rs'),'-o',str(validator)],env=arena_env.rust_env(),check=True)
    subprocess.run([str(validator),str(crate/'target/x86_64-unknown-none/release/arena-phase10-graphical-probe')],check=True)
    unsigned=rec.signed_package(stage.ID,7,elf,rec.ROOT)
    signed=unsigned+openssl_sign(RFC_SEED,rec.PKG_DOMAIN+unsigned)
    esp=mtest.build(LABEL,desktop=True);disk=arena_env.make_scratch_disk()
    rc,s,_=mtest.boot(LABEL+'-seed',esp,[(b'arena>',1,b'shutdown\r')],disk,pointer=True);assert rc==0
    stage.host_seed(disk,{stage.STAGE1:signed,stage.POLICY1:stage.POLICY})
    original=stage.contents(disk)
    pre_refusal=[]
    def samples(d):return [tuple(map(int,m)) for m in re.findall(r'measured frames/records/processes/regions/pages/maps/caps=(\d+)/(\d+)/(\d+)/(\d+)/(\d+)/(\d+)/(\d+)',d.serial())]
    def visible():
        d=Desktop(LABEL)
        try:
            i=d.serial().count('servicemgr: real signed Image delegated for broker-owned graphical spawn')-1
            x=70+i*26;y=60+i*24
            d.shot(f'signed-{i}',lambda p:len(set(crop(p,x+5,y+32,60,24)[j:j+3] for j in range(0,60*24*3,3)))==2)
            if i==3:
                d.wait(lambda:samples(d)[-1][5]==10,'four signed children not fully mapped')
                pre_refusal.append((samples(d)[-1],len(samples(d))))
            return b'pkg graphics\r'
        finally:d.dispose()
    def exercised():
        d=Desktop(LABEL)
        try:
            full=d.shot('four-live')
            d.wait(lambda:len(samples(d))>pre_refusal[0][1],'refusal snapshot missing')
            assert samples(d)[-1]==pre_refusal[0][0],('fifth refusal mutated resources',pre_refusal[0][0],samples(d)[-1])
            # A fifth dynamic app was refused even though two desktop slots
            # remain. Its temporary backing must have rolled back completely.
            assert d.serial().count('[desktop] real application spawned;')==4
            d.click(155,177);d.q.key('a')
            def raster_changed(p,x,y,old):
                current=crop(p,x,y,45,20)
                return sum(current[i:i+3]!=old[i:i+3] for i in range(0,len(current),3))>45*20*.95
            changed=d.shot('key-owned',lambda p:raster_changed(p,153,164,crop(full,153,164,45,20)))
            # Drag exact raster of the top signed child.
            d.point(154,142,True);d.point(354,242);d.point(354,242,False);d.point(780,500)
            d.shot('signed-dragged',lambda p:crop(p,353,264,45,20)==crop(changed,153,164,45,20))
            return b'pkg graphicsrevoke\r'
        finally:d.dispose()
    def revoked():
        d=Desktop(LABEL)
        try:
            before=d.shot('revoked-still-live')
            d.q.key('b')
            d.shot('revoked-child-input',lambda p:sum(a!=b for a,b in zip(crop(p,353,264,45,20),crop(before,353,264,45,20)))>45*20*3*.95)
            d.click(414,142+100) # top child now at x348,y232; close x414,y242.
            d.wait(lambda:d.serial().count('[desktop] application retired:')>=1,'signed child close not reaped')
            for i in (2,1,0):
                d.click(136+i*26,70+i*24)
                d.wait(lambda:d.serial().count('[desktop] application retired:')>=4-i,'remaining signed child not reaped')
            d.shot('signed-all-closed')
            return b'shutdown\r'
        finally:d.dispose()
    feed=[((b'arena>',b'[desktop] real desktop frame presented',b'packaged READY'),1,b'pkg graphics\r')]
    for n in range(1,5):feed.append((b'servicemgr: real signed Image delegated for broker-owned graphical spawn',n,visible))
    feed.extend([(b'servicemgr: graphical Image launch bounded refusal',1,exercised),(b'servicemgr: revoked graphical Image cannot spawn again PASS',1,revoked)])
    rc,s,_=mtest.boot(LABEL,esp,feed,disk,pointer=True,timeout_s=120)
    assert rc==0 and s.count('[desktop] real application spawned;')==4 and s.count('[desktop] application retired:')==4
    assert 'GRAPHICALTEST refused' not in s
    assert stage.contents(disk)[stage.STAGE1]==signed and all(stage.contents(disk)[k]==v for k,v in original.items())
    assert not __import__('afs1').audit(disk)
    print(f'[m10-dynamic] signed ELF {len(elf)} bytes sha256={hashlib.sha256(elf).hexdigest()}; four broker-owned real dynamic graphical processes, fifth refusal, owned key pixels, Process close and revoke-with-live-copied-pages PASS')
if __name__=='__main__':main()
