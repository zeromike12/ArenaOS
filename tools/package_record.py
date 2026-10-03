#!/usr/bin/env python3
"""Host-only, stdlib byte reference for proposed ADR-0053 v1. Not a guest verifier.

Crypto signing/verification in the test harness uses independent OpenSSL. This
module performs format and policy checks; it never grants install authority.
"""
from __future__ import annotations

import hashlib
import struct
from dataclasses import dataclass

PKG_DOMAIN = b"ArenaOS.pkg.v1\x00"
POL_DOMAIN = b"ArenaOS.policy.v1\x00"
ROOT = bytes.fromhex("d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a")
assert len(PKG_DOMAIN) == 15 and len(POL_DOMAIN) == 18 and len(ROOT) == 32


class Refusal(ValueError):
    pass


def digest(data: bytes) -> bytes:
    return hashlib.sha256(data).digest()


def identity(raw: bytes) -> bytes:
    if len(raw) != 32 or b"\0" not in raw:
        raise Refusal("id width/terminator")
    prefix, pad = raw.split(b"\0", 1)
    if not 1 <= len(prefix) <= 31 or any(pad):
        raise Refusal("noncanonical id")
    if prefix[0] not in b"abcdefghijklmnopqrstuvwxyz0123456789":
        raise Refusal("id start")
    if any(c not in b"abcdefghijklmnopqrstuvwxyz0123456789.-" for c in prefix):
        raise Refusal("id alphabet")
    return raw


def make_id(name: str) -> bytes:
    raw = name.encode("ascii")
    return identity(raw + b"\0" * (32 - len(raw)))


def signed_package(package_id: bytes, version: int, payload: bytes, signer: bytes) -> bytes:
    identity(package_id)
    if not 1 <= version < 1 << 64 or not 1 <= len(payload) <= 4096 or len(signer) != 32:
        raise Refusal("package bounds")
    return b"".join((b"APKG", struct.pack("<HHHH", 1, 128, 1, 0), package_id,
                     struct.pack("<QI", version, len(payload)), digest(payload),
                     digest(signer), bytes(8), payload))


@dataclass(frozen=True)
class Package:
    package_id: bytes
    version: int
    signer_id: bytes
    payload: bytes
    signature: bytes
    signed: bytes
    full_digest: bytes


def parse_package(file: bytes) -> Package:
    if len(file) < 193 or len(file) > 4288 or file[:4] != b"APKG":
        raise Refusal("package envelope")
    if struct.unpack_from("<HHHH", file, 4) != (1, 128, 1, 0) or any(file[120:128]):
        raise Refusal("package header")
    package_id = identity(file[12:44])
    version, n = struct.unpack_from("<QI", file, 44)
    if version == 0 or not 1 <= n <= 4096 or len(file) != 192 + n:
        raise Refusal("package length/version")
    payload = file[128:128+n]
    if file[56:88] != digest(payload):
        raise Refusal("payload digest")
    return Package(package_id, version, file[88:120], payload,
                   file[128+n:], PKG_DOMAIN + file[:128+n], digest(file))


def signed_policy(package_id: bytes, generation: int, subordinate: bytes,
                  minimum: int, state: int, revoked: tuple[bytes, ...]) -> bytes:
    identity(package_id)
    if not 1 <= generation <= 4 or len(subordinate) != 32 or subordinate == ROOT:
        raise Refusal("policy key/gen")
    if not 1 <= minimum < 1 << 64 or state not in (1, 2) or len(revoked) > 8:
        raise Refusal("policy bounds")
    if any(len(d) != 32 or not any(d) for d in revoked) or list(revoked) != sorted(set(revoked)):
        raise Refusal("policy revocations")
    return b"".join((b"APOL", struct.pack("<HHQ", 1, 448, generation), package_id,
                     subordinate, digest(subordinate), struct.pack("<QBB", minimum, state, len(revoked)),
                     bytes(6), b"".join(revoked) + bytes(32 * (8 - len(revoked))), bytes(64)))


@dataclass(frozen=True)
class Policy:
    package_id: bytes
    generation: int
    subordinate: bytes
    minimum: int
    state: int
    revoked: frozenset[bytes]
    signed: bytes
    signature: bytes


def parse_policy(file: bytes) -> Policy:
    if len(file) != 512 or file[:4] != b"APOL":
        raise Refusal("policy envelope")
    if struct.unpack_from("<HH", file, 4) != (1, 448):
        raise Refusal("policy header")
    generation = struct.unpack_from("<Q", file, 8)[0]
    package_id = identity(file[16:48])
    subordinate = file[48:80]
    count = file[121]
    minimum = struct.unpack_from("<Q", file, 112)[0]
    if not 1 <= generation <= 4 or minimum == 0 or subordinate == ROOT or count > 8:
        raise Refusal("policy bounds")
    if digest(subordinate) != file[80:112] or file[120] not in (1, 2):
        raise Refusal("policy key/state")
    if any(file[122:128]) or any(file[128 + count*32:448]):
        raise Refusal("policy padding")
    rev = tuple(file[128+i*32:160+i*32] for i in range(count))
    if any(not any(x) for x in rev) or list(rev) != sorted(set(rev)):
        raise Refusal("policy sorting")
    return Policy(package_id, generation, subordinate, minimum, file[120],
                  frozenset(rev), POL_DOMAIN + file[:448], file[448:])


def chain(policies: list[Policy]) -> Policy | None:
    previous = None
    retired: set[bytes] = set()
    for index, p in enumerate(policies, 1):
        if p.generation != index or (previous is not None and
            (p.package_id != previous.package_id or p.minimum < previous.minimum or
             not previous.revoked <= p.revoked)):
            raise Refusal("policy history")
        if previous is not None and (previous.state == 2 or previous.subordinate != p.subordinate):
            retired.add(previous.subordinate)
        if p.state == 1 and p.subordinate in retired:
            raise Refusal("retired signer")
        previous = p
    return previous


def revoked_add(current: tuple[bytes, ...], new_digest: bytes) -> tuple[bytes, ...]:
    """Typed bounded refusal before attempting fsd CREATE."""
    if len(new_digest) != 32 or not any(new_digest):
        raise Refusal("invalid digest")
    result = tuple(sorted(set(current) | {new_digest}))
    if len(result) > 8:
        raise Refusal("NO_SPACE")
    return result
