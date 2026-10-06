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
NATIVE_COUNTERS=re.compile(r'measured frames/records/processes/regions/pages/maps/caps=(\d+)/(\d+)/(\d+)/(\d+)/(\d+)/(\d+)/(\d+)\r?\n')
# Resource receipts are judged only after the boot's transient processes
# ended (packaged's boot scan exits before READY, then the permission app
# is reaped). The desktop may present before that, and its earliest
# receipts then still count them (Phase 11: the reply handoff brought the
# first frame forward), so they are never a base.
SETTLED='servicemgr: permission app reaped through held Process cap'
def receipts(text):
    at=text.find(SETTLED)
    return [] if at<0 else [tuple(map(int,m)) for m in NATIVE_COUNTERS.findall(text[at:])]

def serial_text(path):
    # QEMU appends while the host reads. A final digit/UTF-8 code point may
    # still be incomplete; only newline-committed records are observations.
    data=path.read_bytes()
    return data[:data.rfind(b'\n')+1].decode('utf-8',errors='replace')

def file_bytes(path,name):
    d=afs1.Disk(path.read_bytes());_,ot,_=d.commit();o=d.find(name,ot)
    if o is None:return None
    return b''.join(d.data[a*512:(a+n)*512] for a,n in d.extents(o['extent_head']))[:o['size']]

class Desktop:
    def __init__(self,label):
        self.label=label
        self.q=qmp.Qmp(str(BUILD/f'qmp-{label}.sock'))
        try:self.wait(lambda:receipts(self.serial()),'initial complete native accounting sample absent')
        except BaseException:
            self.q.close()
            raise
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
        return self.settled(name,(x+12,y+68,300,180))
    def settled(self,name,region,predicate=lambda p:True):
        previous=None;equal=0;end=time.monotonic()+10
        while time.monotonic()<end:
            data=self.shot(name,predicate);current=crop(data,*region)
            equal=equal+1 if current==previous else 0;previous=current
            if equal>=3:return data
            time.sleep(.05)
        raise AssertionError(f'owned raster did not settle {name}')
    def launch(self,kind,name,index=0):
        self.click(255+kind*58,570)
        return self.opened(name,index)
    def close(self,index=0):self.click(506+index*26,70+index*24)
    def serial(self):return serial_text(BUILD/f'serial-{self.label}.log')
    def dispose(self):self.q.close()

def doc_bytes(path,name):
    """A document in /Users/user/Documents on the AFS2 region (Phase 11.6:
    user documents moved from flat AFS1 records to AFS2; settings stay AFS1)."""
    import afs2
    for _ in range(50):
        try:
            vol=afs2.Volume(path.read_bytes()[arena_env.AFS2_BASE_SECTOR*512:]);afs2.check(vol)
            return afs2.walk(vol).get('/Users/user/Documents/'+name.decode())
        except Exception:time.sleep(.1)
    raise AssertionError('AFS2 region never mounted on the host')

