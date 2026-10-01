#!/usr/bin/env python3
"""Controlled test-only route into unchanged production proc::destroy LIVE-ID guard.

No public Process cap for the manager is granted to the shell. Only this
build's diagnostic call-site and test accessor are temporary; both source
files, EFI and ESP are restored byte-exactly after the expected fail-stop.
"""
import hashlib,subprocess,sys
from pathlib import Path
import afs1,arena_env,mtest,package_record as rec
import test_m84_stage as t
from test_package_record import RFC_SEED,openssl_sign
ROOT=Path(__file__).resolve().parent.parent
SYSCALL=ROOT/'kernel/kernel/src/arch/x86_64/syscall.rs'
REG=ROOT/'kernel/kernel/src/image_registry.rs'
EFI=ROOT/'build/arena-boot.efi';ESP=ROOT/'build/arena-esp.img'
TAG='m85-manager-destroy'
NEEDLE=b'    manager_check_last_thread(); // before even recording the exit status\n'
REPLACE=b'''    // MUTANT ONLY: route this manager exit into unchanged production
    // proc::destroy PRE-teardown guard, before its ordinary death check.
    if let Some(pid) = crate::sched::current_proc_id() {
        if crate::image_registry::destroy_test_manager_pid() == pid
            && crate::image_registry::active() {
            let _ = crate::proc::destroy(pid);
        }
    }
    manager_check_last_thread(); // before even recording the exit status
'''
ANCHOR=b'pub fn registrar_alive() -> bool {\n'
ACCESSOR=b'''// MUTANT ONLY: read the exact kernel-owned manager ID for direct guard proof.
pub fn destroy_test_manager_pid() -> u64 {
    without_interrupts(|| unsafe { *MANAGER_PID.get() })
}
'''
def build(label):
    with (arena_env.build_dir()/f'{TAG}-{label}-build.log').open('w') as f:
        subprocess.run(['bash','tools/build.sh','--image'],cwd=ROOT,check=True,stdout=f,stderr=subprocess.STDOUT)
def main():
    original={p:p.read_bytes() for p in (SYSCALL,REG)}
    assert original[SYSCALL].count(NEEDLE)==1 and original[REG].count(ANCHOR)==1
    build('original');artifacts={p:p.read_bytes() for p in (EFI,ESP)}
    passed=False
    try:
        SYSCALL.write_bytes(original[SYSCALL].replace(NEEDLE,REPLACE))
        REG.write_bytes(original[REG].replace(ANCHOR,ACCESSOR+ANCHOR))
        build('direct-destroy')
        subprocess.run([sys.executable,str(ROOT/'tools/test_phase85_elf_fit.py')],cwd=ROOT,check=True,
                       stdout=(arena_env.build_dir()/f'{TAG}-fit.log').open('w'))
        elf=(ROOT/'tools/phase85-elf-probe/target/x86_64-unknown-none/release/arena-phase85-elf-probe').read_bytes()
        unsigned=rec.signed_package(t.ID,7,elf,rec.ROOT)
        signed=unsigned+openssl_sign(RFC_SEED,rec.PKG_DOMAIN+unsigned)
        disk=arena_env.make_scratch_disk()
        rc,s,_=mtest.boot(f'{TAG}-baseline',ESP,[(b'arena>',1,b'shutdown\r')],disk)
        assert rc==0 and list(t.contents(disk))==[b'arena.txt']
        t.host_seed(disk,{t.INPUT:signed,t.INTENT:t.POLICY})
        rc,s,_=mtest.boot(f'{TAG}-fatal',ESP,[
            ((b'arena>',b'packaged READY'),1,b'pkg stage app.test\r'),
            (b'arena>',2,b'pkg policy app.test\r'),
            (b'arena>',3,b'pkg installtest\r'),
            ((b'arena>',b'servicemgr: Phase 8.5 signed app.test INSTALL committed'),1,b'pkg deathtest\r')],disk)
        (arena_env.build_dir()/f'serial-{TAG}-fatal.log').write_text(s)
        items=t.contents(disk)
        passed=(rc==97 and s.count('[arena ERROR halt] halting machine: ADR-0055: manager death with dynamic authority/child')==1
                and 'servicemgr: deliberate manager last-thread exit with PREPARED LIVE ID' in s
                and ('n8-'+t.PREFIX+'-01').encode() in items
                and not any(k.startswith(b'v8-') for k in items)
                and not afs1.audit(disk))
    finally:
        for p,data in original.items():p.write_bytes(data)
        try:build('restored')
        finally:
            for p,data in artifacts.items():p.write_bytes(data)
        assert all(p.read_bytes()==data for p,data in original.items())
        assert all(p.read_bytes()==data for p,data in artifacts.items())
    assert passed,'direct production proc::destroy guard not proven'
    print(f'[{TAG}] direct production proc::destroy guard under LIVE manager ID HALT; exact source/EFI/ESP restored PASS',flush=True)
if __name__=='__main__':main()
