#!/usr/bin/env python3
"""ADR-0053 guest negative control: replace the EMBEDDED public test root.

An independently OpenSSL-signed test-root package must be refused by a
receiver built with a different canonical public root/fingerprint. No
private material enters either image. Restore original source/artifacts
byte-exact in finally; the mutant is test-only, never a checkpoint.
"""
import hashlib
import subprocess
import sys
from pathlib import Path
sys.path.insert(0,str(Path(__file__).resolve().parent))
import arena_env,mtest
import test_m84_stage as t
from test_package_record import SUB_PUB

ROOT=Path(__file__).resolve().parent.parent
SOURCE=ROOT/'userspace/package.rs'
EFI=ROOT/'build/arena-boot.efi'
ESP=ROOT/'build/arena-esp.img'
BIN=ROOT/'userspace/packaged/target/x86_64-unknown-none/release/arena-packaged'

def build():
    subprocess.run(['bash','tools/build.sh','--image'],cwd=ROOT,env=arena_env.rust_env() | {'ARENA_GRAPHICS_FIXTURE':'phase9'},check=True,
                   capture_output=True,text=True)

def boot(tag,disk):
    rc,s,secs=mtest.boot('m84-wrong-root-'+tag,ESP,
        [((b'packaged READY',b'arena>'),1,b'pkg stage app.test\r'),
         (b'arena>',2,b'shutdown\r')],disk)
    (arena_env.build_dir()/f'serial-m84-wrong-root-{tag}.log').write_text(s)
    return rc,s

def main():
    build()
    source=SOURCE.read_text()
    root='pub const ROOT: [u8; 32] = ['
    root_id='pub const ROOT_ID: [u8; 32] = ['
    assert source.count(root)==1 and source.count(root_id)==1
    def array(blob):
        return '\n'+''.join('    '+', '.join(f'0x{v:02x}' for v in blob[i:i+8])+',\n'
                              for i in range(0,32,8))+']'
    before_root=source.split(root,1)[1].split('];',1)[0]
    before_id=source.split(root_id,1)[1].split('];',1)[0]
    mutant=(source.replace(root+before_root+'];',root+array(SUB_PUB)+';')
                  .replace(root_id+before_id+'];',root_id+array(hashlib.sha256(SUB_PUB).digest())+';'))
    assert mutant!=source and 'pub const ROOT_ID' in mutant
    originals={p:p.read_bytes() for p in (EFI,ESP,BIN)}
    disk=arena_env.make_scratch_disk()
    rc,s,_=mtest.boot('m84-wrong-root-baseline',ESP,[(b'arena>',1,b'shutdown\r')],disk)
    assert rc==0 and 'packaged READY' in s
    t.host_seed(disk,{t.INPUT:t.ROOT_FILE})
    denied=False
    try:
        SOURCE.write_text(mutant)
        build()
        rc,s=boot('mutant',disk)
        denied=(rc==0 and 'pkg: refused status -2' in s
                and t.STAGE1 not in t.contents(disk))
    finally:
        SOURCE.write_text(source)
        try:build()
        finally:
            for path,bytes_ in originals.items():path.write_bytes(bytes_)
        assert SOURCE.read_text()==source
        assert all(p.read_bytes()==b for p,b in originals.items()),'wrong-root mutant artifact escaped'
    rc,s=boot('restored',disk)
    accepted=(rc==0 and 'ELIGIBLE (staged, NOT installed/active) version 7' in s
              and t.contents(disk).get(t.STAGE1)==t.ROOT_FILE)
    print(f'[m84-wrong-root] {"PASS" if denied else "FAIL"}: guest embedded wrong root refuses authentic test-root package')
    print(f'[m84-wrong-root] {"PASS" if accepted else "FAIL"}: restored public root accepts; source/EFI/ESP/ELF exact')
    return 0 if denied and accepted else 1
if __name__=='__main__':sys.exit(main())