def workflow(label,disk):
    d=Desktop(label)
    try:
        settled_base=receipts(d.serial())
        assert settled_base, 'settled boot resource baseline is absent'
        base=settled_base[0]
        empty=d.shot('empty')
        terminal=d.launch(0,'terminal')
        d.q.type_text('put Documents/user-note hello desktop\r',gap_s=.04)
        d.wait(lambda:doc_bytes(disk,b'user-note')==b'hello desktop','terminal did not commit actual file')
        d.q.type_text('echo pointer and keyboard\r',gap_s=.04)
        d.shot('terminal-echo',lambda p:crop(p,82,128,360,160)!=crop(terminal,82,128,360,160))
        d.q.type_text('shutdown\r',gap_s=.04)
        d.shot('terminal-ordinary',lambda p:crop(p,82,220,360,40)!=crop(terminal,82,220,360,40))
        assert 'kernel] shutdown requested by pid' not in d.serial(),'graphical terminal inherited Power'
        d.close();d.shot('terminal-closed',lambda p:crop(p,100,110,300,180)==crop(empty,100,110,300,180))
        blank=d.launch(2,'editor')
        d.click(345,110)  # Open: the trusted chooser (Phase 11.6)
        d.shot('editor-open-dialog',lambda p:crop(p,190,130,420,340)!=crop(blank,190,130,420,340))
        for key in ('down','ret'):  # Documents holds exactly the terminal's note
            d.q.command('input-send-event',events=[d.q._ev(key,True),d.q._ev(key,False)]);time.sleep(.1)
        editor=d.shot('editor-open',lambda p:crop(p,94,134,94,10)!=crop(blank,94,134,94,10))
        d.q.type_text('saved ',gap_s=.04)
        typed=d.shot('editor-typed',lambda p:crop(p,88,134,200,10)!=crop(editor,88,134,200,10))
        assert doc_bytes(disk,b'user-note')==b'hello desktop','typing falsely saved file'
        d.click(185,110)
        d.wait(lambda:doc_bytes(disk,b'user-note')==b'saved hello desktop','editor save failed exact durable bytes')
        d.q.type_text('dirty',gap_s=.04)
        d.shot('editor-dirty',lambda p:crop(p,88,134,200,10)!=crop(typed,88,134,200,10))
        d.close()
        pending=d.shot('editor-close-decision',lambda p:crop(p,82,96,354,24)!=crop(typed,82,96,354,24))
        d.click(450,110) # Cancel: window and dirty text remain, owned CancelClose.
        d.shot('editor-close-cancelled',lambda p:crop(p,82,96,354,24)!=crop(pending,82,96,354,24))
        d.close();d.click(360,110) # Discard only after a new close request.
        d.shot('editor-closed',lambda p:crop(p,100,110,300,180)==crop(empty,100,110,300,180))
        assert doc_bytes(disk,b'user-note')==b'saved hello desktop'
        # Phase 11.8 explorer: Ctrl+N makes "Untitled.txt" at once and opens
        # its name for editing; Enter keeps it.
        files=d.launch(1,'files')
        d.q.command('input-send-event',events=[d.q._ev('ctrl',True),d.q._ev('n',True),d.q._ev('n',False),d.q._ev('ctrl',False)])
        d.shot('files-new-named',lambda p:crop(p,187,89,300,200)!=crop(files,187,89,300,200))
        d.q.key('\r')
        d.wait(lambda:doc_bytes(disk,b'Untitled.txt')==b'','file manager did not create actual file')
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
        d.click(153,142) # Activate Settings through its exposed title strip.
        regions=((75,150,5,30),(101,170,5,30),(127,190,5,30),
                 (154,220,5,30),(180,430,10,10),(600,250,30,80))
        names=('terminal','files','editor','settings','monitor','gallery')
        owned_dark=d.settled('six-dark',(600,250,30,80))
        d.click(348,232)
        d.wait(lambda:(file_bytes(disk,b'ui10-prefs') or b'')[:7]==b'UI10\x01\x00\x00','live light preference not persisted')
        for name,region in zip(names,regions):
            old=crop(owned_dark,*region)
            d.shot('live-theme-'+name,lambda p,region=region,old=old:sum(
                crop(p,*region)[i:i+3]!=old[i:i+3] for i in range(0,len(old),3))>len(old)/3*.8)
        d.click(348,232)
        d.wait(lambda:(file_bytes(disk,b'ui10-prefs') or b'')[:7]==b'UI10\x01\x01\x00','live dark preference not persisted')
        for name,region in zip(names,regions):
            d.shot('live-theme-restored-'+name,lambda p,region=region:crop(p,*region)==crop(owned_dark,*region))
        # Preserve Phase-11's twelve-session interaction/resource regression.
        # Phase 12's independent M12 scale proof fills all 32 slots and checks
        # the 33rd mutation-free refusal.
        spawned=d.serial().count('[desktop] real application spawned;')
        for kind in range(6):
            d.click(255+kind*58,570)
            d.wait(lambda:d.serial().count('[desktop] real application spawned;')>=spawned+kind+1,'second-lane session not spawned')
        d.settled('twelve-desktop',(0,26,800,500))
        # Spawn markers precede each app's asynchronous first frame and
        # filesd view. Wait for the full twelve-session working set before
        # closing it; this Phase-12 startup audit makes that boundary visible.
        expected_live=(base[1]+12,base[2]+12,base[3]+24,
                       base[4]+12*(471+469),base[5]+36+4,base[6]+2*12)
        end=time.monotonic()+30
        while time.monotonic()<end:
            current=receipts(d.serial())
            if current and current[-1][1:]==expected_live:
                break
            time.sleep(.04)
        else:
            raise AssertionError(('twelve-session startup working set did not settle',
                                  expected_live,receipts(d.serial())[-1:]))
        retired=d.serial().count('[desktop] application retired:')
        for i in range(12):
            d.q.command('input-send-event',events=[d.q._ev('f8',True),d.q._ev('f8',False)])
            d.wait(lambda:d.serial().count('[desktop] application retired:')>=retired+i+1,'F8 failed to retire held Process')
        d.shot('all-closed',lambda p:crop(p,100,110,300,180)==crop(dark_empty,100,110,300,180))
        # Existing VM policy retains empty intermediate page tables until
        # the owning address space dies. The twelve-session sweep warmed all
        # twelve broker VA slots (two PTs each: an ADR-0075 slot spans the
        # 470-page shared and 469-page snapshot mappings). Warm the six apps, then
        # independently prove the complete working set can cycle without growth.
        for cycle in range(2):
            for kind in range(6):d.launch(kind,f'cycle-{cycle}-app-{kind}',kind)
            for i in reversed(range(6)):
                d.close(i)
                d.wait(lambda:d.serial().count('[desktop] application retired:')>=16+cycle*6+6-i,'cycle did not retire exact child')
            d.shot(f'cycle-{cycle}-closed',lambda p:crop(p,100,110,300,180)==crop(dark_empty,100,110,300,180))
        # Reopen without any generation/cap/resource growth.
        d.launch(5,'gallery-reopened');d.q.key('t');d.shot('gallery-local-theme');d.close()
        d.shot('reopened-closed',lambda p:crop(p,100,110,300,180)==crop(dark_empty,100,110,300,180))
        d.wait(lambda:d.serial().count('[desktop] application retired:')>=29,'last application not reaped before shutdown')
        def clean_snapshot():
            samples=receipts(d.serial())
            return len(samples)>1 and samples[-1][1:]==samples[0][1:]
        d.wait(clean_snapshot,'final measured teardown not complete before shutdown')
        return b'shutdown\r'
    finally:d.dispose()

