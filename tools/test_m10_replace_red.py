#!/usr/bin/env python3
"""Production RED: reuse old data sector, crash observes torn old document."""
import hashlib
import subprocess
from pathlib import Path
import mtest
import arena_env
import test_m10_replace as green
ROOT=Path(__file__).resolve().parent.parent
SOURCE=ROOT/'userspace/fsd/src/main.rs'

def main():
    original=SOURCE.read_bytes();efi=ROOT/'build/arena-boot.efi'
    esp=mtest.build('m10-put-red-baseline');artifacts={p:p.read_bytes() for p in (efi,esp)};digest=hashlib.sha256(artifacts[efi]).hexdigest()
    mutant=original.replace(b'let Some(plan) = replace::reserve',b'let Some(mut plan) = replace::reserve',1)
    anchor=b'        begin_tx(fs);\n        for s in plan.sectors()'
    assert original.count(anchor)==1
    mutant=mutant.replace(anchor,b'        if old_count > 1 { plan.data[0] = old[1]; }\n'+anchor,1)
    assert mutant!=original
    red=False
    try:
        SOURCE.write_bytes(mutant)
        try:green.main()
        except AssertionError as error:
            observation=error.args[0] if error.args else None
            red=isinstance(observation,tuple) and len(observation)==2 and isinstance(observation[1],bytes) and observation[1] not in (green.OLD,green.NEW)
            print(f'[m10-put-red] real production in-place replacement rejected by independent crash bytes: {error}',flush=True)
    finally:
        SOURCE.write_bytes(original)
        try:mtest.build('m10-put-restored')
        finally:
            for path,data in artifacts.items():path.write_bytes(data)
    assert red,'in-place production mutation did not go RED'
    assert hashlib.sha256(efi.read_bytes()).hexdigest()==digest,'restored EFI differs'
    try:green.main(esp=esp)
    finally:
        for path,data in artifacts.items():path.write_bytes(data)
    assert hashlib.sha256(efi.read_bytes()).hexdigest()==digest
    assert SOURCE.read_bytes()==original
    print(f'[m10-put-red] byte-exact source/EFI restoration and complete guest GREEN sha256={digest}',flush=True)
if __name__=='__main__':main()
