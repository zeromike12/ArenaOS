#!/usr/bin/env python3
"""Coverage-guided Rust verifier fuzz using LLVM source-region feedback.

Runs an instrumented native build of the *exact vendored* no_std verifier in
one-shot host oracle processes. llvm-profdata/llvm-cov supply source-region
feedback for corpus growth; no libFuzzer crate or network dependency. Only
public test keys/signatures and offline host-signed test-only artifacts occur.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
import random
import struct
import subprocess
from pathlib import Path

import package_record as r
from test_package_record import TestRecord, RFC_SEED, SUB_PUB, openssl_sign

ROOT = Path(__file__).resolve().parent.parent
ROOTKEY = r.ROOT
RFC1 = bytes.fromhex('e5564300c360ac729086e2cc806e828a84877f1eb8e5d974d873e06522490155'
                     '5fb8821590a33bacc61e39701cf9b46bd25bf5f0595bbe24655141438e7a100b')
RFC2 = bytes.fromhex('92a009a9f0d4cab8720e820b5f642540a2b27b5416503f8fb3762223ebdb69da'
                     '085ac1e43e15996e458f3613d0f11d8c387b2eaeb4302aeeb00d291612bb0c00')
ORDER = bytes.fromhex('edd3f55c1a631258d69cf7a2def9de1400000000000000000000000000000010')
ALIAS = bytes.fromhex('f0' + 'ff'*30 + '7f')


def frame(mode: int, key: bytes, msg: bytes, signature: bytes, subordinate: bytes = b'') -> bytes:
    assert mode in (0, 1) and len(key) == 32 and len(signature) == 64
    assert (mode == 0 and not subordinate) or (mode == 1 and len(subordinate) == 32)
    assert len(msg) <= 5000
    return bytes([mode]) + key + struct.pack('<I', len(msg)) + msg + signature + subordinate


def fields(raw: bytes):
    mode = raw[0]
    size = struct.unpack_from('<I', raw, 33)[0]
    msg = raw[37:37 + size]
    sig = raw[37 + size:101 + size]
    sub = raw[101 + size:]
    assert len(sig) == 64 and len(sub) == (32 if mode else 0)
    return mode, raw[1:33], msg, sig, sub


def mutate(raw: bytes, rng: random.Random) -> bytes:
    mode, key, msg, sig, sub = fields(raw)
    key, msg, sig, sub = bytearray(key), bytearray(msg), bytearray(sig), bytearray(sub)
    kind = rng.randrange(12)
    if kind == 0:
        key[rng.randrange(32)] ^= 1 << rng.randrange(8)
    elif kind == 1:
        key[:] = rng.choice((bytes(32), bytes([0xff])*32, ALIAS, ROOTKEY, SUB_PUB))
    elif kind == 2:
        sig[rng.randrange(64)] ^= 1 << rng.randrange(8)
    elif kind == 3:
        sig[32:] = rng.choice((ORDER, bytes(32), bytes([0xff])*32))
    elif kind == 4:
        sig[:32] = bytes(32)
    elif kind == 5:
        if msg: msg[rng.randrange(len(msg))] ^= 1 << rng.randrange(8)
        else: msg.extend(b'\xff')
    elif kind == 6:
        if msg: msg.pop(rng.randrange(len(msg)))
        else: msg.extend(b'X')
    elif kind == 7:
        msg.insert(rng.randrange(len(msg) + 1), rng.randrange(256))
    elif kind == 8:
        msg.extend(b'\0')
    elif kind == 9:
        pos = rng.randrange(len(sig)); sig[pos] = rng.randrange(256)
    elif kind == 10 and mode == 1:
        sub[rng.randrange(32)] ^= 1 << rng.randrange(8)
    elif mode == 1:
        sub[:] = rng.choice((ALIAS, ROOTKEY, bytes(32), bytes([0xff])*32))
    else:
        key[0] ^= 1
    return frame(mode, bytes(key), bytes(msg), bytes(sig), bytes(sub))


def fixtures() -> list[bytes]:
    t = TestRecord(); t.setUp()
    alias_unsigned = r.signed_policy(t.id, 1, ALIAS, 7, 1, ())
    alias_sig = openssl_sign(RFC_SEED, r.POL_DOMAIN + alias_unsigned)
    return [
        frame(0, ROOTKEY, b'', RFC1),
        frame(0, SUB_PUB, b'\x72', RFC2),
        frame(0, ROOTKEY, r.parse_package(t.file).signed, t.file[-64:]),
        frame(0, SUB_PUB, r.parse_package(t.sub_file).signed, t.sub_file[-64:]),
        frame(1, ROOTKEY, r.parse_policy(t.policy).signed, t.policy[-64:], SUB_PUB),
        frame(1, ROOTKEY, r.POL_DOMAIN + alias_unsigned, alias_sig, ALIAS),
    ]


def coverage(binary: Path, profdata: Path, llvm_cov: Path) -> set[tuple[str, int, int]]:
    cp = subprocess.run([str(llvm_cov), 'export', str(binary), f'-instr-profile={profdata}',
                         '-format=text'], capture_output=True, check=True)
    data = json.loads(cp.stdout)['data'][0]['files']
    covered = set()
    for item in data:
        name = str(Path(item['filename']).resolve())
        if not name.startswith(str(ROOT / 'vendor/phase84')) and not name.startswith(str(ROOT / 'audit/phase84-crypto')):
            continue
        short = str(Path(name).relative_to(ROOT))
        for segment in item['segments']:
            line, col, count, has_count = segment[:4]
            if has_count and count > 0:
                covered.add((short, line, col))
    return covered


def run(binary: Path, llvm_profdata: Path, llvm_cov: Path, rounds: int, corpus_in: Path | None,
        corpus_out: Path | None, temp: Path) -> None:
    binary = binary.resolve(strict=True)
    seeds = fixtures()
    positives = set(seeds[:-1])
    pool = list(seeds)
    if corpus_in is not None:
        for p in sorted(corpus_in.glob('*.bin')):
            raw = p.read_bytes()
            if hashlib.sha256(raw).hexdigest()[:24] != p.stem:
                raise AssertionError(f'{p}: corpus filename checksum mismatch')
            fields(raw)
            if raw not in pool:pool.append(raw)
    if corpus_out is not None:
        corpus_out.mkdir(parents=True, exist_ok=True)
    seen = set()
    checked = set()
    rng = random.Random(0x84_53_8032)
    temp.mkdir(parents=True, exist_ok=True)
    rawprof, indexed = temp / 'fuzz.profraw', temp / 'fuzz.profdata'
    hits = 0
    for i in range(rounds):
        sample = pool[i] if i < len(pool) else mutate(rng.choice(pool if rng.randrange(4) else seeds), rng)
        rawprof.unlink(missing_ok=True)
        cp = subprocess.run([str(binary)], input=sample, capture_output=True,
                            env=dict(os.environ, LLVM_PROFILE_FILE=str(rawprof.resolve())), check=True)
        if cp.stdout != bytes([int(sample in positives)]):
            raise AssertionError(f'Rust verifier mismatch at iteration {i}; frame SHA256 {hashlib.sha256(sample).hexdigest()}')
        hits += int(cp.stdout == b'\x01')
        subprocess.run([str(llvm_profdata), 'merge', '-sparse', str(rawprof), '-o', str(indexed)],
                       capture_output=True, check=True)
        current = coverage(binary, indexed, llvm_cov)
        if any('/signing.rs' in name and name.startswith('vendor/phase84/ed25519-dalek-') for name, _, _ in current):
            raise AssertionError('signing source executed in the verifier probe')
        if current - seen and sample not in checked and len(pool) < 256:
            pool.append(sample)
        seen.update(current)
        checked.add(sample)
    if corpus_out is not None:
        for sample in pool:
            (corpus_out / (hashlib.sha256(sample).hexdigest()[:24] + '.bin')).write_bytes(sample)
    review = {name for name, _, _ in seen}
    print(f'vendored Rust LLVM coverage: cases={rounds}, unique_source_regions={len(seen)}, '
          f'corpus={len(set(pool))}, positive_cases={hits}, signing_source_regions=0')
    print('covered crate groups:', ', '.join(sorted({n.split('/')[2].split('-0.')[0] for n in review if n.startswith('vendor/phase84/')})))
    if len(seen) < 100 or hits < 5 or len(pool) < 10:
        raise AssertionError('Rust fuzz campaign failed minimum coverage/seed checks')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    sysroot = Path(subprocess.check_output(['rustc', '--print', 'sysroot'], text=True).strip())
    llvm = sysroot / 'lib/rustlib/x86_64-unknown-linux-gnu/bin'
    parser.add_argument('--binary', type=Path, default=ROOT / 'build/phase84-coverage-target/x86_64-unknown-linux-gnu/debug/verify_wire')
    parser.add_argument('--llvm-profdata', type=Path, default=llvm / 'llvm-profdata')
    parser.add_argument('--llvm-cov', type=Path, default=llvm / 'llvm-cov')
    parser.add_argument('--rounds', type=int, default=300)
    parser.add_argument('--corpus-in', type=Path)
    parser.add_argument('--corpus-out', type=Path)
    parser.add_argument('--temp', type=Path, default=ROOT / 'build/phase84-rust-fuzz-profile')
    a = parser.parse_args()
    if not 50 <= a.rounds <= 10000: parser.error('rounds outside [50,10000]')
    run(a.binary, a.llvm_profdata, a.llvm_cov, a.rounds, a.corpus_in, a.corpus_out, a.temp)
