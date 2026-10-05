#!/usr/bin/env python3
"""Fault-injected ordinary client exit: sibling pixels survive, exact cleanup."""
import hashlib
import re
from pathlib import Path
import arena_env
import mtest
from test_m10_apps import Desktop, receipts
from test_m10_desktop import crop
import test_m10_boundaries as green
ROOT=arena_env.REPO_ROOT;BUILD=arena_env.build_dir();LABEL='m10-client-death'
SOURCE=ROOT/'userspace/desktop/src/bin/application.rs'
NEEDLE=b'    fn key(&mut self, key: u16, client: &Client) -> Result<(), i64> {\n'
MUTANT=NEEDLE+b'        if self.kind == apps::GALLERY && key == 103 { client::exit(77) }\n        if self.kind == apps::GALLERY && key == 104 { loop { let _ = service::idle(None); } }\n'
def session_pages(d):
    shared,snapshot=map(int,re.search(r'session reservation shared/snapshot pages=(\d+)/(\d+)',d.serial()).groups())
    return shared+snapshot
def samples(d):return receipts(d.serial())
def workflow():
    d=Desktop(LABEL)
    try:
        base=samples(d)[0];empty=d.shot('empty');terminal=d.launch(0,'terminal')
        d.launch(5,'gallery',1);d.q.key('g')
        d.wait(lambda:d.serial().count('[desktop] application retired:')==1,'dead ordinary Gallery Process not retired')
        # One live session (ADR-0075): record, Process, shared + snapshot
        # regions, their 939 pages, three maps, two broker caps.
        d.wait(lambda:samples(d)[-1][1:]==(base[1]+1,base[2]+1,base[3]+2,base[4]+session_pages(d),base[5]+3,base[6]+2),'client death did not clean exact shared/cap/process state')
        d.shot('sibling-preserved',lambda p:crop(p,100,110,300,180)==crop(terminal,100,110,300,180))
        d.q.type_text('echo alive\r',gap_s=.04)
        d.shot('sibling-input',lambda p:crop(p,90,164,290,28)!=crop(terminal,90,164,290,28))
        d.close();d.wait(lambda:d.serial().count('[desktop] application retired:')==2 and samples(d)[-1][1:]==base[1:],'sibling cleanup failed')
        d.shot('clean-empty',lambda p:crop(p,100,110,300,180)==crop(empty,100,110,300,180))
        # A non-polling LIVE app fills its event queue. The second close must
        # still use the exact original Process, regardless of event delivery.
        d.launch(5,'unresponsive-gallery');d.q.key('h');d.q.type_text('a'*40,gap_s=.035)
        d.close();d.shot('first-close-pending');d.close()
        d.wait(lambda:d.serial().count('[desktop] application retired:')==3 and samples(d)[-1][1:]==base[1:],'full-queue live client could not be forcibly closed')
        assert base[0]-samples(d)[-1][0]==2*2,'client death exceeded two warmed private PT slots (two PTs each, ADR-0075)'
        return b'shutdown\r'
    finally:d.dispose()
def main():
    original=SOURCE.read_bytes();assert original.count(NEEDLE)==1
    esp=mtest.build(LABEL+'-base',desktop=True)
    artifacts={p:p.read_bytes() for p in (esp,BUILD/'arena-boot.efi',BUILD/'graphics-profile.txt')}
    try:
        SOURCE.write_bytes(original.replace(NEEDLE,MUTANT,1))
        mutant=mtest.build(LABEL+'-fault',desktop=True)
        rc,s,_=mtest.boot(LABEL,mutant,[((b'[desktop] real desktop frame presented',b'arena>'),1,workflow)],arena_env.make_scratch_disk(),pointer=True)
        assert rc==0 and '[arena ERROR halt]' not in s and s.count('[desktop] application retired:')==3
    finally:
        SOURCE.write_bytes(original)
        try:mtest.build(LABEL+'-restored',desktop=True)
        finally:
            for p,data in artifacts.items():p.write_bytes(data)
    assert SOURCE.read_bytes()==original and all(p.read_bytes()==data for p,data in artifacts.items())
    try:green.main(esp)
    finally:
        for p,data in artifacts.items():p.write_bytes(data)
    print('[m10-client-death] actual unexpected ordinary client exit cleans original Process/shared/map/cap state; sibling raster/input survives; full-queue LIVE client second close consumes exact Process; byte-exact source/EFI restored GREEN sha256='+hashlib.sha256(artifacts[BUILD/'arena-boot.efi']).hexdigest())
if __name__=='__main__':main()
