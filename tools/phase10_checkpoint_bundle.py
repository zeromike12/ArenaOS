#!/usr/bin/env python3
"""Create and independently graphically boot the qualified Sol handoff archive."""
import argparse
import hashlib
import json
import os
import re
import shutil
import subprocess
import sys
import tarfile
import tempfile
from pathlib import Path
import afs1
import arena_env
from pyfatfs.PyFatFS import PyFatFS

ROOT=arena_env.REPO_ROOT; BUILD=arena_env.build_dir()
NAME='phase10-engineering-baseline'
SCRIPTS=('phase10_archive_boot.py','check_phase10_pixels.py','qmp.py','network_fixture.py','tcp_fixture.py','udp_dns_fixture.py')
DOCS=('ENGINEERING.md','UI-CAPABILITIES.md','APP-CONTRACTS.md','DESIGN-HANDOFF.md','DESIGN-REQUESTS.md','SCREENSHOT-MANIFEST.md','MANUAL-SMOKE.md')
def sha(path):return hashlib.sha256(path.read_bytes()).hexdigest()

def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('suite_log',type=Path)
    parser.add_argument('stability_log',type=Path)
    parser.add_argument('--source',help='qualified source commit, before documentation/artifact preservation')
    args=parser.parse_args()
    commit=subprocess.check_output(['git','rev-parse',args.source or 'HEAD'],cwd=ROOT,text=True).strip()
    assert not subprocess.check_output(['git','status','--porcelain'],cwd=ROOT),'freeze and preserve source before qualification'
    changed=subprocess.check_output(['git','diff','--name-only',commit,'HEAD'],cwd=ROOT,text=True).splitlines()
    assert all(p.startswith(('docs/phase10/','releases/checkpoints/'+NAME+'/')) for p in changed),'production/test/tools differ from qualified source'
    log=args.suite_log.read_text();count=len(list((ROOT/'tools').glob('test_m*.py')))+11
    required=(f'ALL TESTS PASSED ({count} test suites)',f'QUALIFICATION SOURCE COMMIT: {commit}','QUALIFICATION SOURCE CLEAN: yes',f'QUALIFICATION SOURCE END: {commit}',
              '[m10-boundaries-red] 10 real production RED controls;',
              '[m10-multi-red] exact source/artifact restoration;',
              '[m10-put-red] byte-exact source/EFI restoration and complete guest GREEN',
              '[m10-client-death] actual unexpected ordinary client exit',
              '[m10-service-death] real compositor death answers in-flight input CALL',
              '[m10-display] independently booted GOP800 and actual GPU800/GPU640 shipping desktop modes PASS',
              '[m10-handoff] actual desktop, all six real apps, full working set, durable dark appearance; independent-boot byte-identical light/dark owned gallery QMP rasters PASS',
              'transient-broker-caps=29')
    assert all(marker in log for marker in required),'missing complete source-bound historical/Phase-10 evidence'
    assert (BUILD/'graphics-profile.txt').read_text().strip()=='desktop'
    efi=sha(BUILD/'arena-boot.efi')
    assert (BUILD/'stability-receipt.txt').read_text().split()==[efi,'100/100']
    fs=PyFatFS(str(BUILD/'arena-esp.img'),read_only=True)
    try:assert fs.getbytes('/EFI/BOOT/BOOTX64.EFI')==(BUILD/'arena-boot.efi').read_bytes()
    finally:fs.close()
    stability=args.stability_log.read_text()
    assert efi in stability and '100/100' in stability and 'FAIL' not in stability,'invalid stability attempt'
    manifest=json.loads((BUILD/'phase10-screenshots/manifest.json').read_text())
    assert manifest['source_commit']==commit and manifest['efi_sha256']==efi,'stale screenshot fixture'
    assert len(manifest['entries'])>=12
    for entry in manifest['entries']:
        assert Path(entry['path']).name==entry['path'] and sha(BUILD/'phase10-screenshots'/entry['path'])==entry['sha256']
    destination=ROOT/'releases/checkpoints'/NAME;destination.mkdir(parents=True,exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='phase10-bundle-',dir=BUILD) as tmp:
        stage=Path(tmp)
        for filename in ('arena-boot.efi','arena-esp.img','stability-receipt.txt'):
            shutil.copyfile(BUILD/filename,stage/filename)
        for script in SCRIPTS:shutil.copyfile(ROOT/'tools'/script,stage/script)
        for doc in DOCS:shutil.copyfile(ROOT/'docs/phase10'/doc,stage/doc)
        shutil.copyfile(arena_env.ovmf_code(),stage/'edk2-x86_64-code.fd')
        shutil.copyfile(arena_env.ovmf_vars_template(),stage/'ovmf-vars-template.img')
        afs1.mkfs(stage/'scratch-template.img',8*1024*1024//afs1.SECTOR)
        shutil.copyfile(args.suite_log,stage/'full-suite.log')
        shutil.copyfile(args.stability_log,stage/'100-boot.log')
        shutil.copyfile(BUILD/'phase10-screenshots/manifest.json',stage/'screenshot-manifest.json')
        for entry in manifest['entries']:shutil.copyfile(BUILD/'phase10-screenshots'/entry['path'],stage/entry['path'])
        for proof in sorted(BUILD.glob('m10-*-red.log')):shutil.copyfile(proof,stage/('evidence-'+proof.name))
        peaks=re.findall(r'\[m10-(?:apps|display)\].*(?:baseline=|six-app peak=).*',log)
        (stage/'QUALIFICATION.json').write_text(json.dumps(dict(checkpoint=NAME,source_commit=commit,branch='arena/phase10-sol-engineering',suite_count=count,efi_sha256=efi,stability='100/100',resource_receipts=peaks,scope='Sol engineering/reference skin; Opus visual design remains pending'),indent=2)+'\n')
        (stage/'RUNNING.md').write_text('''# ArenaOS Phase-10 Sol engineering baseline

Verify the outer archive checksum and extracted `sha256sums.txt`, then install
QEMU and Python 3 and run `python3 phase10_archive_boot.py` in this directory.
The standalone script requires no repository or build tree. It copies fresh
OVMF variables and the bundled AFS1 platter, binds the included virtual-network
test peers, launches real graphical applications, injects keyboard/tablet input,
checks exact owned raster movement and Process/resource retirement, relaunches,
then uses the independent serial shell for clean shutdown. Test peers do not
claim public DNS. Set ARENA_QEMU to a shell-tokenized portable QEMU invocation
if it is not on PATH; ARENA_EXTRACTED_EVIDENCE selects an evidence output folder.

For an interactive session use the QEMU device/firmware arguments in
phase10_archive_boot.py with a visible display and fresh writable disk copies.
The historical boot fixture requires virtual keyboard input `arena` before
desktop startup. The serial shell and graphical terminal are independent.
See MANUAL-SMOKE.md and DESIGN-HANDOFF.md for interaction and editing contracts.
''')
        files=sorted(p.name for p in stage.iterdir())
        (stage/'sha256sums.txt').write_text(''.join(f'{sha(stage/name)}  {name}\n' for name in files))
        files.append('sha256sums.txt')
        archive=destination/f'arenaos-{NAME}-qemu-x86_64.tar.gz'
        with tarfile.open(archive,'w:gz') as tar:
            for name in files:tar.add(stage/name,arcname=name)
        digest=sha(archive)
        (destination/(archive.name+'.sha256')).write_text(f'{digest}  {archive.name}\n')
        # Extract only generated, flat regular files. Run only extracted tools
        # and firmware, with no repository on PYTHONPATH or source imports.
        with tempfile.TemporaryDirectory(prefix='phase10-independent-',dir=BUILD) as extracted:
            target=Path(extracted)
            with tarfile.open(archive,'r:gz') as tar:
                assert {m.name for m in tar}==set(files) and all(m.isfile() for m in tar)
                for member in tar:
                    stream=tar.extractfile(member);assert stream is not None
                    (target/member.name).write_bytes(stream.read())
            env=os.environ.copy();env.pop('PYTHONPATH',None)
            env['ARENA_EXTRACTED_EVIDENCE']=str(destination/'extracted-evidence')
            # Isolated Python plus exactly the extracted directory; no checkout
            # imports, installed site packages, PYTHONPATH or signing material.
            result=subprocess.run([sys.executable,'-I','-c','import runpy,sys;sys.path.insert(0,sys.argv[1]);runpy.run_path(sys.argv[1]+"/phase10_archive_boot.py",run_name="__main__")',str(target)],cwd=target,env=env,text=True,capture_output=True)
            (destination/'independent-extracted-boot.log').write_text(result.stdout+result.stderr)
            assert result.returncode==0 and 'EXTRACTED PHASE10 PIXELS PASS:' in result.stdout,result.stdout+result.stderr
        print(f'{NAME}: source={commit} suites={count}/{count} EFI={efi} stability=100/100 archive={digest}; independently extracted graphical boot PASS')

if __name__=='__main__':main()
