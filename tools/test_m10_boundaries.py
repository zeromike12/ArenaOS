#!/usr/bin/env python3
"""Small real desktop oracle for authority, pointer, spawn and exact cleanup."""
import re
import arena_env
import mtest
from test_m10_apps import Desktop
from test_m10_desktop import crop
LABEL='m10-boundaries'
def samples(d):
    return [tuple(map(int,m)) for m in re.findall(r'measured frames/records/processes/regions/pages/maps/caps=(\d+)/(\d+)/(\d+)/(\d+)/(\d+)/(\d+)/(\d+)',d.serial())]
def workflow():
    d=Desktop(LABEL)
    try:
        empty=d.shot('empty');base=samples(d)[0]
        terminal=d.launch(0,'terminal');gallery=d.launch(5,'gallery',1)
        d.click(82,70)
        d.shot('pointer-focus',lambda p:crop(p,75,65,16,14)!=crop(gallery,75,65,16,14))
        d.q.type_text('echo key abc\r',gap_s=.04)
        d.shot('terminal-key',lambda p:crop(p,90,164,290,28)!=crop(terminal,90,164,290,28))
        d.click(535,150)
        before=d.shot('gallery-focused',lambda p:crop(p,130,144,240,160)==crop(gallery,130,144,240,160))
        d.q.key('t')
        def changed_pixels(p):
            a=crop(p,130,144,240,160);b=crop(before,130,144,240,160)
            return sum(a[i:i+3]!=b[i:i+3] for i in range(0,len(a),3))>240*160*.1
        d.shot('gallery-key',changed_pixels)
        changed=d.settled('gallery-key',(130,144,240,160),changed_pixels)
        content=crop(changed,130,144,240,160)
        d.point(116,94,True);d.point(316,194);d.point(316,194,False);d.point(780,500)
        d.shot('exact-drag',lambda p:crop(p,330,244,240,160)==content)
        monitor=d.launch(4,'monitor',2)
        assert samples(d)[-1][2]==base[2]+3,'launcher did not create three actual processes'
        assert len(set(crop(monitor,144,182,350,130)[i:i+3] for i in range(0,350*130*3,3)))>=2
        d.close(2);d.wait(lambda:d.serial().count('[desktop] application retired:')>=1,'monitor not retired')
        d.click(732,194);d.wait(lambda:d.serial().count('[desktop] application retired:')>=2,'moved gallery not retired')
        d.close();d.wait(lambda:d.serial().count('[desktop] application retired:')>=3,'terminal not retired')
        d.wait(lambda:samples(d)[-1][1:]==base[1:],'process/map/region/cap resources did not return exactly')
        d.shot('clean-empty',lambda p:crop(p,100,110,300,180)==crop(empty,100,110,300,180))
        assert base[0]-samples(d)[-1][0]==3,'bounded broker PT residency differs from three warmed VA slots'
        return b'shutdown\r'
    finally:d.dispose()
def main(esp=None):
    if esp is None:esp=mtest.build(LABEL,desktop=True)
    rc,s,_=mtest.boot(LABEL,esp,[((b'[desktop] real desktop frame presented',b'arena>'),1,workflow)],arena_env.make_scratch_disk(),pointer=True)
    assert rc==0 and s.count('[desktop] real application spawned;')==3 and s.count('[desktop] application retired:')==3
    print('[m10-boundaries] native function/diagnostic audits; actual launch/process counts; pointer activation; owned key raster/exact drag; Process, cap, SharedRegion/map cleanup PASS')
if __name__=='__main__':main()
