#!/usr/bin/env python3
"""Production IPC reply-cap hook omission RED, original exact EFI GREEN.

Temporarily removes the actual `image_registry::add_cap` in production
`ipc::reply`, proving the independent stable-boundary capspace/queue walk
halts on a real signed Image reply. Restores byte-exact source and EFI in
finally; this test is NOT a substitute for all queue/death paths.
"""
import hashlib
import subprocess
import sys
from pathlib import Path
import arena_env

ROOT=Path(__file__).resolve().parent.parent
SOURCE=ROOT/'kernel/kernel/src/ipc.rs'
EFI=ROOT/'build/arena-boot.efi'
ESP=ROOT/'build/arena-esp.img'
# Phase 9 added an independent SharedRegion credit between the existing
# Image credit and the reply slot. RED must omit only the Image hook, not
# both kinds of authority or the actual reply staging.
NEEDLE=b'            crate::image_registry::add_cap(staged);\n            crate::shared::add_cap(staged);\n            slot.reply_cap = staged;'
MUTANT=b'            // RED ONLY: intentionally omitted production Image reply-cap credit.\n            crate::shared::add_cap(staged);\n            slot.reply_cap = staged;'

def run_fixture(log):
    with log.open('w') as f:
        proc=subprocess.run([sys.executable,str(ROOT/'tools/test_m85_select.py')],
            cwd=ROOT,stdout=f,stderr=subprocess.STDOUT)
    serial=(arena_env.build_dir()/'serial-m85-select-select.log').read_text(errors='replace')
    (arena_env.build_dir()/f'{log.stem}-serial.log').write_text(serial)
    return proc.returncode,serial

def main():
    original=SOURCE.read_bytes()
    assert original.count(NEEDLE)==1 and MUTANT not in original
    subprocess.run(['bash','tools/build.sh','--image'],cwd=ROOT,check=True,
        stdout=(arena_env.build_dir()/'m85-ref-hook-build.log').open('w'))
    originals={p:p.read_bytes() for p in (EFI,ESP)}
    source_hash=hashlib.sha256(original).digest()
    red=False
    try:
        SOURCE.write_bytes(original.replace(NEEDLE,MUTANT))
        rc,s=run_fixture(arena_env.build_dir()/'m85-ref-hook-red.log')
        red=(rc!=0 and s.count('[arena ERROR halt] halting machine: Image reference/pin conservation failure')==1
             and 'servicemgr: Phase 8.5 signed SELECT/LAUNCH/DEACTIVATE/reselect' not in s)
    finally:
        SOURCE.write_bytes(original)
        # Build the unmodified production hook before restoring the exact
        # source-bound image, even when the red test did not behave as hoped.
        try:
            subprocess.run(['bash','tools/build.sh','--image'],cwd=ROOT,check=True,
                stdout=(arena_env.build_dir()/'m85-ref-hook-restored-build.log').open('w'))
        finally:
            for path, data in originals.items(): path.write_bytes(data)
        assert SOURCE.read_bytes()==original and hashlib.sha256(SOURCE.read_bytes()).digest()==source_hash
        assert all(path.read_bytes()==data for path,data in originals.items()), 'mutant EFI escaped'
    try:
        green_rc,green_s=run_fixture(arena_env.build_dir()/'m85-ref-hook-green.log')
        green=(green_rc==0 and 'servicemgr: Phase 8.5 signed SELECT/LAUNCH/DEACTIVATE/reselect' in green_s
               and '[arena ERROR halt]' not in green_s)
    finally:
        # The guest helper rebuilds its ESP (FAT metadata may vary); present
        # the exact pre-control EFI + ESP again, not a guessed equivalent.
        for path,data in originals.items(): path.write_bytes(data)
    assert all(path.read_bytes()==data for path,data in originals.items()), 'control left an image modified'
    print(f'[m85-ref-hook] RED omitted real IPC reply credit: {"PASS" if red else "FAIL"}; '
          f'GREEN restored exact source/EFI and guest: {"PASS" if green else "FAIL"}',flush=True)
    return 0 if red and green else 1

if __name__=='__main__':sys.exit(main())
