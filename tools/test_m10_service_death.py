#!/usr/bin/env python3
"""Actual Phase-10 compositor death while inputd is in CALL, restored GREEN."""
import hashlib
import re
from pathlib import Path
import arena_env
import mtest
import qmp
import test_m10_boundaries as green
ROOT=arena_env.REPO_ROOT;BUILD=arena_env.build_dir();LABEL='m10-service-death'
SOURCE=ROOT/'userspace/desktop/src/bin/desktop.rs'
# Phase 11.4: key frames carry press/release and modifiers (key_input).
# Phase 11.8: the key arm also routes desktop-surface keys; same frame.
NEEDLE=(b'                                let a = app_action\n'
        b'                                    .unwrap_or_else(|| state.key_input(code, pressed, mods));')
MUTANT=(b'                                if code == 113 && pressed { die(77) }\n'
        b'                                let a = app_action\n'
        b'                                    .unwrap_or_else(|| state.key_input(code, pressed, mods));')
def main():
    original=SOURCE.read_bytes();assert original.count(NEEDLE)==1
    esp=mtest.build(LABEL+'-base',desktop=True)
    artifacts={p:p.read_bytes() for p in (esp,BUILD/'arena-boot.efi',BUILD/'graphics-profile.txt')}
    def killed():
        conn=qmp.Qmp(str(BUILD/f'qmp-{LABEL}.sock'))
        try:conn.key('q')
        finally:conn.close()
        return b''
    try:
        SOURCE.write_bytes(original.replace(NEEDLE,MUTANT,1))
        mutant=mtest.build(LABEL+'-mutant',desktop=True)
        rc,s,_=mtest.boot(LABEL,mutant,[((b'[desktop] real desktop frame presented',b'arena>'),1,killed)],arena_env.make_scratch_disk(),pointer=True)
        # QEMU returns zero for the guest's requested UEFI shutdown. The
        # fatal marker and manager-death reason are the lifecycle evidence.
        assert '[arena ERROR halt]' in s and 'desktop: trusted application manager died; fail-stop to retire all session authority' in s
        assert re.search(r'destroy pid \d+: 1 in-flight call\(s\) answered STATUS_SERVICE_GONE',s),s[-2000:]
        assert s.count('[desktop] real desktop frame presented')==1
    finally:
        SOURCE.write_bytes(original)
        try:mtest.build(LABEL+'-restored',desktop=True)
        finally:
            for p,data in artifacts.items():p.write_bytes(data)
    assert SOURCE.read_bytes()==original and all(p.read_bytes()==data for p,data in artifacts.items())
    try:green.main(esp)
    finally:
        for p,data in artifacts.items():p.write_bytes(data)
    print('[m10-service-death] real compositor death answers in-flight input CALL and root fail-stops without restart; exact source/EFI restored graphical GREEN sha256='+hashlib.sha256(artifacts[BUILD/'arena-boot.efi']).hexdigest())
if __name__=='__main__':main()
