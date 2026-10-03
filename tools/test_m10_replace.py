#!/usr/bin/env python3
"""Real AFS1 named replacement: persistence + crashes, independent disk bytes."""
import shutil
import struct
from pathlib import Path
import afs1
import arena_env
import mtest
ROOT=Path(__file__).resolve().parent.parent
NAME=b'user-note'
OLD=b'original exact document'
NEW=b'new replacement with different size and bytes'

def read(path):
    d=afs1.Disk(path.read_bytes());_,ot,_=d.commit();o=d.find(NAME,ot)
    if o is None:return None
    return b''.join(d.data[s*512:(s+n)*512] for s,n in d.extents(o['extent_head']))[:o['size']]

def boot(label,esp,disk,commands,kill=None):
    feed=[(b'arena>',i+1,c+b'\r') for i,c in enumerate(commands)]
    return mtest.boot(label,esp,feed,disk,kill=kill)

def main(esp=None):
    if esp is None:esp=mtest.build('m10-replace')
    seed=arena_env.make_scratch_disk()
    rc,s,_=boot('m10-replace-seed',esp,seed,[b'put '+NAME+b' '+OLD,b'shutdown'])
    assert rc==0 and 'put committed' in s and read(seed)==OLD
    base=arena_env.build_dir()/'m10-replace-seed.img';shutil.copyfile(seed,base)
    rounds=[(b'fsd: PUT data written',1,0),(b'storaged: WRITE',3,0),
            (b'storaged: WRITE',7,0),(b'put committed',1,0)]
    for i,kill in enumerate(rounds):
        disk=arena_env.build_dir()/f'm10-replace-crash-{i}.img';shutil.copyfile(base,disk)
        rc,s,_=boot(f'm10-replace-crash-{i}',esp,disk,[b'put '+NAME+b' '+NEW],kill)
        assert rc is None,f'crash control missed {i}'
        actual=read(disk);assert actual in (OLD,NEW),(i,actual)
        if i==3:assert actual==NEW
        assert afs1.audit(disk)==[]
        rc,s,_=boot(f'm10-replace-recover-{i}',esp,disk,[b'cat '+NAME,b'shutdown'])
        assert rc==0 and actual.decode() in s and read(disk)==actual
        print(f'[m10-replace] crash {i}: exact old-or-new {len(actual)} bytes, independent bitmap/extent audit and remount PASS',flush=True)
    disk=arena_env.build_dir()/'m10-replace-full.img';shutil.copyfile(base,disk)
    rc,s,_=boot('m10-replace-full',esp,disk,[b'put '+NAME+b' '+NEW,b'put '+NAME,b'shutdown'])
    assert rc==0 and read(disk)==b'' and afs1.audit(disk)==[]

    # Both valid records: previous NEW must remain intact after publishing EMPTY.
    d=afs1.Disk(disk.read_bytes());generations=[]
    for sector in (1,2):
        rec=d.sector(sector);assert afs1.fnv1a64(rec[:504])==struct.unpack_from('<Q',rec,504)[0]
        seq=struct.unpack_from('<Q',rec,8)[0];ot=struct.unpack_from('<I',rec,24)[0]
        o=d.find(NAME,ot);assert o is not None
        data=b''.join(d.data[a*512:(a+n)*512] for a,n in d.extents(o['extent_head']))[:o['size']]
        generations.append((seq,data))
    assert [data for _,data in sorted(generations)]==[NEW,b''],generations
    # A full bitmap refuses before touching either commit, old data, or bitmap.
    full=arena_env.build_dir()/'m10-replace-refusal.img';shutil.copyfile(base,full)
    raw=bytearray(full.read_bytes());_,_,bm=afs1.Disk(raw).commit();raw[bm*512:(bm+4)*512]=b'\xff'*2048;full.write_bytes(raw)
    before=full.read_bytes()
    rc,s,_=boot('m10-replace-refusal',esp,full,[b'put '+NAME+b' '+NEW,b'shutdown'])
    assert rc==0 and 'put committed' not in s and 'put:' in s
    assert full.read_bytes()==before and read(full)==OLD
    print('[m10-replace] complete create, replace, shrink-to-empty, two valid generations, mutation-free full refusal and guest crash-prefix recovery PASS')
if __name__=='__main__':main()
