#!/usr/bin/env python3
"""Independently extract and graphically verify this frozen engineering bundle."""
from pathlib import Path
import hashlib,os,subprocess,sys,tarfile,tempfile

root=Path(__file__).resolve().parent
archive=root/'arenaos-phase10-engineering-baseline-qemu-x86_64.tar.gz'
expected=(root/(archive.name+'.sha256')).read_text().split()[0]
assert hashlib.sha256(archive.read_bytes()).hexdigest()==expected
with tempfile.TemporaryDirectory(prefix='arena-independent-') as directory:
    target=Path(directory)
    with tarfile.open(archive,'r:gz') as tar:
        members=tar.getmembers()
        assert all(m.isfile() and Path(m.name).name==m.name for m in members)
        assert len({m.name for m in members})==len(members)
        for member in members:
            stream=tar.extractfile(member);assert stream is not None
            (target/member.name).write_bytes(stream.read())
    for line in (target/'sha256sums.txt').read_text().splitlines():
        digest,name=line.split()
        assert Path(name).name==name and hashlib.sha256((target/name).read_bytes()).hexdigest()==digest
    env=os.environ.copy();env.pop('PYTHONPATH',None)
    env['ARENA_EXTRACTED_EVIDENCE']=str(root/'verification-rerun')
    # Consume the directory argument before runpy sets the script's argv[0].
    # No checkout modules or installed site packages enter isolated Python.
    runner='import runpy,sys;sys.path.insert(0,sys.argv.pop(1));runpy.run_path(sys.path[0]+"/phase10_archive_boot.py",run_name="__main__")'
    result=subprocess.run([sys.executable,'-I','-c',runner,str(target)],cwd=target,env=env,text=True,capture_output=True)
    print(result.stdout+result.stderr,end='')
    assert result.returncode==0 and 'EXTRACTED PHASE10 PIXELS PASS:' in result.stdout
print('INDEPENDENT ARCHIVE GRAPHICAL GREEN SHA256:',expected)
