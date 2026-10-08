#!/usr/bin/env python3
"""Shipping keyboard-only desktop, real F-key launch/focus/close and own input."""
import arena_env
import mtest
from test_m10_apps import Desktop
from test_m10_desktop import crop

LABEL='m10-keyboard'
def workflow():
    d=Desktop(LABEL)
    def key(q):d.q.command('input-send-event',events=[d.q._ev(q,True),d.q._ev(q,False)])
    try:
        empty=d.shot('empty')
        key('f6');gallery=d.opened('gallery',0)
        key('f1');d.opened('terminal',1)
        key('f7')
        raised=d.shot('gallery-focused',lambda p:crop(p,100,150,250,100)==crop(gallery,100,150,250,100))
        d.q.key('t')
        d.shot('gallery-key',lambda p:crop(p,100,150,250,100)!=crop(raised,100,150,250,100))
        key('f8')
        terminal=d.shot('terminal-restored',lambda p:crop(p,100,150,250,100)!=crop(gallery,100,150,250,100))
        d.q.type_text('echo keyboard only\r',gap_s=.04)
        d.shot('terminal-echo',lambda p:crop(p,120,220,250,80)!=crop(terminal,120,220,250,80))
        key('f8')
        d.shot('empty-again',lambda p:crop(p,100,150,250,100)==crop(empty,100,150,250,100))
        d.wait(lambda:d.serial().count('[desktop] application retired:')==2,'keyboard close did not consume actual Processes')
        return b'shutdown\r'
    finally:d.dispose()
def main():
    esp=mtest.build(LABEL,desktop=True)
    rc,s,_=mtest.boot(LABEL,esp,[((b'arena>',b'[desktop] real desktop frame presented'),1,workflow)],arena_env.make_scratch_disk())
    assert rc==0 and s.count('[desktop] real application spawned;')==2
    assert 'echo keyboard only\n' not in s and 'kernel] shutdown requested by pid' in s
    print('[m10-keyboard] actual shipping desktop with one keyboard: F1..F6 ordinary launch, F7 focus, F8 owned close and graphical keyboard output; serial shutdown remains independent PASS')
if __name__=='__main__':main()
