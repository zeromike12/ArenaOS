#!/usr/bin/env python3
"""Real app processes, owned pixels, durable bytes and full-desktop accounting."""
import re
import time
import afs1
import arena_env
import mtest
import qmp
from test_m10_desktop import ppm, crop

BUILD=arena_env.build_dir()

def file_bytes(path,name):
    d=afs1.Disk(path.read_bytes());_,ot,_=d.commit();o=d.find(name,ot)
    if o is None:return None
    return b''.join(d.data[a*512:(a+n)*512] for a,n in d.extents(o['extent_head']))[:o['size']]

class Desktop:
    def __init__(self,label):
        self.label=label
        self.q=qmp.Qmp(str(BUILD/f'qmp-{label}.sock'))
    def point(self,x,y,down=None):
        events=[{'type':'abs','data':{'axis':'x','value':(x*32767+799)//799}},
                {'type':'abs','data':{'axis':'y','value':(y*32767+599)//599}}]
        if down is not None:events.append({'type':'btn','data':{'button':'left','down':down}})
        self.q.command('input-send-event',events=events)
    def click(self,x,y):
        self.point(x,y,True);self.point(x,y,False);self.point(780,500)
    def shot(self,name,predicate=lambda p:True):
        path=BUILD/f'{self.label}-{name}.ppm';end=time.monotonic()+10
        while time.monotonic()<end:
            self.q.command('screendump',filename=str(path),format='ppm');data=ppm(path)
            if predicate(data):return data
            time.sleep(.04)
        raise AssertionError(f'missing graphical state {name}')
    def wait(self,predicate,description):
        end=time.monotonic()+10
        while time.monotonic()<end:
            if predicate():return
            time.sleep(.04)
        raise AssertionError(description)
    def opened(self,name,index=0):
        x=70+index*26;y=60+index*24
        self.shot(name,lambda p:len(set(crop(p,x+12,y+8,180,10)[i:i+3] for i in range(0,180*10*3,3)))>=2 and len(set(crop(p,x+12,y+68,300,180)[i:i+3] for i in range(0,300*180*3,3)))>=2)
        previous=None;equal=0;end=time.monotonic()+10
        while time.monotonic()<end:
            data=self.shot(name);region=crop(data,x+12,y+68,300,180)
            equal=equal+1 if region==previous else 0;previous=region
            if equal>=3:return data
            time.sleep(.05)
        raise AssertionError(f'window did not settle {name}')
    def launch(self,kind,name,index=0):
        self.click(255+kind*58,570)
        return self.opened(name,index)
    def close(self,index=0):self.click(506+index*26,70+index*24)
    def serial(self):return (BUILD/f'serial-{self.label}.log').read_text()
    def dispose(self):self.q.close()

def workflow(label,disk):
    d=Desktop(label)
    try:
        empty=d.shot('empty')
        terminal=d.launch(0,'terminal')
        d.q.type_text('put user-note hello desktop\r',gap_s=.04)
        d.wait(lambda:file_bytes(disk,b'user-note')==b'hello desktop','terminal did not commit actual file')
        d.q.type_text('echo pointer and keyboard\r',gap_s=.04)
        d.shot('terminal-echo',lambda p:crop(p,82,128,360,160)!=crop(terminal,82,128,360,160))
        d.q.type_text('shutdown\r',gap_s=.04)
        d.shot('terminal-ordinary',lambda p:crop(p,82,220,360,40)!=crop(terminal,82,220,360,40))
        assert 'kernel] shutdown requested by pid' not in d.serial(),'graphical terminal inherited Power'
        d.close();d.shot('terminal-closed',lambda p:crop(p,100,110,300,180)==crop(empty,100,110,300,180))
        blank=d.launch(2,'editor')
        d.click(345,110)
        d.shot('editor-open-dialog',lambda p:crop(p,82,96,354,24)!=crop(blank,82,96,354,24))
        d.q.key('\r')
        editor=d.shot('editor-open',lambda p:crop(p,94,134,94,10)!=crop(blank,94,134,94,10))
        d.q.type_text('saved ',gap_s=.04)
        typed=d.shot('editor-typed',lambda p:crop(p,88,134,200,10)!=crop(editor,88,134,200,10))
        assert file_bytes(disk,b'user-note')==b'hello desktop','typing falsely saved file'
        d.click(185,110)
        d.wait(lambda:file_bytes(disk,b'user-note')==b'saved hello desktop','editor save failed exact durable bytes')
        d.q.type_text('dirty',gap_s=.04)
        d.shot('editor-dirty',lambda p:crop(p,88,134,200,10)!=crop(typed,88,134,200,10))
        d.close()
        pending=d.shot('editor-close-decision',lambda p:crop(p,82,96,354,24)!=crop(typed,82,96,354,24))
        d.click(450,110) # Cancel: window and dirty text remain, owned CancelClose.
        d.shot('editor-close-cancelled',lambda p:crop(p,82,96,354,24)!=crop(pending,82,96,354,24))
        d.close();d.click(360,110) # Discard only after a new close request.
        d.shot('editor-closed',lambda p:crop(p,100,110,300,180)==crop(empty,100,110,300,180))
        assert file_bytes(disk,b'user-note')==b'saved hello desktop'
        files=d.launch(1,'files');d.click(105,110)
        d.shot('files-new-dialog',lambda p:crop(p,82,96,354,24)!=crop(files,82,96,354,24))
        d.q.key('\r')
        d.wait(lambda:file_bytes(disk,b'user-new')==b'','file manager did not create actual file')
        d.shot('files-created');d.close()
        d.shot('files-closed',lambda p:crop(p,100,110,300,180)==crop(empty,100,110,300,180))
        settings=d.launch(3,'settings');d.click(270,160)
        dark=d.shot('settings-dark',lambda p:crop(p,600,200,40,40)!=crop(settings,600,200,40,40))
        d.wait(lambda:(file_bytes(disk,b'ui10-prefs') or b'')[:7]==b'UI10\x01\x01\x01','theme preference not persisted')
        d.click(270,200)
        d.wait(lambda:(file_bytes(disk,b'ui10-prefs') or b'')[:7]==b'UI10\x01\x01\x00','motion preference not persisted')
        d.close();dark_empty=d.shot('dark-empty',lambda p:crop(p,100,110,10,10)==crop(p,600,200,10,10))
        # Fill every bounded desktop session with actual ordinary processes.
        for kind in range(6):d.launch(kind,f'full-app-{kind}',kind)
        full=d.shot('full-desktop')
        before=d.serial().count('[desktop] real application spawned;')
        d.click(255,570)
        d.shot('capacity-refused',lambda p:crop(p,10,28,250,20)!=crop(full,10,28,250,20))
        assert d.serial().count('[desktop] real application spawned;')==before,'capacity refusal spawned a seventh process'
        for i in reversed(range(6)):
            d.close(i)
            d.wait(lambda:d.serial().count('[desktop] application retired:')>=10-i,'close failed to retire held Process')
        d.shot('all-closed',lambda p:crop(p,100,110,300,180)==crop(dark_empty,100,110,300,180))
        # Existing VM policy retains empty intermediate page tables until
        # the owning address space dies. Warm all six fixed VA slots, then
        # independently prove the complete working set can cycle without growth.
        for cycle in range(2):
            for kind in range(6):d.launch(kind,f'cycle-{cycle}-app-{kind}',kind)
            for i in reversed(range(6)):
                d.close(i)
                d.wait(lambda:d.serial().count('[desktop] application retired:')>=10+cycle*6+6-i,'cycle did not retire exact child')
            d.shot(f'cycle-{cycle}-closed',lambda p:crop(p,100,110,300,180)==crop(dark_empty,100,110,300,180))
        # Reopen without any generation/cap/resource growth.
        d.launch(5,'gallery-reopened');d.q.key('t');d.shot('gallery-local-theme');d.close()
        d.shot('reopened-closed',lambda p:crop(p,100,110,300,180)==crop(dark_empty,100,110,300,180))
        d.wait(lambda:d.serial().count('[desktop] application retired:')>=23,'last application not reaped before shutdown')
        def clean_snapshot():
            samples=re.findall(r'measured frames/records/processes/regions/pages/maps/caps=(\d+)/(\d+)/(\d+)/(\d+)/(\d+)/(\d+)/(\d+)',d.serial())
            return len(samples)>1 and samples[-1][1:]==samples[0][1:]
        d.wait(clean_snapshot,'final measured teardown not complete before shutdown')
        return b'shutdown\r'
    finally:d.dispose()

def main(esp=None):
    if esp is None:esp=mtest.build('m10-apps',desktop=True)
    disk=arena_env.make_scratch_disk();label='m10-apps'
    rc,s,_=mtest.boot(label,esp,[((b'[desktop] real desktop frame presented',b'arena>'),1,lambda:workflow(label,disk))],disk,pointer=True)
    assert rc==0
    assert afs1.audit(disk)==[] and file_bytes(disk,b'user-note')==b'saved hello desktop'
    samples=[tuple(map(int,m)) for m in re.findall(r'measured frames/records/processes/regions/pages/maps/caps=(\d+)/(\d+)/(\d+)/(\d+)/(\d+)/(\d+)/(\d+)',s)]
    assert samples and samples[-1][1:]==samples[0][1:],(samples[0],samples[-1])
    assert samples[0][0]-samples[-1][0]==6,'unexpected retained frames beyond the six existing intermediate PTs'
    empty=[row for row in samples if row[1:]==samples[0][1:]]
    assert len(empty)>=5 and all(row==samples[-1] for row in empty[-4:]),empty
    peak=min(samples,key=lambda row:row[0]);assert peak[1:4]==(samples[0][1]+6,samples[0][2]+6,samples[0][3]+6),peak
    assert peak[4]==samples[0][4]+6*127 and peak[5]<=32 and peak[6]<=32,peak
    print(f'[m10-apps] real six-app desktop, terminal commands, file create, editor exact transactional save/unsaved-close, durable theme/motion, monitor, capacity refusal and exact cleanup PASS; baseline={samples[0]} peak={peak}',flush=True)
    # Durable appearance must affect actual desktop pixels on a fresh boot.
    label='m10-apps-persist'
    def persisted():
        d=Desktop(label)
        try:
            dark=ppm(BUILD/'m10-apps-dark-empty.ppm')
            d.shot('persisted',lambda p:crop(p,600,200,80,80)==crop(dark,600,200,80,80))
            return b'shutdown\r'
        finally:d.dispose()
    rc,s,_=mtest.boot(label,esp,[(b'[desktop] real desktop frame presented',1,persisted)],disk,pointer=True)
    assert rc==0 and afs1.audit(disk)==[]
    print('[m10-apps] independent fresh boot reads durable preference and changes desktop pixels PASS')
if __name__=='__main__':main()
