#!/usr/bin/env python3
"""Real tablet + ordinary spawned gallery; semantic pixels, no Sol RGB golden."""
import re
import time
from pathlib import Path
import arena_env
import mtest
import qmp
LABEL='m10-desktop'
BUILD=arena_env.build_dir()

def ppm(path):
    data=path.read_bytes();h=re.match(rb'P6\s+(\d+)\s+(\d+)\s+255\s',data)
    assert h and (int(h[1]),int(h[2]))==(800,600)
    return data[h.end():]

def crop(data,x,y,w,h):
    return b''.join(data[((y+row)*800+x)*3:((y+row)*800+x+w)*3] for row in range(h))

def workflow():
    c=qmp.Qmp(str(BUILD/f'qmp-{LABEL}.sock'))
    def shot(name,predicate=lambda data:True):
        end=time.monotonic()+8
        path=BUILD/f'{LABEL}-{name}.ppm'
        while time.monotonic()<end:
            c.command('screendump',filename=str(path),format='ppm');data=ppm(path)
            if predicate(data):return data
            time.sleep(.04)
        raise AssertionError(f'visual state missing: {name}')
    def point(x,y,down=None):
        events=[{'type':'abs','data':{'axis':'x','value':(x*32767+799)//799}},
                {'type':'abs','data':{'axis':'y','value':(y*32767+599)//599}}]
        if down is not None:events.append({'type':'btn','data':{'button':'left','down':down}})
        c.command('input-send-event',events=events)
    try:
        empty=shot('empty')
        point(545,570,True);point(545,570,False)
        opened=shot('gallery',lambda p:crop(p,100,100,100,80)!=crop(empty,100,100,100,80))
        # Wait for actual client drawing: nonuniform list/control/body region.
        opened=shot('gallery',lambda p:len(set(crop(p,200,150,70,70)[i:i+3] for i in range(0,70*70*3,3)))>3)
        c.key('t')
        changed=shot('keyboard',lambda p:crop(p,100,100,100,100)!=crop(opened,100,100,100,100))
        # Title drag translates the exact owned application raster.
        point(90,70,True);point(290,170);point(290,170,False);point(780,500)
        content=crop(changed,100,110,300,180)
        moved=shot('moved',lambda p:crop(p,300,210,300,180)==content)
        assert crop(moved,100,110,100,100)!=crop(changed,100,110,100,100)
        point(706,170,True);point(706,170,False);point(780,500)
        closed=shot('closed',lambda p:crop(p,300,210,300,180)==crop(empty,300,210,300,180))
        assert crop(closed,100,110,100,100)==crop(empty,100,110,100,100)
        # Relaunch uses a new Process/region and produces an ordinary client.
        point(545,570,True);point(545,570,False);point(780,500)
        shot('relaunched',lambda p:len(set(crop(p,200,150,70,70)[i:i+3] for i in range(0,70*70*3,3)))>3)
    finally:c.close()
    return b'shutdown\r'

def main():
    esp=mtest.build(LABEL,desktop=True)
    rc,s,_=mtest.boot(LABEL,esp,[(b'[desktop] real desktop frame presented',1,workflow)],arena_env.make_scratch_disk(),pointer=True)
    assert rc==0
    assert s.count('[desktop] real application spawned;')==2
    assert '[desktop] application retired:' in s
    print('[m10-desktop] QMP real dock spawn, owned raster, keyboard theme, drag, close and relaunch PASS')
if __name__=='__main__':main()
