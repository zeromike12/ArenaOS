#!/usr/bin/env python3
"""Coverage-guided, reproducible host artifact fuzz against Python decoder + vendored Rust verifier.

No extra packages, network, or guest secrets. The fixed public RFC 8032 test seed
is used only by the host test fixture generator in test_package_record.py.
Rebuild the host Rust oracle before invocation (see ADR-0053 source audit).
Coverage feedback is Python decoder line *transitions*, not Rust machine-code
coverage. This distinction is intentional and explicitly reported.
"""
from __future__ import annotations

import argparse
import hashlib
import random
import struct
import subprocess
import sys
from pathlib import Path

import package_record as record
from test_package_record import TestRecord, SUB_PUB, RFC_SEED, openssl_sign, openssl_verify

ROOT = Path(__file__).resolve().parent.parent
PY_TARGET = Path(record.__file__).resolve()


def traced_parse(parser, data: bytes):
    transitions = set()
    last = {}
    def trace(frame, event, arg):
        if Path(frame.f_code.co_filename).resolve() != PY_TARGET:
            return None
        fid = id(frame)
        if event == 'call':
            last[fid] = 0
        elif event == 'line':
            here = (frame.f_code.co_name, last.get(fid, 0), frame.f_lineno)
            transitions.add(here)
            last[fid] = frame.f_lineno
        elif event in ('return', 'exception'):
            if event == 'return':
                last.pop(fid, None)
        return trace
    sys.settrace(trace)
    try:
        try:
            return parser(data), transitions
        except record.Refusal:
            return None, transitions
    finally:
        sys.settrace(None)


def mutate(data: bytes, corpus: list[bytes], rng: random.Random) -> bytes:
    b = bytearray(data)
    offset = rng.randrange(len(b)) if b else 0
    kind = rng.randrange(9)
    if kind == 0:
        b[offset] ^= 1 << rng.randrange(8)
    elif kind == 1:
        b[offset] = rng.randrange(256)
    elif kind == 2:
        b = b[:offset]
    elif kind == 3:
        b.insert(offset, rng.randrange(256))
    elif kind == 4:
        b.pop(offset)
    elif kind == 5:
        b += bytes([rng.randrange(256)])
    elif kind == 6:
        other = rng.choice(corpus)
        b[offset:offset + rng.randrange(1, 13)] = other[offset:offset + rng.randrange(1, 13)]
    elif kind == 7:
        # Boundary-heavy header and signature fields; no signing.
        pos = rng.choice((4, 6, 8, 10, 12, 44, 48, 52, 80, 112, 120, 121, 127, 128, len(b)-1))
        if 0 <= pos < len(b):
            b[pos] = rng.choice((0, 1, 2, 4, 8, 0x7f, 0x80, 0xff))
    else:
        pos = rng.randrange(max(1, len(b) - 8))
        b[pos:pos+8] = rng.choice((bytes(8), b'\xff'*8, b'\x01'+bytes(7)))
    return bytes(b)


def oracle_verify(proc: subprocess.Popen, key: bytes, signed: bytes, sig: bytes,
                  subordinate: bytes | None = None) -> bool:
    assert len(key) == 32 and len(sig) == 64 and len(signed) <= 5000
    assert subordinate is None or len(subordinate) == 32
    proc.stdin.write(bytes([int(subordinate is not None)]) + key +
                     struct.pack('<I', len(signed)) + signed + sig + (subordinate or b''))
    proc.stdin.flush()
    result = proc.stdout.read(1)
    if result not in (b'\x00', b'\x01'):
        raise RuntimeError('vendored verifier oracle exited unexpectedly')
    return result == b'\x01'


