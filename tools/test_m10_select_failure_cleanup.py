#!/usr/bin/env python3
"""Bad signed-child receipt still retires the original Process authority.

Actual 648-byte signed ELF, 32-object AFS platter and kernel resource counts.
Omitting production FINISH must leak; the repaired refusal returns all counts.
"""
import re
import subprocess
import sys
import hashlib
import arena_env
import mtest
import afs1
import package_record as rec
import test_m84_stage as stage
import test_m85_select as select
from test_package_record import RFC_SEED,openssl_sign

ROOT=arena_env.REPO_ROOT;BUILD=arena_env.build_dir()
SOURCE=ROOT/'userspace/servicemgr/src/package.rs'
RECEIPT=b'                    || bits != MGR_BADGE_PKG_PROBE_EXIT'
FINISH=b'                let later_finished = finish(later);'

def refused_fixture(esp,label):
    elf=(ROOT/'tools/phase85-elf-probe/target/x86_64-unknown-none/release/arena-phase85-elf-probe').read_bytes()
    assert len(elf)==648
    unsigned=rec.signed_package(stage.ID,7,elf,rec.ROOT)
    signed=unsigned+openssl_sign(RFC_SEED,rec.PKG_DOMAIN+unsigned)
    disk=arena_env.make_scratch_disk()
    rc,_,_=mtest.boot(label+'-seed',esp,[(b'arena>',1,b'shutdown\r')],disk);assert rc==0
    fillers={f'hist{i:02d}'.encode():bytes([i+1]) for i in range(23)}
    stage.host_seed(disk,{stage.INPUT:signed,stage.INTENT:stage.POLICY,**fillers})
    feed=[((b'arena>',b'packaged READY'),1,b'pkg stage app.test\r'),
          (b'arena>',2,b'pkg policy app.test\r'),(b'arena>',3,b'pkg installtest\r'),
          ((b'arena>',b'servicemgr: Phase 8.5 signed app.test INSTALL committed'),1,b'pkg resources\r'),
          (b'pkg: observed frames=',1,b'pkg selecttest\r'),
          (b'servicemgr: SELECTTEST refused: lifecycle receipt or Image invariant',1,b'pkg resources\r'),
          (b'pkg: observed frames=',2,b'shutdown\r')]
    rc,serial,_=mtest.boot(label,esp,feed,disk,timeout_s=120)
    (BUILD/(label+'.log')).write_text(serial)
    assert rc==0 and 'select later receipt bits=' in serial and '[arena ERROR halt]' not in serial
    assert all(stage.contents(disk)[k]==v for k,v in fillers.items()) and not afs1.audit(disk)
    counts=[tuple(map(int,row)) for row in re.findall(r'pkg: observed frames=(\d+) records=(\d+) processes=(\d+)',serial)]
    assert len(counts)==2,counts
    return counts

def main():
    subprocess.run([sys.executable,str(ROOT/'tools/test_phase85_elf_fit.py')],cwd=ROOT,check=True)
    original=SOURCE.read_bytes();assert original.count(RECEIPT)==original.count(FINISH)==1
    esp=mtest.build('m10-select-cleanup-base')
    artifacts={p:p.read_bytes() for p in (esp,BUILD/'arena-boot.efi',BUILD/'graphics-profile.txt')}
    forced=original.replace(RECEIPT,b'                    || true // TEST: force bad later receipt',1)
    try:
        SOURCE.write_bytes(forced.replace(FINISH,b'                let later_finished = if false { finish(later) } else { Ok(()) };',1))
        mutant=mtest.build('m10-select-cleanup-red')
        red=refused_fixture(mutant,'m10-select-cleanup-red')
        assert red[1][0]<red[0][0] and red[1][1:]==(red[0][1]+1,red[0][2]+1),red
        print(f'[m10-select-cleanup] omitted original Process FINISH leaks native resources RED PASS {red}',flush=True)
        SOURCE.write_bytes(forced)
        fixed=mtest.build('m10-select-cleanup-forced-refusal')
        green=refused_fixture(fixed,'m10-select-cleanup-forced-refusal')
        assert green[0]==green[1],green
        print(f'[m10-select-cleanup] same forced receipt refusal retires exact original Process GREEN PASS {green}',flush=True)
    finally:
        SOURCE.write_bytes(original)
        try:mtest.build('m10-select-cleanup-restored')
        finally:
            for p,data in artifacts.items():p.write_bytes(data)
    try:
        select.main(esp)
    finally:
        for p,data in artifacts.items():p.write_bytes(data)
    assert SOURCE.read_bytes()==original and all(p.read_bytes()==data for p,data in artifacts.items())
    print('[m10-select-cleanup] bad receipt native cleanup RED/GREEN; exact original source/EFI historical signed selection GREEN sha256='+hashlib.sha256(artifacts[BUILD/'arena-boot.efi']).hexdigest(),flush=True)

if __name__=='__main__':main()
