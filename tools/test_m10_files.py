#!/usr/bin/env python3
"""AFS1 create refusal, real Files->Editor launch, bounded editing and Save As."""
import afs1
import arena_env
import mtest
import test_m84_stage as seed
from test_m10_apps import Desktop, file_bytes
from test_m10_desktop import crop

LABEL='m10-files';BUILD=arena_env.build_dir()
def replace_line(d,text):
    d.q.type_text('\b'*31+text+'\r',gap_s=.035)
def workflow(disk):
    d=Desktop(LABEL)
    try:
        empty=d.shot('empty');files=d.launch(1,'files')
        d.click(110,142)
        preview=d.shot('selected-preview',lambda p:len(set(crop(p,296,136,190,14)[i:i+3] for i in range(0,190*14*3,3)))>=2)
        count=d.serial().count('[desktop] real application spawned;')
        d.click(345,110);editor=d.opened('files-open-editor',1)
        assert d.serial().count('[desktop] real application spawned;')==count+1
        # Case-sensitive bytes have different real glyphs in the owned raster.
        assert crop(editor,114,158,5,7)!=crop(editor,120,158,5,7),'upper/lower case rendered identically'
        d.q.type_text('X',gap_s=.04);d.click(211,134)
        d.wait(lambda:file_bytes(disk,b'user-note')==b'XAa file manager','Files-open editor did not save actual selected file')
        d.close(1);d.wait(lambda:d.serial().count('[desktop] application retired:')>=1,'opened editor not retired')
        # New must refuse an existing name without emptying its bytes.
        d.click(105,110);dialog=d.shot('create-dialog',lambda p:crop(p,82,96,354,24)!=crop(preview,82,96,354,24))
        replace_line(d,'user-note')
        d.shot('create-refused',lambda p:crop(p,82,328,350,12)!=crop(dialog,82,328,350,12))
        assert file_bytes(disk,b'user-note')==b'XAa file manager'
        d.q.key('\x1b');d.click(420,110)
        d.wait(lambda:file_bytes(disk,b'user-note') is None,'Files delete did not unlink actual object')
        d.close();d.wait(lambda:d.serial().count('[desktop] application retired:')>=2,'Files not retired')
        blank=d.launch(2,'editor')
        d.click(345,110);d.shot('open-dialog',lambda p:crop(p,82,96,354,24)!=crop(blank,82,96,354,24))
        replace_line(d,'user-full')
        full=d.shot('full-text',lambda p:crop(p,88,134,330,140)!=crop(blank,88,134,330,140))
        d.q.key('q')
        d.shot('full-refused',lambda p:crop(p,82,328,350,12)!=crop(full,82,328,350,12))
        assert file_bytes(disk,b'user-full')==b'f'*4096
        d.q.command('input-send-event',events=[d.q._ev('delete',True),d.q._ev('delete',False)])
        d.q.type_text('Z',gap_s=.04);d.click(185,110)
        d.wait(lambda:file_bytes(disk,b'user-full')==b'Z'+b'f'*4095,'4096-byte bounded editing/save failed')
        # Save As creates real bytes through the transactional replacement API.
        d.click(265,110);saved=d.shot('save-as-dialog',lambda p:crop(p,82,96,354,24)!=crop(full,82,96,354,24))
        replace_line(d,'user-copy')
        d.wait(lambda:file_bytes(disk,b'user-copy')==b'Z'+b'f'*4095,'Save As did not commit exact 4096-byte document')
        before_open=d.shot('before-binary-open')
        d.click(345,110);d.shot('unsupported-dialog',lambda p:crop(p,82,96,354,24)!=crop(before_open,82,96,354,24))
        replace_line(d,'user-bin')
        d.shot('unsupported-refused',lambda p:crop(p,82,328,350,12)!=crop(full,82,328,350,12))
        assert file_bytes(disk,b'user-bin')==b'\x80binary' and file_bytes(disk,b'user-copy')==b'Z'+b'f'*4095
        d.q.key('\x1b');d.close();d.wait(lambda:d.serial().count('[desktop] application retired:')>=3,'editor not retired')
        d.shot('empty-again',lambda p:crop(p,100,110,300,180)==crop(empty,100,110,300,180))
        d.launch(0,'terminal');before=d.serial().count('[desktop] real application spawned;')
        d.q.type_text('launch gallery\r',gap_s=.04);d.opened('terminal-launched-gallery',1)
        assert d.serial().count('[desktop] real application spawned;')==before+1
        d.close(1);d.wait(lambda:d.serial().count('[desktop] application retired:')>=4,'terminal-launched gallery not retired')
        d.close();d.wait(lambda:d.serial().count('[desktop] application retired:')>=5,'terminal not retired')
        return b'shutdown\r'
    finally:d.dispose()
def main(esp=None):
    if esp is None:esp=mtest.build(LABEL,desktop=True)
    disk=arena_env.make_scratch_disk()
    rc,s,_=mtest.boot(LABEL+'-seed',esp,[(b'arena>',1,b'shutdown\r')],disk,pointer=True);assert rc==0
    seed.host_seed(disk,{b'user-note':b'Aa file manager',b'user-full':b'f'*4096,b'user-bin':b'\x80binary'})
    rc,s,_=mtest.boot(LABEL,esp,[((b'[desktop] real desktop frame presented',b'arena>'),1,lambda:workflow(disk))],disk,pointer=True)
    assert rc==0 and not afs1.audit(disk)
    assert file_bytes(disk,b'user-note') is None and file_bytes(disk,b'user-copy')==b'Z'+b'f'*4095
    print('[m10-files] Files actual select/read/open/delete, duplicate create refuses without overwrite, case-sensitive owned glyphs, bounded full document/refusal, Save As durable bytes, unsupported-file refusal and terminal real launch PASS')
if __name__=='__main__':main()