def run(oracle: Path, iterations: int, corpus_out: Path | None, corpus_in: Path | None) -> None:
    fixture = TestRecord()
    fixture.setUp() # host-only OpenSSL signing with RFC 8032 published test seed
    digest_list = tuple(sorted(record.digest(bytes([n])) for n in range(8)))
    unsigned = record.signed_policy(fixture.id, 2, SUB_PUB, 8, 2, digest_list)
    full_policy = unsigned + openssl_sign(RFC_SEED, record.POL_DOMAIN + unsigned)
    # Real ZIP-215 noncanonical, non-small-order key that dalek decompresses;
    # root signs it, so signature-valid alone MUST NOT admit this policy.
    alias = bytes.fromhex('f0' + 'ff'*30 + '7f')
    alias_unsigned = record.signed_policy(fixture.id, 1, alias, 7, 1, ())
    alias_policy = alias_unsigned + openssl_sign(RFC_SEED, record.POL_DOMAIN + alias_unsigned)
    seeds = [(fixture.file, record.ROOT), (fixture.sub_file, SUB_PUB),
             (fixture.policy, record.ROOT), (full_policy, record.ROOT),
             (alias_policy, record.ROOT)]
    assert record.parse_package(fixture.file).full_digest == record.digest(fixture.file)
    rng = random.Random(0x0053_84ed)
    if corpus_out is not None:
        corpus_out.mkdir(parents=True, exist_ok=True)
    oracle = oracle.resolve(strict=True)
    with subprocess.Popen([str(oracle)], stdin=subprocess.PIPE, stdout=subprocess.PIPE) as proc:
        for kind, parser in [('package', record.parse_package), ('policy', record.parse_policy)]:
            cases = [(data, key) for data, key in seeds if (data[:4] == (b'APKG' if kind == 'package' else b'APOL'))]
            valid = {data for data, _ in cases if data != alias_policy}
            corpus = [data for data, _ in cases]
            if corpus_in is not None:
                for file in sorted(corpus_in.glob(f'{kind}-*.bin')):
                    data = file.read_bytes()
                    if hashlib.sha256(data).hexdigest()[:24] != file.stem.split('-', 1)[1]:
                        raise AssertionError(f'{file}: corpus checksum mismatch')
                    if data not in corpus:
                        corpus.append(data)
                if len(corpus) < 4:
                    raise AssertionError(f'{kind}: replay corpus absent/incomplete')
            seen_cases = set(corpus)
            seen_edges = set()
            accepted_negative = rejected = oracle_calls = openssl_calls = 0
            for i in range(iterations):
                data = rng.choice(corpus if rng.randrange(5) else [x for x, _ in cases])
                candidate = corpus[i] if corpus_in is not None and i < len(corpus) else (data if i < len(cases) else mutate(data, corpus, rng))
                parsed, edges = traced_parse(parser, candidate)
                fresh = edges - seen_edges
                seen_edges.update(edges)
                if fresh and candidate not in seen_cases and len(corpus) < 256:
                    corpus.append(candidate)
                    seen_cases.add(candidate)
                if parsed is None:
                    rejected += 1
                    continue
                # The policy's root signs policy bytes. A package signer must
                # match an actually known key ID, never just a guessed filename.
                key = record.ROOT if kind == 'policy' else (
                    record.ROOT if parsed.signer_id == record.digest(record.ROOT) else
                    SUB_PUB if parsed.signer_id == record.digest(SUB_PUB) else None)
                if key is None:
                    accepted_negative += 1
                    continue
                verdict = oracle_verify(proc, key, parsed.signed, parsed.signature,
                                        parsed.subordinate if kind == 'policy' else None)
                oracle_calls += 1
                if verdict != (candidate in valid):
                    raise AssertionError(f'{kind} iteration {i}: unexpected crypto accept/refusal; sha256={hashlib.sha256(candidate).hexdigest()}')
                if candidate not in valid:
                    accepted_negative += 1
                if oracle_calls % 73 == 0:
                    independent = openssl_verify(key, parsed.signed, parsed.signature)
                    openssl_calls += 1
                    # OpenSSL proves the alias policy is root-signature-valid;
                    # it does not certify the subordinate's canonical bytes.
                    if candidate == alias_policy:
                        if not independent or verdict:
                            raise AssertionError('noncanonical root-signed policy bypassed key guard')
                    elif independent != verdict:
                        raise AssertionError(f'{kind} iteration {i}: OpenSSL / vendored verifier disagreement')
            if corpus_out is not None:
                for candidate in corpus:
                    name = hashlib.sha256(candidate).hexdigest()[:24]
                    (corpus_out / f'{kind}-{name}.bin').write_bytes(candidate)
            print(f'{kind}: iterations={iterations}, edges={len(seen_edges)}, corpus={len(corpus)}, '
                  f'parser_refusals={rejected}, accepted_negative={accepted_negative}, '
                  f'vendored_verifier_calls={oracle_calls}, independent_openssl_checks={openssl_calls}')
            if len(seen_edges) < 20 or accepted_negative < 100 or oracle_calls < 100:
                raise AssertionError(f'{kind}: fuzz campaign missed expected paths')
        proc.stdin.close()
        if proc.wait(timeout=10) != 0:
            raise RuntimeError('vendored verifier oracle failed')


if __name__ == '__main__':
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--oracle', type=Path, default=ROOT / 'audit/phase84-crypto/target/x86_64-unknown-linux-gnu/release/verify_wire')
    p.add_argument('--iterations', type=int, default=10000)
    p.add_argument('--corpus-out', type=Path)
    p.add_argument('--corpus-in', type=Path)
    args = p.parse_args()
    if not 100 <= args.iterations <= 1000000:
        p.error('iterations outside [100, 1000000]')
    run(args.oracle, args.iterations, args.corpus_out, args.corpus_in)
