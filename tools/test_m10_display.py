#!/usr/bin/env python3
"""Shipping desktop GOP + real GPU at 800x600/640x480, full six-app pressure."""
import re
import time
import arena_env
import mtest
import qmp
from test_m10_apps import serial_text, receipts
BUILD=arena_env.build_dir()
def workflow(label,width,height):
    serial=BUILD/f'serial-{label}.log';conn=qmp.Qmp(str(BUILD/f'qmp-{label}.sock'))
    def wait(predicate,message):
        end=time.monotonic()+12
        while time.monotonic()<end:
            if predicate():return
            time.sleep(.04)
        raise AssertionError(message)
    def text():return serial_text(serial)
    def samples():return receipts(text())
    def crop(p,x,y,w,h):return b''.join(p[((y+r)*width+x)*3:((y+r)*width+x+w)*3] for r in range(h))
    def shot(name,predicate=lambda p:True):
        path=BUILD/f'{label}-{name}.ppm';end=time.monotonic()+12
        while time.monotonic()<end:
            conn.command('screendump',filename=str(path),format='ppm');raw=path.read_bytes()
            head=re.match(rb'P6\s+(\d+)\s+(\d+)\s+255\s',raw)
            assert head and (int(head[1]),int(head[2]))==(width,height)
            pixels=raw[head.end():];assert len(pixels)==width*height*3
            if predicate(pixels):return pixels
            time.sleep(.04)
        raise AssertionError('missing actual mode/pixel state '+name)
    def point(x,y,down=None):
        ev=[{'type':'abs','data':{'axis':'x','value':(x*32767+width-1)//(width-1)}},
            {'type':'abs','data':{'axis':'y','value':(y*32767+height-1)//(height-1)}}]
        if down is not None:ev.append({'type':'btn','data':{'button':'left','down':down}})
        conn.command('input-send-event',events=ev)
    def click(x,y):point(x,y,True);point(x,y,False);point(width-20,height-100)
    def key(name):conn.command('input-send-event',events=[conn._ev(name,True),conn._ev(name,False)])
    try:
        wait(lambda:samples(),'initial complete mode/native accounting sample absent')
        base=samples()[0];empty=shot('empty');dock_x=(width-348)//2+29
        click(dock_x+5*58,height-30)
        first=shot('gallery',lambda p:len(set(crop(p,200,150,70,70)[i:i+3] for i in range(0,70*70*3,3)))>3)
        time.sleep(.15);first=shot('gallery-settled');conn.key('t')
        changed=shot('key',lambda p:crop(p,100,110,300,180)!=crop(first,100,110,300,180))
        previous=None;equal=0
        while equal<3:
            changed=shot('key-settled');body=crop(changed,100,110,300,180)
            equal=equal+1 if body==previous else 0;previous=body;time.sleep(.05)
        point(90,70,True);point(190,150);point(190,150,False);point(width-20,height-100)
        shot('drag',lambda p:crop(p,200,190,300,180)==crop(changed,100,110,300,180))
        key('f8');wait(lambda:text().count('[desktop] application retired:')==1,'mode gallery close did not retire process')
        shot('empty-again',lambda p:crop(p,100,110,300,180)==crop(empty,100,110,300,180))
        for kind in range(6):
            key('f'+str(kind+1))
            wait(lambda:text().count('[desktop] real application spawned;')==kind+2,'mode launcher did not spawn actual app')
            # Native grant audits, own map and real raster must complete before
            # launching the next app; slots use deterministic cascade geometry.
            shot(f'app-{kind}',lambda p:len(set(crop(p,82+kind*26,128+kind*24,250,140)[i:i+3] for i in range(0,250*140*3,3)))>=2)
        # ADR-0075: each session holds a shared and a snapshot region sized
        # for this mode's work area, mapped three times (broker x2, client).
        shared,snapshot=map(int,re.search(r'session reservation shared/snapshot pages=(\d+)/(\d+)',text()).groups())
        wait(lambda:samples()[-1][5]==base[5]+18,'six client mappings did not finish')
        peak=samples()[-1];assert peak[2:5]==(base[2]+6,base[3]+12,base[4]+6*(shared+snapshot)),peak
        assert peak[3]<=32 and peak[4]<=20480 and peak[5]<=64 and peak[6]<=64,peak
        shot('full-six-apps')
        for n in range(6):
            key('f8');wait(lambda:text().count('[desktop] application retired:')==n+2,'mode working set failed exact Process retirement')
        wait(lambda:samples()[-1][1:]==base[1:],'mode working set leaked resources')
        # Six warmed broker VA slots, two retained PTs each (ADR-0075).
        assert base[0]-samples()[-1][0]==6*2,'mode warm PT bound not exact'
        print(f'[m10-display] {label} {width}x{height}: real pixels/input/drag, six-app peak={peak}, exact teardown PASS',flush=True)
        return b'shutdown\r'
    finally:conn.close()
def main(esp=None):
    if esp is None:esp=mtest.build('m10-display',desktop=True)
    for video,width,height in [('default',800,600),('gpu',800,600),('gpu640',640,480)]:
        label='m10-display-'+video
        rc,s,_=mtest.boot(label,esp,[((b'[desktop] real desktop frame presented',b'arena>'),1,lambda:workflow(label,width,height))],arena_env.make_scratch_disk(),pointer=True,video=video,timeout_s=120)
        assert rc==0 and s.count('[desktop] real application spawned;')==7 and s.count('[desktop] application retired:')==7
        if video.startswith('gpu'):assert '[displayd] ring3 virtio-gpu 2D pixels ready' in s
    print('[m10-display] independently booted GOP800 and actual GPU800/GPU640 shipping desktop modes PASS')
if __name__=='__main__':main()
