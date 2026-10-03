#!/usr/bin/env python3
"""Real QMP app references; independent-boot gallery raster determinism.

PNG encoding uses only the standard library. Full desktop references contain
real uptime and diagnostic state; only the owned gallery crop is a golden.
Functional gates separately use structural pixels and native accounting.
"""
import hashlib
import json
import shutil
import struct
import subprocess
import zlib
import arena_env
import afs1
import mtest
import test_m84_stage as seed
from test_m10_apps import Desktop
from test_m10_desktop import crop

BUILD=arena_env.build_dir(); ROOT=arena_env.REPO_ROOT
OUT=BUILD/'phase10-screenshots'
NAMES=('terminal','files','editor','settings','monitor','gallery')

def png(width,height,pixels):
    assert len(pixels)==width*height*3
    def chunk(kind,data):
        return struct.pack('>I',len(data))+kind+data+struct.pack('>I',zlib.crc32(kind+data))
    scan=b''.join(b'\0'+pixels[y*width*3:(y+1)*width*3] for y in range(height))
    return b'\x89PNG\r\n\x1a\n'+chunk(b'IHDR',struct.pack('>IIBBBBB',width,height,8,2,0,0,0))+chunk(b'IDAT',zlib.compress(scan,9))+chunk(b'IEND',b'')

def preference():
    data=b'UI10\x01\x00\x00\x00'; value=0xcbf29ce484222325
    for byte in data:value=((value^byte)*0x100000001b3)&0xffffffffffffffff
    return data+struct.pack('<Q',value)

def main(esp=None):
    if esp is None:esp=mtest.build('m10-handoff',desktop=True)
    OUT.mkdir(parents=True,exist_ok=True)
    entries=[]; gallery={}
    def save(name,data,region=None,deterministic=False):
        w,h=(800,600) if region is None else region[2:]
        encoded=png(w,h,data if region is None else crop(data,*region))
        (OUT/f'{name}.png').write_bytes(encoded)
        entries.append(dict(path=f'{name}.png',width=w,height=h,sha256=hashlib.sha256(encoded).hexdigest(),deterministic=deterministic))
    disk=arena_env.make_scratch_disk()
    rc,_,_=mtest.boot('m10-handoff-seed',esp,[(b'arena>',1,b'shutdown\r')],disk,pointer=True);assert rc==0
    seed.host_seed(disk,{b'ui10-prefs':preference(),b'user-note':b'ArenaOS desktop reference\nOrdinary application, owned pixels.\n'})
    repeat=BUILD/'m10-handoff-repeat.img';shutil.copyfile(disk,repeat)
    label='m10-handoff'
    def workflow():
        d=Desktop(label)
        try:
            save('desktop-light',d.shot('empty'))
            for kind,name in enumerate(NAMES):
                opened=d.launch(kind,name)
                if kind==0:
                    d.q.type_text('echo ArenaOS desktop\rhelp\r',gap_s=.04)
                elif kind==1:d.click(110,142)
                elif kind==2:
                    d.click(345,110);d.q.key('\r')
                    d.shot('editor-loaded',lambda p:crop(p,88,134,200,20)!=crop(opened,88,134,200,20))
                data=d.settled(name+'-reference',(70,60,448,288))
                save(name,data)
                if kind==5:
                    gallery['light']=crop(data,70,60,448,288)
                    save('gallery-owned-light',data,(70,60,448,288),True)
                    d.q.key('t')
                    data=d.settled('gallery-palette-dark',(70,60,448,288),lambda p:crop(p,82,96,400,220)!=crop(opened,82,96,400,220))
                    gallery['dark']=crop(data,70,60,448,288)
                    save('gallery-owned-dark',data,(70,60,448,288),True)
                d.close()
                d.wait(lambda:d.serial().count('[desktop] application retired:')>=kind+1,'reference app not retired')
            for kind,name in enumerate(NAMES):d.launch(kind,'full-'+name,kind)
            save('desktop-six-apps',d.settled('full-reference',(200,220,350,140)))
            for i in reversed(range(6)):
                d.close(i)
                d.wait(lambda:d.serial().count('[desktop] application retired:')>=12-i,'full reference app not retired')
            before=d.launch(3,'appearance')
            d.click(270,160)
            save('settings-dark',d.settled('appearance-dark',(70,60,448,288),lambda p:crop(p,600,200,40,40)!=crop(before,600,200,40,40)))
            d.close();d.wait(lambda:d.serial().count('[desktop] application retired:')>=13,'appearance not retired')
            save('desktop-dark',d.shot('empty-dark',lambda p:crop(p,100,110,20,20)==crop(p,600,200,20,20)))
            return b'shutdown\r'
        finally:d.dispose()
    rc,_,_=mtest.boot(label,esp,[((b'[desktop] real desktop frame presented',b'arena>'),1,workflow)],disk,pointer=True);assert rc==0
    label='m10-handoff-independent'
    def independent():
        d=Desktop(label)
        try:
            data=d.launch(5,'gallery-independent')
            assert crop(data,70,60,448,288)==gallery['light'],'independent-boot light gallery differs'
            d.q.key('t')
            data=d.settled('gallery-independent-dark',(70,60,448,288),lambda p:crop(p,70,60,448,288)!=gallery['light'])
            assert crop(data,70,60,448,288)==gallery['dark'],'independent-boot dark gallery differs'
            d.close();d.wait(lambda:d.serial().count('[desktop] application retired:')==1,'independent gallery not retired')
            return b'shutdown\r'
        finally:d.dispose()
    rc,_,_=mtest.boot(label,esp,[((b'[desktop] real desktop frame presented',b'arena>'),1,independent)],repeat,pointer=True);assert rc==0
    assert not afs1.audit(disk) and not afs1.audit(repeat)
    manifest=dict(source_commit=subprocess.check_output(['git','rev-parse','HEAD'],cwd=ROOT,text=True).strip(),efi_sha256=hashlib.sha256((BUILD/'arena-boot.efi').read_bytes()).hexdigest(),resolution='800x600 GOP',fixture='user-note text; canonical light/motion-disabled UI10 preference; fresh AFS1; cursor parked at 780,500',capture='QMP screendump, settled owned raster; lossless RGB PNG',entries=entries)
    (OUT/'manifest.json').write_text(json.dumps(manifest,indent=2)+'\n')
    print('[m10-handoff] actual desktop, all six real apps, full working set, durable dark appearance; independent-boot byte-identical light/dark owned gallery QMP rasters PASS',flush=True)

if __name__=='__main__':main()
