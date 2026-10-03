#!/usr/bin/env python3
"""Design-only 8.5 measurement: useful signed APKG v1 ELF fits 4096 bytes.

Builds the EXISTING 8.4 kernel only to make production elf.rs's embedded
include_bytes available to the host adapter. Never registers a disk image,
changes a kernel syscall, runs an installer or claims that the probe has
executed in the guest. The private RFC *test* seed lives only in OpenSSL's
host TemporaryDirectory via the existing Phase 8.4 test helper.
"""
import hashlib
import os
import shutil
import subprocess
import sys
from pathlib import Path

ROOT=Path(__file__).resolve().parent.parent
FIX=ROOT/'tools/phase85-elf-probe'
sys.path.insert(0,str(ROOT/'tools'))
import package_record as record
from test_package_record import RFC_SEED, openssl_sign, openssl_verify

def run(*cmd,**kwargs):
    return subprocess.run(cmd,check=True,cwd=kwargs.get('cwd',ROOT),
                          env=kwargs.get('env'),capture_output=True,text=True)

def main(existing_userspace=False):
    env=os.environ.copy()
    rust=Path('/opt/rust/prefix/bin')
    if rust.is_dir():env['PATH']=str(rust)+os.pathsep+env['PATH']
    assert shutil.which('cargo',path=env['PATH']) and shutil.which('rustc',path=env['PATH']), 'install Rust 1.97 + bare-metal target'
    # Compile unmodified production elf.rs in validate.rs. Its embedded
    # include_bytes entries need the normal existing userspace build.
    if not existing_userspace:
        run('bash','tools/build.sh','--image',env=env)
    run('cargo','build','--offline','--locked','--release','--target','x86_64-unknown-none',cwd=FIX,env=env)
    name='arena-phase85-elf-probe'
    image=(FIX/'target/x86_64-unknown-none/release'/name).read_bytes()
    # Fresh independent target directory, not merely an incremental compile.
    clean_target=ROOT/'build/phase85-independent-target'
    env2={**env,'CARGO_TARGET_DIR':str(clean_target)}
    run('cargo','build','--offline','--locked','--release','--target','x86_64-unknown-none',cwd=FIX,env=env2)
    independent=(clean_target/'x86_64-unknown-none/release'/name).read_bytes()
    assert image==independent, 'offline fresh build differs byte-for-byte'
    adapter=ROOT/'build/phase85-production-elf-validator'
    run('rustc','--edition=2024',str(FIX/'validate.rs'),'-o',str(adapter),env=env)
    verdict=run(str(adapter),str(FIX/'target/x86_64-unknown-none/release'/name),env=env).stdout.strip()
    assert len(image)==648 and len(image)<=4096, f'probe ELF changed: {len(image)}'
    unsigned=record.signed_package(record.make_id('app.test'),7,image,record.ROOT)
    signed=unsigned+openssl_sign(RFC_SEED,record.PKG_DOMAIN+unsigned)
    parsed=record.parse_package(signed)
    assert parsed.payload==image and parsed.version==7
    assert openssl_verify(record.ROOT,parsed.signed,parsed.signature)
    tampered=bytearray(unsigned)
    tampered[128] ^= 1  # signed ELF's first byte, not incidental EOF padding
    assert not openssl_verify(record.ROOT,record.PKG_DOMAIN+tampered,parsed.signature)
    assert len(signed)==192+len(image)<=4288
    print(verdict)
    print(f'APKG v1: independently OpenSSL-signed/test-root-verified file={len(signed)} bytes, '
          f'payload={len(image)}; payload_sha256={hashlib.sha256(image).hexdigest()}; '
          f'full_file_sha256={hashlib.sha256(signed).hexdigest()}')
    print('fresh offline build byte-identical; host-only measurement PASS; dynamic guest execution NOT proven')

if __name__=='__main__':
    assert sys.argv[1:] in ([],['--existing-userspace'])
    main(existing_userspace=bool(sys.argv[1:]))
