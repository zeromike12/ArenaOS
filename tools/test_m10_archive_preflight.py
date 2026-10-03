#!/usr/bin/env python3
"""Isolated standalone extraction-tool smoke; deliberately no 100/100 claim."""
import hashlib
import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path
import afs1
import arena_env
import mtest
from phase10_checkpoint_bundle import SCRIPTS

ROOT=arena_env.REPO_ROOT;BUILD=arena_env.build_dir()
def main():
    esp=mtest.build('m10-archive-preflight',desktop=True)
    with tempfile.TemporaryDirectory(prefix='arena-independent-preflight-') as directory:
        stage=Path(directory)
        shutil.copyfile(esp,stage/'arena-esp.img')
        shutil.copyfile(BUILD/'arena-boot.efi',stage/'arena-boot.efi')
        shutil.copyfile(arena_env.ovmf_code(),stage/'edk2-x86_64-code.fd')
        shutil.copyfile(arena_env.ovmf_vars_template(),stage/'ovmf-vars-template.img')
        afs1.mkfs(stage/'scratch-template.img',8*1024*1024//afs1.SECTOR)
        for script in SCRIPTS:shutil.copyfile(ROOT/'tools'/script,stage/script)
        (stage/'sha256sums.txt').write_text(''.join(
            f'{hashlib.sha256(p.read_bytes()).hexdigest()}  {p.name}\n' for p in sorted(stage.iterdir())))
        env=os.environ.copy();env.pop('PYTHONPATH',None)
        env['ARENA_EXTRACTED_EVIDENCE']=str(BUILD/'phase10-independent-preflight')
        runner='import runpy,sys;sys.path.insert(0,sys.argv.pop(1));runpy.run_path(sys.path[0]+"/phase10_archive_boot.py",run_name="__main__")'
        command=[sys.executable,'-I','-c',runner,str(stage)]
        # A staged unqualified image cannot pass the strict archival gate.
        strict=subprocess.run(command,cwd=stage,env=env,text=True,capture_output=True)
        assert strict.returncode!=0 and 'EXTRACTED PHASE10 BOOT FAILED:' in strict.stderr
        result=subprocess.run(command+['--unqualified-smoke'],cwd=stage,env=env,text=True,capture_output=True)
        (BUILD/'m10-archive-preflight.log').write_text(result.stdout+result.stderr)
        assert result.returncode==0 and 'UNQUALIFIED EXTRACTED PHASE10 PREFLIGHT PASS:' in result.stdout,result.stdout+result.stderr
        assert 'EXTRACTED PHASE10 PIXELS PASS:' not in result.stdout
    print('[m10-archive-preflight] independently staged standard-library tools, firmware, full shipping console/tablet/network, real two-spawn/retire graphical workflow PASS; strict qualification refuses absent 100/100 receipt')

if __name__=='__main__':main()
