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
def main(esp=None):
    crate=ROOT/'tools/phase10-graphical-probe'
    subprocess.run(['cargo','build','--offline','--locked','--release'],cwd=crate,env=arena_env.rust_env(),check=True)
    elf=(crate/'target/x86_64-unknown-none/release/arena-phase10-graphical-probe').read_bytes()
    assert 1<=len(elf)<=4096
    validator=BUILD/'m10-dynamic-validator'
    subprocess.run(['rustc','--edition=2024',str(ROOT/'tools/phase85-elf-probe/validate.rs'),'-o',str(validator)],env=arena_env.rust_env(),check=True)
    subprocess.run([str(validator),str(crate/'target/x86_64-unknown-none/release/arena-phase10-graphical-probe')],check=True)
    unsigned=rec.signed_package(stage.ID,7,elf,rec.ROOT)
    signed=unsigned+openssl_sign(RFC_SEED,rec.PKG_DOMAIN+unsigned)
    if esp is None:esp=mtest.build(LABEL,desktop=True)
    disk=arena_env.make_scratch_disk()
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
            def checker(p):
                pixel=lambda a,b:p[(b*800+a)*3:(b*800+a)*3+3]
                a=pixel(x+5,y+35);b=pixel(x+15,y+35)
                return a!=b and all(pixel(x+5+k*10,y+35+r*10)==(a if (k+r)%2==0 else b) for r in range(2) for k in range(6))
            d.shot(f'signed-{i}',checker)
            if i==3:
                # Base two maps + three per session (ADR-0075).
                d.wait(lambda:samples(d)[-1][5]==2+4*3,'four signed children not fully mapped')
                pre_refusal.append((samples(d)[-1],len(samples(d))))
            return b'pkg graphics\r'
        finally:d.dispose()
    def exercised():
        d=Desktop(LABEL)
        try:
            __import__('time').sleep(.16) # settle the bounded open/focus transition
            full=d.shot('four-live')
            d.wait(lambda:len(samples(d))>pre_refusal[0][1],'refusal snapshot missing')
            assert samples(d)[-1]==pre_refusal[0][0],('fifth refusal mutated resources',pre_refusal[0][0],samples(d)[-1])
            # A fifth dynamic app was refused even though two desktop slots
            # remain. Its temporary backing must have rolled back completely.
            assert d.serial().count('[desktop] real application spawned;')==4
            d.click(155,177)
            d.shot('signed-focus-settled',lambda p:crop(p,153,164,45,20)==crop(full,153,164,45,20))
            d.q.key('f')
            # Pointer redraws must use the last complete published snapshot,
            # while the client has already changed its writable staging bytes.
            # Composition is damage-driven (ADR-0070): only rectangles the
            # arrow covers or uncovers are recomposed. Each cycle therefore
            # moves the arrow onto the signed raster (recomposing those window
            # pixels from whatever the compositor treats as the window's
            # content), then away, and compares the uncovered raster. The
            # child sees 'f' on its own pacing timer, so cycles continue for
            # at least one second after the key: comparisons keep running
            # after the staging bytes have certainly changed.
            key_sent=__import__('time').monotonic();cycle=0
            while cycle<6 or __import__('time').monotonic()-key_sent<1.0:
                x=156+15*(cycle%3);y=167
                d.point(x,y)
                d.shot('unpublished-sweep',lambda p:crop(p,x,y,10,13)!=crop(full,x,y,10,13))
                # Repeated identical absolute coordinates can be coalesced by
                # QEMU. Alternate real positions and await the drawn cursor:
                # this proves a compositor redraw happened after the key.
                x=700+20*(cycle%2);y=420+20*(cycle%2)
                d.point(x,y)
                def cursor_drawn(p):
                    pixel=lambda a,b:p[(b*800+a)*3:(b*800+a)*3+3]
                    return any(pixel(a,b)==pixel(a,b+6)!=pixel(a+6,b+6)
                               for a in range(x-1,x+2) for b in range(y-1,y+2))
                staged=d.shot('unpublished-staged',cursor_drawn)
                assert crop(staged,153,164,45,20)==crop(full,153,164,45,20),'unpublished backing became visible'
                cycle+=1
            d.q.key('p')
            published=d.shot('published-staged',lambda p:sum(a!=b for a,b in zip(crop(p,153,164,45,20),crop(full,153,164,45,20)))>45*20*3*.95)
            full=published
            d.q.key('a')
            def raster_changed(p,x,y,old):
                current=crop(p,x,y,45,20)
                return sum(current[i:i+3]!=old[i:i+3] for i in range(0,len(current),3))>45*20*.95
            changed=d.shot('key-owned',lambda p:raster_changed(p,153,164,crop(full,153,164,45,20)))
            changed=d.settled('key-owned',(153,164,45,20))
            # Phase 11.1: the child repaints its whole staging raster but
            # declares one 10x10 Damage rectangle. Exactly that rectangle may
            # change on screen; every other staged byte stays unpublished.
            region=(120,140,160,110)
            def box(p):
                a,b=crop(p,*region),crop(changed,*region)
                xs=[(i//3)%region[2] for i in range(0,len(a),3) if a[i:i+3]!=b[i:i+3]]
                ys=[(i//3)//region[2] for i in range(0,len(a),3) if a[i:i+3]!=b[i:i+3]]
                return (min(xs),min(ys),max(xs)-min(xs)+1,max(ys)-min(ys)+1,len(xs)) if xs else None
            d.q.key('r')
            partial=d.shot('partial-publish',lambda p:box(p) is not None)
            partial=d.settled('partial-publish',region)
            x0,y0,bw,bh,n=box(partial)
            assert bw<=10 and bh<=10 and n>=90,('partial Damage published outside its rectangle',x0,y0,bw,bh,n)
            changed=partial
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
            # Dynamic and BootImage children share the general session
            # ownership model (twelve sessions, ADR-0075), while the native
            # dynamic quota stays four.
            d.launch(0,'mixed-terminal',4);d.launch(3,'mixed-settings',5)
            shared,snapshot=map(int,re.search(r'session reservation shared/snapshot pages=(\d+)/(\d+)',d.serial()).groups())
            base=samples(d)[0]
            # Six sessions: shared + snapshot region each, three maps each.
            d.wait(lambda:samples(d)[-1][3:6]==(base[3]+12,base[4]+6*(shared+snapshot),base[5]+18),'mixed working set not fully mapped')
            mixed=samples(d)[-1]
            assert mixed[1:3]==(base[1]+6,base[2]+6) and mixed[6]==base[6]+12,('mixed working-set counters',mixed)
            # Six more builtins fill the twelve sessions; the thirteenth
            # launch is refused with nothing spawned.
            for k in range(6):
                d.click(255+k*58,570)
                d.wait(lambda:d.serial().count('[desktop] real application spawned;')>=7+k,'mixed second-lane session not spawned')
            d.wait(lambda:samples(d)[-1][3:6]==(base[3]+24,base[4]+12*(shared+snapshot),base[5]+36),'twelve mixed sessions not fully mapped')
            d.settled('mixed-twelve',(0,26,800,500))
            full_mixed=d.shot('mixed-full')
            d.click(255+5*58,570)
            d.shot('mixed-capacity-refused',lambda p:crop(p,10,28,250,20)!=crop(full_mixed,10,28,250,20))
            assert d.serial().count('[desktop] real application spawned;')==12,'thirteenth mixed application spawned'
            for k in range(6):
                d.q.command('input-send-event',events=[d.q._ev('f8',True),d.q._ev('f8',False)])
                d.wait(lambda:d.serial().count('[desktop] application retired:')>=k+1,'F8 failed to retire second-lane session')
            d.wait(lambda:samples(d)[-1][1:]==mixed[1:],'second lane did not return to the six-session working set')
            for index in (5,4):
                d.close(index)
                d.wait(lambda:d.serial().count('[desktop] application retired:')>=12-index,'mixed builtin not reaped')
            # A small signed window must remain reachable above the dock.
            # Compare its exact owned chrome after a real bottom-edge drag.
            before_bottom=d.settled('bottom-drag-start',(353,237,60,18))
            d.point(354,242,True);d.point(330,590);d.point(330,590,False);d.point(780,500)
            d.shot('safe-title-above-dock',lambda p:crop(p,329,521,60,18)==crop(before_bottom,353,237,60,18))
            d.click(390,526) # exact child now at x324,y516; reachable close.
            d.wait(lambda:d.serial().count('[desktop] application retired:')>=9,'signed child close not reaped')
            for i in (2,1,0):
                d.click(136+i*26,70+i*24)
                d.wait(lambda:d.serial().count('[desktop] application retired:')>=12-i,'remaining signed child not reaped')
            # Retirement and periodic native observation are independent.
            # Require a fresh exact teardown sample before the serial shell
            # can halt the machine; an old one-child sample is not a leak.
            d.wait(lambda:samples(d)[-1][1:]==samples(d)[0][1:],
                   'signed teardown resources did not return exactly')
            d.shot('signed-all-closed')
            return b'shutdown\r'
        finally:d.dispose()
    feed=[((b'arena>',b'[desktop] real desktop frame presented',b'packaged READY'),1,b'pkg graphics\r')]
    for n in range(1,5):feed.append((b'servicemgr: real signed Image delegated for broker-owned graphical spawn',n,visible))
    feed.extend([(b'servicemgr: graphical Image launch bounded refusal',1,exercised),(b'servicemgr: revoked graphical Image cannot spawn again PASS',1,revoked)])
    rc,s,_=mtest.boot(LABEL,esp,feed,disk,pointer=True,timeout_s=120)
    assert rc==0 and s.count('[desktop] real application spawned;')==12 and s.count('[desktop] application retired:')==12
    rows=[tuple(map(int,m)) for m in re.findall(r'measured frames/records/processes/regions/pages/maps/caps=(\d+)/(\d+)/(\d+)/(\d+)/(\d+)/(\d+)/(\d+)',s)]
    assert rows[-1][1:]==rows[0][1:],('mixed teardown resources',rows[0],rows[-1])
    assert 'GRAPHICALTEST refused' not in s
    assert stage.contents(disk)[stage.STAGE1]==signed and all(stage.contents(disk)[k]==v for k,v in original.items())
    assert not __import__('afs1').audit(disk)
    print(f'[m10-dynamic] signed ELF {len(elf)} bytes sha256={hashlib.sha256(elf).hexdigest()}; four broker-owned real dynamic graphical processes plus eight ordinary builtins, mixed thirteenth-session refusal, native counters, owned key pixels, unpublished drawing remains invisible until authenticated Damage, dock-safe title movement/close, Process close and revoke-with-live-copied-pages PASS')
if __name__=='__main__':main()
