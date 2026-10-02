#!/usr/bin/env python3
"""ADR-0053 controlled guest red test: disable ONLY receiver marker gate.

Prove the exact focused marker/no-mutation guest test goes RED against a
locally built unsafe image, restore source in `finally`, rebuild the final
artifact, and reprove green. This is an intentionally UNSHIPPABLE test
image, never a milestone or second branch. Source and EFI digests must
return to their exact pre-control bytes even if the red boot fails.
"""
import hashlib
import shutil
import subprocess
import sys
from pathlib import Path
sys.path.insert(0,str(Path(__file__).resolve().parent))
import afs1, arena_env, mtest
import test_m84_stage as t

ROOT=Path(__file__).resolve().parent.parent
SOURCE=ROOT/'userspace/packaged/src/main.rs'
EFI=ROOT/'build/arena-boot.efi'
LABEL='m84-red-control'
NEEDLE=b'} else if op != PKG_OP_QUERY && !marked {'
MUTANT=b'} else if op != PKG_OP_QUERY && !marked && false {'

def build():
    subprocess.run(['bash','tools/build.sh','--image'],cwd=ROOT,check=True,
                   capture_output=True,text=True)

def boot(label,esp,disk):
    rc,s,elapsed=mtest.boot(label,esp,[((b'packaged READY',b'arena>'),1,b'pkg stage-noauth app.test\r'),
                                       (b'arena>',2,b'shutdown\r')],disk)
    (arena_env.build_dir()/f'serial-{label}.log').write_text(s)
    return rc,s

def main():
    build()
    original=SOURCE.read_bytes()
    assert original.count(NEEDLE)==1 and MUTANT not in original
    esp=ROOT/'build/arena-esp.img'
    service=ROOT/'userspace/packaged/target/x86_64-unknown-none/release/arena-packaged'
    # Keep the exact original *known-clean* artifacts as well as the source.
    # Recompilation may change ELF build metadata/order without changing
    # semantics; the final artifact is restored byte-for-byte, not inferred.
    originals={p:p.read_bytes() for p in (EFI,esp,service)}
    efi_before=hashlib.sha256(originals[EFI]).digest()
    disk=arena_env.make_scratch_disk()
    rc,s,_=mtest.boot(LABEL+'-baseline',esp,[(b'arena>',1,b'shutdown\r')],disk)
    assert rc==0 and 'packaged READY' in s and not afs1.audit(disk)
    t.host_seed(disk,{t.INPUT:t.ROOT_FILE})
    clean=arena_env.build_dir()/'m84-red-control-clean.img'
    shutil.copyfile(disk,clean)
    red=False
    try:
        SOURCE.write_bytes(original.replace(NEEDLE,MUTANT))
        build()
        rc,s=boot(LABEL+'-mutant',esp,disk)
        red=(rc==0 and 'pkg: ELIGIBLE (staged, NOT installed/active)' in s
             and 'pkg: refused status -2' not in s
             and t.contents(disk).get(t.STAGE1)==t.ROOT_FILE
             and not afs1.audit(disk))
    finally:
        SOURCE.write_bytes(original)
        try:
            build()  # recompile the actual ordinary guard before restoring artifacts
        finally:
            # Preserve the previously verified exact image even if a build
            # fails. No red source or red EFI may escape this test.
            for path,bytes_ in originals.items():
                path.write_bytes(bytes_)
        assert SOURCE.read_bytes()==original, 'RED CONTROL LEFT SOURCE MODIFIED'
        assert hashlib.sha256(EFI.read_bytes()).digest()==efi_before, 'RED CONTROL LEFT EFI MODIFIED'
    rc,s=boot(LABEL+'-clean',esp,clean)
    green=(rc==0 and 'pkg: refused status -2' in s and t.STAGE1 not in t.contents(clean)
           and not afs1.audit(clean))
    print(f'[{LABEL}] {"PASS" if red else "FAIL"}: red mutant allows unauthorized real stage (gate detects violation)')
    print(f'[{LABEL}] {"PASS" if green else "FAIL"}: exact restored EFI refuses it, no stage; source/EFI hashes exact')
    return 0 if red and green else 1

if __name__=='__main__':sys.exit(main())
