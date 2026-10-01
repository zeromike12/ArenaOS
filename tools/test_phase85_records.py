#!/usr/bin/env python3
"""Cross-language AINS/AACT v1 byte vectors, not fsd/guest crash proof.

The Rust source is the production no_std record codec; the independent host
builder is deliberately separate and uses stdlib hashlib. Nothing here signs,
installs, registers an Image or establishes available AFS1 capacity.
"""
from __future__ import annotations
import hashlib
import pathlib
import subprocess
import tempfile

ROOT = pathlib.Path(__file__).resolve().parents[1]
MOD = ROOT / "userspace/installed.rs"
ID = b"echo.test" + bytes(23)
assert len(ID) == 32
D1 = bytes(range(1, 33))
D2 = bytes(range(33, 65))
D3 = bytes(range(65, 97))
ZERO = bytes(32)
H = lambda b: hashlib.sha256(b).digest()


def ins(gen: int, prev: bytes) -> bytes:
    b = bytearray(512)
    b[:4] = b"AINS"; b[4:6] = (1).to_bytes(2, 'little')
    b[6:8] = (512).to_bytes(2, 'little'); b[8:16] = gen.to_bytes(8, 'little')
    b[16:48] = ID; b[48:80] = D1; b[80:88] = (7).to_bytes(8, 'little')
    b[88:120] = D2; b[120:122] = bytes((gen, 3)); b[128:160] = prev
    b[480:] = H(b[:480]); return bytes(b)


def act(gen: int, select: bool, prev: bytes, ihash: bytes) -> bytes:
    b = bytearray(512)
    b[:4] = b"AACT"; b[4:6] = (1).to_bytes(2, 'little')
    b[6:8] = (512).to_bytes(2, 'little'); b[8:16] = gen.to_bytes(8, 'little')
    b[16:48] = ID
    if select:
        b[48:80] = ihash; b[80:112] = D1
        b[112:120] = (7).to_bytes(8, 'little')
    b[120] = 1 if select else 2; b[128:160] = prev
    b[480:] = H(b[:480]); return bytes(b)


