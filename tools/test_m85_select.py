#!/usr/bin/env python3
"""Narrow guest PREPARE/ABORT/COMMIT/LAUNCH and signed ELF child fixture.

Manager revokes old ID, checks the four-child capacity refusal, reaps the dynamic child and
revokes the LAUNCH ID. Not the full Phase-8.5 qualification.
"""
import hashlib
import shutil
import subprocess
import sys
from pathlib import Path
import arena_env, mtest, afs1, package_record as record
import test_m84_stage as t
from test_package_record import RFC_SEED, openssl_sign

ROOT=Path(__file__).resolve().parent.parent
LABEL='m85-select'

def boot(esp,disk,tag,feed):
    rc,s,sec=mtest.boot(f'{LABEL}-{tag}',esp,feed,disk)
    (arena_env.build_dir()/f'serial-{LABEL}-{tag}.log').write_text(s)
    print(f'[{LABEL}] {tag}: rc={rc} seconds={sec:.1f}',flush=True)
    assert rc==0 and 'm7: RESULT PASS (2/2)' in s and 'PANIC' not in s and '[arena ERROR halt]' not in s
    return s

def main():
    # Independently signed PUBLIC fixture only. No signing code or private key
    # enters the guest. A frozen host measurement checks this exact ELF subset.
    subprocess.run([sys.executable,str(ROOT/'tools/test_phase85_elf_fit.py')],check=True,
        stdout=(arena_env.build_dir()/'m85-select-elf-fit.log').open('w'))
    elf=(ROOT/'tools/phase85-elf-probe/target/x86_64-unknown-none/release/arena-phase85-elf-probe').read_bytes()
    assert len(elf)==648
    unsigned=record.signed_package(t.ID,7,elf,record.ROOT)
    signed=unsigned+openssl_sign(RFC_SEED,record.PKG_DOMAIN+unsigned)
    assert len(signed)==840
    esp=mtest.build(LABEL)
    disk=arena_env.make_scratch_disk()
    boot(esp,disk,'baseline',[(b'arena>',1,b'shutdown\r')])
    assert list(t.contents(disk))==[b'arena.txt']
    # 1 historical arena.txt + two signed inputs + 23 inert historical-slot
    # occupants + STAGE/POLICY/AINS + three AACT decisions = exactly 32.
    fillers={f'hist{i:02d}'.encode(): bytes([i+1]) for i in range(23)}
    t.host_seed(disk,{t.INPUT:signed,t.INTENT:t.POLICY,**fillers})
    assert len(t.contents(disk))==26
    s=boot(esp,disk,'select',[( (b'arena>',b'packaged READY'),1,b'pkg stage app.test\r'),
        (b'arena>',2,b'pkg policy app.test\r'),
        (b'arena>',3,b'pkg installtest\r'),
        ((b'arena>',b'servicemgr: Phase 8.5 signed app.test INSTALL committed'),1,b'pkg selecttest\r'),
        ((b'arena>',b'servicemgr: Phase 8.5 signed SELECT/LAUNCH/DEACTIVATE/reselect; ring-3 child reaped'),1,b'shutdown\r')])
    assert 'servicemgr: SELECTTEST refused' not in s
    assert 'phase85: bounded image queried signed stage via inherited endpoint' in s
    contents=t.contents(disk)
    acts=[('v8-'+t.PREFIX+f'-{i:02d}').encode() for i in (1,2,3)]
    assert len(contents)==32 and set(contents)-({b'arena.txt',t.INPUT,t.INTENT,t.STAGE1,t.POLICY1,('n8-'+t.PREFIX+'-01').encode()}|set(fillers))==set(acts)
    assert all(contents[name]==data for name,data in fillers.items())
    assert s.count('packaged: AACT CREATE submitted')==3, 'fourth must refuse before CREATE'
    wire=contents[acts[0]]
    assert len(wire)==512 and wire[:4]==b'AACT' and wire[8:16]==(1).to_bytes(8,'little')
    assert wire[16:48]==t.ID and wire[48:80]==hashlib.sha256(contents[('n8-'+t.PREFIX+'-01').encode()]).digest()
    assert wire[80:112]==hashlib.sha256(signed).digest() and wire[112:120]==(7).to_bytes(8,'little')
    assert wire[120]==1 and wire[121:480]==bytes(359) and wire[480:]==hashlib.sha256(wire[:480]).digest()
    disabled=contents[acts[1]]
    assert disabled[:4]==b'AACT' and disabled[8:16]==(2).to_bytes(8,'little')
    assert disabled[16:48]==t.ID and disabled[48:120]==bytes(72)
    assert disabled[120]==2 and disabled[121:128]==bytes(7)
    assert disabled[128:160]==hashlib.sha256(wire).digest()
    assert disabled[160:480]==bytes(320) and disabled[480:]==hashlib.sha256(disabled[:480]).digest()
    third=contents[acts[2]]
    assert third[:128]==wire[:8]+(3).to_bytes(8,'little')+wire[16:128]
    assert third[128:160]==hashlib.sha256(disabled).digest()
    assert third[160:480]==bytes(320) and third[480:]==hashlib.sha256(third[:480]).digest()
    assert not afs1.audit(disk)
    before=disk.read_bytes()
    s=boot(esp,disk,'reboot',[((b'arena>',b'packaged READY'),1,b'shutdown\r')])
    assert 'phase85: bounded image queried' not in s and disk.read_bytes()==before
    corrupt=arena_env.build_dir()/f'{LABEL}-corrupt.img'
    shutil.copyfile(disk,corrupt)
    d=afs1.Disk(corrupt.read_bytes());_,table,_=d.commit()
    obj=next(o for o in d.objects(table) if o['type']==afs1.OBJ_FILE and o['name']==acts[2])
    sector=d.extents(obj['extent_head'])[0][0]
    raw=bytearray(corrupt.read_bytes());raw[sector*512+200]^=1;corrupt.write_bytes(raw)
    s=boot(esp,corrupt,'corrupt',[((b'arena>',b'packaged: boot namespace scan refused'),1,b'shutdown\r')])
    assert 'packaged: boot namespace scan refused: corrupt/offline, no READY' in s
    assert 'servicemgr: packaged READY' not in s and corrupt.read_bytes()==raw
    print(f'[{LABEL}] signed ELF SELECT/DEACTIVATE/SELECT hashes={[hashlib.sha256(contents[name]).hexdigest() for name in acts]}; ring-3 IPC/reap, same-platter reboot no auto launch, corrupt newest refusal PASS',flush=True)

if __name__=='__main__':main()
