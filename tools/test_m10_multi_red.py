#!/usr/bin/env python3
"""Production multi-child quota + cancelled-reply RED, exact restoration GREEN."""
import hashlib
import subprocess
import sys
from pathlib import Path
import arena_env
import mtest
ROOT=Path(__file__).resolve().parent.parent
BUILD=arena_env.build_dir()
SPAWN=ROOT/'kernel/kernel/src/spawn.rs'
IPC=ROOT/'kernel/kernel/src/ipc.rs'

def fixture(name,existing=False):
    log=BUILD/f'{name}.log'
    command=[sys.executable,str(ROOT/'tools/test_m85_resources.py')]
    if existing:
        program='import sys;sys.path.insert(0,'+repr(str(ROOT/'tools'))+');import mtest;from pathlib import Path;mtest.build=lambda label:Path('+repr(str(BUILD/'arena-esp.img'))+');import test_m85_resources;test_m85_resources.main()'
        command=[sys.executable,'-c',program]
    with log.open('w') as stream:
        rc=subprocess.run(command,cwd=ROOT,stdout=stream,stderr=subprocess.STDOUT).returncode
    return rc,log.read_text()

def main():
    esp=mtest.build('m10-multi-red-base');efi=BUILD/'arena-boot.efi';artifacts={p:p.read_bytes() for p in (esp,efi)}
    sources={p:p.read_bytes() for p in (SPAWN,IPC)}
    controls=[(SPAWN,b'pub const MAX_DYNAMIC_CHILDREN: usize = 4;',b'pub const MAX_DYNAMIC_CHILDREN: usize = 1;','quota'),
              (IPC,b'                if checked {',b'                if checked && false {','cancel')]
    try:
        for path,before,after,label in controls:
            assert sources[path].count(before)==1
            path.write_bytes(sources[path].replace(before,after,1))
            try:
                rc,log=fixture(f'm10-multi-{label}-red')
                serial=(BUILD/'serial-m85-live-cutover-cutover.log').read_text()
                if label=='quota':
                    assert rc!=0 and 'SELECTTEST refused' in serial,(label,rc,log[-500:])
                else:
                    # The trigger is a signed child destroyed while packaged
                    # serves its call (the kernel leaves a Cancelled
                    # tombstone). Which of the fixture's ~33 such kills lands
                    # on a delivered call depends on scheduling, so the
                    # downstream symptom differs (SELECTTEST refusal, or the
                    # upgrade never completing); the mechanism does not:
                    # packaged's checked reply is refused instead of
                    # consuming the tombstone, packaged fails closed and the
                    # cutover does not complete.
                    assert rc!=0 and 'packaged: reply refused' in serial,(label,rc,log[-500:])
                    assert 'abandoned caller slot' in serial,(label,'no caller destroyed mid-call')
                print(f'[m10-multi-red] production {label} failed actual signed multi-child lifecycle PASS',flush=True)
            finally:path.write_bytes(sources[path])
        mtest.build('m10-multi-restored')
        for path,data in artifacts.items():path.write_bytes(data)
        rc,log=fixture('m10-multi-restored-green',existing=True);assert rc==0,log[-1800:]
    finally:
        for path,data in sources.items():path.write_bytes(data)
        for path,data in artifacts.items():path.write_bytes(data)
    assert all(p.read_bytes()==data for p,data in sources.items())
    assert all(p.read_bytes()==data for p,data in artifacts.items())
    print(f'[m10-multi-red] exact source/artifact restoration; signed full cutover/resource GREEN sha256={hashlib.sha256(artifacts[efi]).hexdigest()}',flush=True)
if __name__=='__main__':main()
