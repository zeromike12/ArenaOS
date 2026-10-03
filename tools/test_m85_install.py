#!/usr/bin/env python3
"""ADR-0054 narrow guest INSTALL + exact repeat: signed APKG, real fsd AINS.

Not SELECT, Image registration, activation, runnable ELF or 8.5 qualification.
The M5 historical fixture MUST first create arena.txt on an empty platter;
seeding public signed inputs before that boot violates its one-file LS proof.
"""
import hashlib
import shutil
from pathlib import Path
import arena_env
import mtest
import afs1
import test_m84_stage as t

LABEL='m85-install'
AINS=('n8-'+t.PREFIX+'-01').encode()

def boot(esp, disk, tag, feed):
    rc, serial, elapsed=mtest.boot(f'{LABEL}-{tag}',esp,feed,disk)
    print(f'[{LABEL}] {tag}: rc={rc}, seconds={elapsed:.1f}',flush=True)
    (arena_env.build_dir()/f'serial-{LABEL}-{tag}-proof.log').write_text(serial)
    return rc,serial

def clean(serial,rc):
    return rc==0 and 'm7: RESULT PASS (2/2)' in serial and 'PANIC' not in serial \
           and '[arena ERROR halt]' not in serial \
           and 'halting via UEFI ResetSystem(shutdown)' in serial

def main():
    esp=mtest.build(LABEL)
    disk=arena_env.make_scratch_disk()
    rc,s=boot(esp,disk,'historical-baseline',[(b'arena>',1,b'shutdown\r')])
    assert clean(s,rc) and list(t.contents(disk))==[b'arena.txt']
    t.host_seed(disk,{t.INPUT:t.ROOT_FILE,t.INTENT:t.POLICY})
    seed=t.contents(disk)
    assert AINS not in seed and t.STAGE1 not in seed
    feed=[((b'arena>',b'packaged READY'),1,b'pkg stage app.test\r'),
          (b'arena>',2,b'pkg policy app.test\r'),
          (b'arena>',3,b'pkg install-wrong app.test\r'),
          (b'arena>',4,b'pkg install-noauth app.test\r'),
          (b'arena>',5,b'pkg installtest\r'),
          (b'servicemgr: Phase 8.5 signed app.test INSTALL committed',1,b'shutdown\r')]
    rc,s=boot(esp,disk,'install',feed)
    assert clean(s,rc) and 'pkg: fixed signed INSTALL fixture requested' in s
    assert s.count('pkg: refused status -2')==2, 'wrong STAGE marker and absent marker must deny INSTALL'
    assert 'servicemgr: Phase 8.5 signed app.test INSTALL committed and exact replay idempotent; no Image cap' in s
    records=t.contents(disk)
    assert set(records)-set(seed)=={t.STAGE1,t.POLICY1,AINS},set(records)-set(seed)
    wire=records[AINS];assert len(wire)==512 and wire[:4]==b'AINS'
    assert wire[4:6]==(1).to_bytes(2,'little') and wire[6:8]==(512).to_bytes(2,'little')
    assert wire[8:16]==(1).to_bytes(8,'little') and wire[16:48]==t.ID
    assert wire[48:80]==hashlib.sha256(t.ROOT_FILE).digest()
    assert wire[80:88]==(7).to_bytes(8,'little') and wire[88:120]==t.ROOT_FILE[56:88]
    assert wire[120:122]==b'\x01\x01' and wire[122:480]==bytes(358)
    assert wire[480:]==hashlib.sha256(wire[:480]).digest()
    assert not afs1.audit(disk)
    print(f'[{LABEL}] exact 512-byte guest-committed AINS hash={hashlib.sha256(wire).hexdigest()}; independently checked',flush=True)
    before=disk.read_bytes()
    rc,s=boot(esp,disk,'reboot-repeat',[(b'arena>',1,b'pkg installtest\r'),
                   (b'servicemgr: Phase 8.5 signed app.test INSTALL committed',1,b'shutdown\r')])
    assert clean(s,rc) and disk.read_bytes()==before
    print(f'[{LABEL}] same-platter reboot exact repeat: byte-exact NO new CREATE',flush=True)
    corrupt=arena_env.build_dir()/f'{LABEL}-corrupt.img'
    shutil.copyfile(disk,corrupt)
    t.host_seed(corrupt,{}) # retains a valid AFS1 commit; corrupt only data below.
    # Rewrite the committed record's data sector without touching the commit.
    d=afs1.Disk(corrupt.read_bytes());_,table,_=d.commit()
    obj=next(o for o in d.objects(table) if o['type']==afs1.OBJ_FILE and o['name']==AINS)
    sector=d.extents(obj['extent_head'])[0][0]
    raw=bytearray(corrupt.read_bytes());raw[sector*512+160]^=1;corrupt.write_bytes(raw)
    rc,s=boot(esp,corrupt,'corrupt-newest',[((b'arena>',b'packaged: boot namespace scan refused'),1,b'shutdown\r')])
    assert clean(s,rc) and 'packaged: boot namespace scan refused: corrupt/offline, no READY' in s
    assert 'servicemgr: packaged READY' not in s and corrupt.read_bytes()==raw
    print(f'[{LABEL}] visible corrupt newest refuses READY; no fallback or disk mutation PASS',flush=True)

if __name__=='__main__':main()
