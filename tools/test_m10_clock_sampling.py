#!/usr/bin/env python3
"""Native clock controls: early return, one host stall, persistent overshoot.

No failed boot is retried or qualified. Each compiled mutation has its own
oracle; the final GREEN boots the saved original EFI and ESP byte for byte.
"""
import hashlib
import arena_env
import mtest

ROOT=arena_env.REPO_ROOT; BUILD=arena_env.build_dir()
M2=ROOT/'kernel/boot/src/m2.rs'
CLOCK=ROOT/'kernel/kernel/src/timekeeping.rs'
NEEDLE=b'        let end = timekeeping::now_us();'

def wait_body(source):
    start=source.index(b'pub fn busy_wait_us(us: u64) -> bool {')
    brace=source.index(b'{',start);end=brace+1;depth=1
    while depth:
        depth+=(source[end:end+1]==b'{')-(source[end:end+1]==b'}');end+=1
    return source[start:end]

def boot(label,esp,success,reason):
    rc,serial,_=mtest.boot(label,esp,[(b'arena>',1,b'shutdown\r')],arena_env.make_scratch_disk())
    (BUILD/(label+'.log')).write_text(serial)
    if success:
        assert rc==0 and 'm2: RESULT PASS' in serial and '[arena ERROR halt]' not in serial
    else:
        assert rc!=0 and reason in serial and '[arena ERROR halt]' in serial
    return serial

def main():
    esp=mtest.build('m10-clock-base')
    artifacts={p:p.read_bytes() for p in (esp,BUILD/'arena-boot.efi',BUILD/'graphics-profile.txt')}
    originals={p:p.read_bytes() for p in (M2,CLOCK)}
    body=wait_body(originals[CLOCK]);assert originals[M2].count(NEEDLE)==1
    controls=(
        (CLOCK,body,b'pub fn busy_wait_us(us: u64) -> bool { let _ = us; true }','early',False,'busy-wait returned before requested monotonic duration'),
        (M2,NEEDLE,b'        if trial == 0 { timekeeping::busy_wait_us(WAIT_US * 5); }\n'+NEEDLE,'one-stall',True,''),
        (M2,NEEDLE,b'        timekeeping::busy_wait_us(WAIT_US * 5);\n'+NEEDLE,'persistent',False,'busy-wait exceeded 4x request in all three windows'),
    )
    try:
        for path,before,after,label,success,reason in controls:
            path.write_bytes(originals[path].replace(before,after,1))
            try:
                mutant=mtest.build('m10-clock-'+label+'-mutant')
                serial=boot('m10-clock-'+label+'-red',mutant,success,reason)
                if label=='one-stall':assert 'clock_monotonic: window 0 overshot:' in serial
                if label=='persistent':assert serial.count('overshot:')==3
                print(f'[m10-clock] compiled {label}: native measurement oracle PASS',flush=True)
            finally:path.write_bytes(originals[path])
    finally:
        for p,data in originals.items():p.write_bytes(data)
        try:mtest.build('m10-clock-restored')
        finally:
            for p,data in artifacts.items():p.write_bytes(data)
    try:boot('m10-clock-green',esp,True,'')
    finally:
        for p,data in artifacts.items():p.write_bytes(data)
    assert all(p.read_bytes()==data for p,data in originals.items())
    assert all(p.read_bytes()==data for p,data in artifacts.items())
    print('[m10-clock] early-return and persistent-upper-bound production RED; one injected measurement stall accepted within three windows; exact source/EFI GREEN sha256='+hashlib.sha256(artifacts[BUILD/'arena-boot.efi']).hexdigest(),flush=True)

if __name__=='__main__':main()
