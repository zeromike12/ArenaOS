#!/usr/bin/env python3
"""Rerun real live cutover and independently parse guest-observed high-water.

The actual Power shell snapshots three kernel counters while the manager
holds the unretired LIVE dynamic child, then after Process-cap STOP/FINISH.
This is not a host source-based resource estimate.
"""
import re
from pathlib import Path
import arena_env
import test_m85_live_cutover as live

def main():
    live.main()
    s=(arena_env.build_dir()/'serial-m85-live-cutover-cutover.log').read_text()
    samples=[tuple(map(int,m)) for m in re.findall(r'pkg: observed frames=(\d+) records=(\d+) processes=(\d+)',s)]
    assert len(samples)==3 and samples[0]==samples[2] and samples[1][0]<samples[0][0]
    assert samples[1][1:]==(samples[0][1]+1,samples[0][2]+1)
    caps={k:int(v) for k,v in re.findall(r'servicemgr: observed cap occupancy ([\w-]+)=(\d+)',s)}
    assert all(0<caps[k]<=32 for k in ('baseline','two-live-images','unretired-child','after-finish'))
    assert caps['two-live-images']>caps['unretired-child']>caps['baseline']
    assert caps['cycle-pre']==caps['cycle-post']==caps['after-finish'],caps
    initial=[int(n) for n in re.findall(r'packaged: observed initial cap occupancy (\d+)',s)]
    peaks=[int(n) for n in re.findall(r'packaged: observed cap high-water (\d+)',s)]
    assert initial and peaks and max(peaks)>initial[0] and max(peaks)<=32
    assert 'servicemgr: full fixture notification budget 31/31; thirty-second refused' in s
    assert 'servicemgr: four dynamic children bounded while live and exited-unreaped; FINISH permits next spawn PASS' in s
    print(f'[m85-resources] guest Power snapshots baseline/live/retired={samples}; manager measured caps={caps}; packaged measured initial={initial[0]} peak={max(peaks)}; Notification=31/31 and thirty-second refused PASS',flush=True)
if __name__=='__main__':main()
