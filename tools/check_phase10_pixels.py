#!/usr/bin/env python3
"""Standalone QMP witness for the shipping Phase-10 desktop.

Real dock spawn, owned keyboard raster, exact pointer drag, Process close and
relaunch are checked with pixels and actual native resource snapshots. No
palette golden or fake serial-only UI proof. Also runs from an extracted bundle.
"""
import hashlib
import re
import sys
import time
from pathlib import Path
import qmp
READY=b'[desktop] real desktop frame presented'
COUNTERS=re.compile(rb'measured frames/records/processes/regions/pages/maps/caps=(\d+)/(\d+)/(\d+)/(\d+)/(\d+)/(\d+)/(\d+)\r?\n')
def capture(sock,serial,image,receipt,timeout):
    deadline=time.monotonic()+timeout
    def log():return serial.read_bytes() if serial.is_file() else b''
    def wait(predicate,message):
        while time.monotonic()<deadline:
            if predicate():return
            time.sleep(.04)
        raise TimeoutError(message)
    def counts():return [tuple(map(int,m)) for m in COUNTERS.findall(log())]
    wait(lambda:READY in log() and b'arena>' in log() and b'servicemgr: production netstackd READY pid' in log() and counts(),'desktop/native snapshot/boot completion absent')
    base=counts()[0];conn=qmp.Qmp(str(sock),connect_timeout_s=4)
    def crop(p,x,y,w,h):return b''.join(p[((y+r)*800+x)*3:((y+r)*800+x+w)*3] for r in range(h))
    def shot(predicate=lambda p:True):
        while time.monotonic()<deadline:
            conn.command('screendump',filename=str(image),format='ppm')
            raw=image.read_bytes();head=re.match(rb'P6\s+(\d+)\s+(\d+)\s+255\s',raw)
            if not head or (int(head[1]),int(head[2]))!=(800,600) or len(raw)-head.end()!=800*600*3:
                raise ValueError('desktop image is not the tested 800x600 P6 scanout')
            pixels=raw[head.end():]
            if predicate(pixels):return pixels,hashlib.sha256(raw).hexdigest()
            time.sleep(.04)
        raise TimeoutError('owned graphical state absent')
    def point(x,y,down=None):
        events=[{'type':'abs','data':{'axis':'x','value':(x*32767+799)//799}},
                {'type':'abs','data':{'axis':'y','value':(y*32767+599)//599}}]
        if down is not None:events.append({'type':'btn','data':{'button':'left','down':down}})
        conn.command('input-send-event',events=events)
    def click(x,y):point(x,y,True);point(x,y,False);point(780,500)
    def opened():
        shot(lambda p:len(set(crop(p,200,150,70,70)[i:i+3] for i in range(0,70*70*3,3)))>3)
        previous=None;equal=0
        while time.monotonic()<deadline:
            p,sha=shot();body=crop(p,100,110,300,180)
            equal=equal+1 if body==previous else 0;previous=body
            if equal>=3:return p,sha
            time.sleep(.05)
        raise TimeoutError('open reveal did not settle')
    try:
        empty,_=shot();click(545,570);first,before_sha=opened()
        wait(lambda:counts()[-1][2]==base[2]+1,'dock did not spawn actual process')
        conn.key('t')
        changed,after_sha=shot(lambda p:crop(p,100,110,300,180)!=crop(first,100,110,300,180))
        previous=None;equal=0
        while equal<3:
            changed,after_sha=shot();body=crop(changed,100,110,300,180)
            equal=equal+1 if body==previous else 0;previous=body;time.sleep(.05)
        point(90,70,True);point(290,170);point(290,170,False);point(780,500)
        moved,moved_sha=shot(lambda p:crop(p,300,210,300,180)==crop(changed,100,110,300,180))
        if crop(moved,100,110,100,100)==crop(changed,100,110,100,100):raise ValueError('old location did not uncover')
        click(706,170)
        shot(lambda p:crop(p,300,210,300,180)==crop(empty,300,210,300,180))
        wait(lambda:log().count(b'[desktop] application retired:')==1 and counts()[-1][1:]==base[1:],'first close did not retire exact resources')
        warm=counts()[-1]
        # One warmed broker VA slot: two retained PTs (ADR-0075 reservation).
        if base[0]-warm[0]!=2:raise ValueError('one warmed broker PT residency not exact')
        click(545,570);second,_=opened();conn.key('t')
        _,final_sha=shot(lambda p:crop(p,100,110,300,180)!=crop(second,100,110,300,180))
        click(506,70)
        shot(lambda p:crop(p,100,110,300,180)==crop(empty,100,110,300,180))
        wait(lambda:log().count(b'[desktop] application retired:')==2 and counts()[-1]==warm,'relaunch/close leaked native resources')
        if log().count(b'[desktop] real application spawned;')!=2:raise ValueError('wrong real launch count')
        # Retain the actually painted empty end state and independent input hashes.
        _,end_sha=shot()
        receipt.write_text(f'800x600 {end_sha}\nINPUT {before_sha} {after_sha}\nDRAG {moved_sha}\nRELAUNCH {final_sha}\nRESOURCES '+','.join(map(str,base))+' '+','.join(map(str,warm))+'\nLIFECYCLE 2 2\n')
    finally:conn.close()
def main():
    if len(sys.argv)!=6:raise SystemExit('usage: check_phase10_pixels.py QMP_SOCKET SERIAL PPM RECEIPT TIMEOUT_S')
    sock,serial,image,receipt=map(Path,sys.argv[1:5])
    try:capture(sock,serial,image,receipt,float(sys.argv[5]))
    except Exception as error:
        receipt.with_suffix(receipt.suffix+'.error').write_text(str(error)+'\n')
        print(f'[phase10-pixels] FAIL: {error}',file=sys.stderr);return 1
    print('[phase10-pixels] real dock/process/key/drag/close/relaunch/resource proof PASS',flush=True);return 0
if __name__=='__main__':sys.exit(main())