def main(esp=None):
    if esp is None:esp=mtest.build('m10-apps',desktop=True)
    disk=arena_env.make_scratch_disk(afs2=True);label='m10-apps'
    rc,s,_=mtest.boot(label,esp,[((b'[desktop] real desktop frame presented',b'arena>',b'AFS2 file service online'),1,lambda:workflow(label,disk))],disk,pointer=True)
    assert rc==0
    assert afs1.audit(disk)==[] and doc_bytes(disk,b'user-note')==b'saved hello desktop'
    samples=receipts(s)
    assert samples and samples[-1][1:]==samples[0][1:],(samples[0],samples[-1])
    # Superseded Phase-10 bound (six slots x one PT): ADR-0075 slots hold
    # 939 pages, so each warmed broker VA slot retains two PTs. Phase 11.6:
    # filesd maps each file session's shared region; the twelve-session
    # sweep holds four at once (two terminals, two Files), warming four PTs
    # in filesd's space (measured: 28 = 24 + 4).
    assert samples[0][0]-samples[-1][0]==12*2+4,('unexpected retained frames beyond twelve warmed slots x two PTs and four filesd session maps',samples[0],samples[-1])
    empty=[row for row in samples if row[1:]==samples[0][1:]]
    assert len(empty)>=5 and all(row==samples[-1] for row in empty[-4:]),empty
    # ADR-0075 per-session reservation, measured by the broker at boot.
    shared,snapshot=map(int,re.search(r'session reservation shared/snapshot pages=(\d+)/(\d+)',s).groups())
    # Shared = I/O page + snapshot-sized surfaces + the filesd page (ADR-0077).
    assert (shared,snapshot)==(471,469),(shared,snapshot)
    cap_peak=max(map(int,re.findall(r'measured broker cap high-water=(\d+)',s)))
    # Each managed session leaves one held Process cap and one filesd
    # lineage-head cap in the broker. The child, not the broker, owns the
    # delegated session-region cap. One input request cap is transiently
    # landed during the twelfth launch (Phase-12 startup ownership).
    assert cap_peak == samples[0][6] + 2 * 12 + 1, (cap_peak, samples[0])
    # The settled peak: most pages, then most maps (clients and filesd map
    # their views after the broker's region creation is sampled).
    peak=max(samples,key=lambda row:(row[4],row[5]))
    # Twelve sessions: one record and process each, two regions each (the
    # shared reservation and the broker-only snapshot), three maps each
    # (broker x2, client x1) plus filesd's map of each of the four file
    # sessions (two terminals, two Files). The broker retains one Process
    # and one filesd lineage head per session; the child alone holds the
    # session-region cap after spawn. Measured 68 = 44 + 24.
    assert peak[1:4]==(samples[0][1]+12,samples[0][2]+12,samples[0][3]+24),peak
    assert peak[4]==samples[0][4]+12*(shared+snapshot) and peak[5]==samples[0][5]+36+4 and peak[6]==samples[0][6]+2*12,peak
    print(f'[m10-apps] real six-app desktop, terminal commands, file create, editor exact transactional save/unsaved-close, durable theme/motion, monitor, twelve-session resource accounting and exact cleanup PASS; baseline={samples[0]} peak={peak} transient-broker-caps={cap_peak}',flush=True)
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