RUST = r'''
#[path = "@MOD@"] mod installed;
use installed::*;
use std::io::Write;
use std::process::{Command,Stdio};
fn hash(b:&[u8])->Digest {
    let mut c=Command::new("openssl").args(["dgst","-sha256","-binary"])
        .stdin(Stdio::piped()).stdout(Stdio::piped()).spawn().unwrap();
    c.stdin.take().unwrap().write_all(b).unwrap();
    let x=c.wait_with_output().unwrap(); assert!(x.status.success());
    x.stdout.as_slice().try_into().unwrap()
}
fn hx(b:&[u8])->String { b.iter().map(|x|format!("{x:02x}")).collect() }
fn main() {
    let mut id=[0u8;32]; id[..9].copy_from_slice(b"echo.test");
    let d1=std::array::from_fn(|i|(i+1) as u8);
    let d2=std::array::from_fn(|i|(i+33) as u8);
    let mut out=[0xee;BYTES];
    let mut i=Installed{generation:1,id,full_digest:d1,version:7,
        payload_digest:d2,stage:1,observed_policy:3,previous:[0;32]};
    encode_installed(&i,&mut out,hash).unwrap();
    assert_eq!(parse_installed(&out,hash),Ok(i));
    assert_eq!(check_predecessor(&id,1,&i.previous,None,true,hash),Ok(()));
    let first=out;
    println!("I1 {}",hx(&first));
    i.generation=2; i.stage=2; i.previous=hash(&first);
    encode_installed(&i,&mut out,hash).unwrap();
    assert_eq!(parse_installed(&out,hash),Ok(i));
    assert_eq!(check_predecessor(&id,2,&i.previous,Some(&first),true,hash),Ok(()));
    println!("I2 {}",hx(&out));
    let second=out;
    let mut a=Activation{generation:1,id,installed_hash:hash(&second),
        full_digest:d1,version:7,select:true,previous:[0;32]};
    encode_activation(&a,&mut out,hash).unwrap();
    assert_eq!(parse_activation(&out,hash),Ok(a));
    let selection=out;
    println!("A1 {}",hx(&selection));
    a.generation=2; a.installed_hash=[0;32]; a.full_digest=[0;32];
    a.version=0; a.select=false; a.previous=hash(&selection);
    encode_activation(&a,&mut out,hash).unwrap();
    assert_eq!(parse_activation(&out,hash),Ok(a));
    assert_eq!(check_predecessor(&id,2,&a.previous,Some(&selection),false,hash),Ok(()));
    println!("A2 {}",hx(&out));
    a.generation=3; a.select=true; a.installed_hash=hash(&second);
    a.full_digest=d1; a.version=7; a.previous=hash(&out);
    encode_activation(&a,&mut out,hash).unwrap();
    assert_eq!(parse_activation(&out,hash),Ok(a));
    println!("A3 {}",hx(&out));
    a.generation=4; a.previous=hash(&out);
    encode_activation(&a,&mut out,hash).unwrap();
    assert_eq!(parse_activation(&out,hash),Ok(a)); // wire admits 4, storage may not
    println!("A4 {}",hx(&out));
    let pristine=out;
    for cut in [0,4,8,16,48,120,128,160,480,511] {
        let mut torn=pristine; torn[cut..].fill(0);
        assert!(parse_activation(&torn,hash).is_err(), "prefix {cut}");
    }
    for offset in [0,4,6,8,16,48,120,121,128,160,479,480,511] {
        let mut bad=pristine; bad[offset]^=1;
        assert!(parse_activation(&bad,hash).is_err(), "offset {offset}");
    }
    let mut bad=second; bad[122]=1;
    assert!(parse_installed(&bad,hash).is_err());
    let mut bad=selection; bad[121]=1;
    assert!(parse_activation(&bad,hash).is_err());
    let mut bad=first; bad[160]=1;
    assert!(parse_installed(&bad,hash).is_err());
    let mut unchanged=[0xee;BYTES];
    let mut invalid=i; invalid.generation=3;
    assert_eq!(encode_installed(&invalid,&mut unchanged,hash),Err(Error::Format));
    assert_eq!(unchanged,[0xee;BYTES]);
    invalid=i; invalid.previous=[0;32];
    assert_eq!(encode_installed(&invalid,&mut unchanged,hash),Err(Error::Chain));
    assert_eq!(unchanged,[0xee;BYTES]);
    let mut invalid_act=a; invalid_act.generation=5;
    assert_eq!(encode_activation(&invalid_act,&mut unchanged,hash),Err(Error::Format));
    assert_eq!(unchanged,[0xee;BYTES]);
    assert!(check_predecessor(&id,2,&i.previous,Some(&selection),true,hash).is_err());
    assert!(check_predecessor(&id,2,&i.previous,None,true,hash).is_err());
    assert!(check_predecessor(&id,1,&[0;32],Some(&first),true,hash).is_err());
}
'''


def main() -> None:
    with tempfile.TemporaryDirectory(prefix="phase85-records-") as d:
        src = pathlib.Path(d) / "test.rs"; exe = pathlib.Path(d) / "test"
        src.write_text(RUST.replace("@MOD@", str(MOD)))
        subprocess.run(["rustc", "--edition=2021", str(src), "-o", str(exe)], check=True)
        lines = subprocess.check_output([str(exe)], text=True).splitlines()
    i1 = ins(1, ZERO); i2 = ins(2, H(i1))
    a1 = act(1, True, ZERO, H(i2)); a2 = act(2, False, H(a1), ZERO)
    a3 = act(3, True, H(a2), H(i2)); a4 = act(4, True, H(a3), H(i2))
    expected = dict(zip(("I1", "I2", "A1", "A2", "A3", "A4"),
                        (i1, i2, a1, a2, a3, a4), strict=True))
    assert len(lines) == 6
    for line in lines:
        name, wire = line.split()
        assert bytes.fromhex(wire) == expected.pop(name)
    assert not expected
    print("Phase 8.5 AINS/AACT accepted wire: 6 independent SHA-256 vectors,"
          " 4-generation admission, prefix/corruption/chain/encode refusal PASS"
          " (host codec only; no AFS1 CREATE or guest proof)")


if __name__ == "__main__":
    main()
