#!/usr/bin/env python3
"""ADR-0053 host-only independent format/crypto evidence (NOT guest implementation).

Requires only Python stdlib and OpenSSL 3's Ed25519 implementation. Seeds below
are publicly known RFC 8032 *test* seeds; generated private DER exists only in
a temporary host test directory, never a guest disk or deployable OS image.
"""
import hashlib
import os
import random
import subprocess
import tempfile
import unittest
from pathlib import Path

import package_record as r

RFC_SEED = bytes.fromhex("9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60")
SUB_SEED = bytes.fromhex("4ccd089b28ff96da9db6c346ec114e0f5b8a319f35aba624da8cf6ed4fb8a6fb")
SUB_PUB = bytes.fromhex("3d4017c3e843895a92b70aa74d1b7ebc9c982ccf2ec4968cc0cd55f12af4660c")
SPKI = bytes.fromhex("302a300506032b6570032100")
PKCS8 = bytes.fromhex("302e020100300506032b657004220420")


def openssl_sign(seed: bytes, message: bytes) -> bytes:
    with tempfile.TemporaryDirectory() as td:
        p = Path(td)
        (p / 'key.der').write_bytes(PKCS8 + seed)
        (p / 'msg').write_bytes(message)
        subprocess.run(['openssl', 'pkeyutl', '-sign', '-rawin', '-keyform', 'DER', '-inkey', str(p/'key.der'),
                        '-in', str(p/'msg'), '-out', str(p/'sig')], check=True, capture_output=True)
        return (p/'sig').read_bytes()


def openssl_verify(pub: bytes, message: bytes, sig: bytes) -> bool:
    with tempfile.TemporaryDirectory() as td:
        p = Path(td)
        (p/'key.der').write_bytes(SPKI + pub)
        (p/'msg').write_bytes(message)
        (p/'sig').write_bytes(sig)
        cp = subprocess.run(['openssl', 'pkeyutl', '-verify', '-rawin', '-pubin', '-keyform', 'DER',
                             '-inkey', str(p/'key.der'), '-in', str(p/'msg'), '-sigfile', str(p/'sig')],
                            capture_output=True)
        return cp.returncode == 0


class TestRecord(unittest.TestCase):
    def setUp(self):
        self.id = r.make_id('app.test')
        self.payload = b'opaque-test-payload\x00\xff'
        unsigned = r.signed_package(self.id, 7, self.payload, r.ROOT)
        self.file = unsigned + openssl_sign(RFC_SEED, r.PKG_DOMAIN + unsigned)
        self.sub_unsigned = r.signed_package(self.id, 8, b'another payload', SUB_PUB)
        self.sub_file = self.sub_unsigned + openssl_sign(SUB_SEED, r.PKG_DOMAIN + self.sub_unsigned)
        self.pol_unsigned = r.signed_policy(self.id, 1, SUB_PUB, 7, 1, ())
        self.policy = self.pol_unsigned + openssl_sign(RFC_SEED, r.POL_DOMAIN + self.pol_unsigned)

    def test_independent_wire_construction(self):
        # Deliberately do not invoke the reference encoders: independently
        # fill offsets with bytearray, integer.to_bytes and hashlib.
        pid = b'app.test' + bytes(24)
        self.assertEqual(pid, self.id)
        m = bytearray(128)
        m[0:4] = b'APKG'
        m[4:6] = (1).to_bytes(2, 'little')
        m[6:8] = (128).to_bytes(2, 'little')
        m[8:10] = (1).to_bytes(2, 'little')
        m[12:44] = pid
        m[44:52] = (7).to_bytes(8, 'little')
        m[52:56] = len(self.payload).to_bytes(4, 'little')
        m[56:88] = hashlib.sha256(self.payload).digest()
        m[88:120] = hashlib.sha256(r.ROOT).digest()
        self.assertEqual(bytes(m) + self.payload, self.file[:-64])
        header = bytearray(448)
        header[:4] = b'APOL'
        header[4:6] = (1).to_bytes(2, 'little')
        header[6:8] = (448).to_bytes(2, 'little')
        header[8:16] = (1).to_bytes(8, 'little')
        header[16:48] = pid
        header[48:80] = SUB_PUB
        header[80:112] = hashlib.sha256(SUB_PUB).digest()
        header[112:120] = (7).to_bytes(8, 'little')
        header[120] = 1
        self.assertEqual(bytes(header), self.policy[:-64])

    def test_rfc_vector2_and_signed_domains(self):
        # RFC 8032 vector 2 (one-byte input), independently signed by OpenSSL.
        # OpenSSL's pkeyutl CLI refuses an empty input even though Ed25519
        # permits it; vector 1's *public* key is exercised below.
        rfc_sig = bytes.fromhex(
            '92a009a9f0d4cab8720e820b5f642540a2b27b5416503f8fb3762223ebdb69da'
            '085ac1e43e15996e458f3613d0f11d8c387b2eaeb4302aeeb00d291612bb0c00')
        self.assertEqual(openssl_sign(SUB_SEED, bytes.fromhex('72')), rfc_sig)
        self.assertTrue(openssl_verify(SUB_PUB, bytes.fromhex('72'), rfc_sig))
        self.assertEqual(hashlib.sha256(r.ROOT).hexdigest(),
                         '21fe31dfa154a261626bf854046fd2271b7bed4b6abe45aa58877ef47f9721b9')
        self.assertEqual(len(self.file), 192 + len(self.payload))
        # Frozen test-only wire-vector digests/signatures: bytes are generated
        # by the stdlib codec and the unrelated OpenSSL Ed25519 engine.
        self.assertEqual(hashlib.sha256(self.file).hexdigest(),
                         '79a0ebe5653addf98c8060fc9dc7d21a748c1d3d258551041a2caa44444409fe')
        self.assertEqual(self.file[-64:].hex(),
                         '8a4c35fb9f7bffbcb68bebadefe8d6e48c666e3a5bf21c7fefe9c2ba9eca6ea28'
                         'c39d4122f99249c902f09413edcebd23b7a4f1598e2df18e2044ee2107e0003')
        self.assertEqual(hashlib.sha256(self.policy).hexdigest(),
                         '06487f2f7e48586e876b85526ba9f2b8c6b9f044eb749d167cc0a1a33ef97ec1')
        self.assertEqual(self.policy[-64:].hex(),
                         '1ad43e6ef3e9245907f589800c036dec163e4b35ee21562ff28cd0b558fc699c'
                         'adefc53e41f3610f95ad0ab0bb727e87b4255190c49cfc1367e4fae24d4a5200')
        p = r.parse_package(self.file)
        self.assertEqual(p.signed, r.PKG_DOMAIN + self.file[:-64])
        self.assertTrue(openssl_verify(r.ROOT, p.signed, p.signature))
        self.assertFalse(openssl_verify(SUB_PUB, p.signed, p.signature))
        self.assertFalse(openssl_verify(r.ROOT, r.POL_DOMAIN + self.file[:-64], p.signature))
        q = r.parse_policy(self.policy)
        self.assertEqual(q.signed, r.POL_DOMAIN + self.policy[:448])
        self.assertTrue(openssl_verify(r.ROOT, q.signed, q.signature))
        self.assertFalse(openssl_verify(SUB_PUB, q.signed, q.signature))
        self.assertEqual(q.subordinate, SUB_PUB)
        self.assertTrue(openssl_verify(SUB_PUB, r.parse_package(self.sub_file).signed,
                                       r.parse_package(self.sub_file).signature))

    def test_boundaries(self):
        self.assertEqual(r.make_id('x'*31), b'x'*31+b'\0')
        with self.assertRaises(r.Refusal): r.make_id('x'*32)
        with self.assertRaises(r.Refusal): r.make_id('../admin')
        for n in (1, 4096):
            unsigned = r.signed_package(self.id, 1, b'x'*n, r.ROOT)
            file = unsigned + openssl_sign(RFC_SEED, r.PKG_DOMAIN + unsigned)
            parsed = r.parse_package(file)
            self.assertEqual(parsed.payload, b'x'*n)
            self.assertTrue(openssl_verify(r.ROOT, parsed.signed, parsed.signature))
        with self.assertRaises(r.Refusal): r.signed_package(self.id, 1, b'x'*4097, r.ROOT)

    def test_negative_package(self):
        for data in [b'', self.file[:-1], self.file+b'!',
                     self.file[:128]+self.file[129:],
                     self.file[:4]+b'\x02'+self.file[5:],
                     self.file[:10]+b'\x01'+self.file[11:],
                     self.file[:12]+b'X'+self.file[13:],
                     self.file[:120]+b'\x01'+self.file[121:],
                     self.file[:128]+b'X'+self.file[129:]]:
            with self.subTest(data=data[:16]):
                with self.assertRaises(r.Refusal): r.parse_package(data)
        badsig = self.file[:-1] + bytes([self.file[-1] ^ 1])
        p = r.parse_package(badsig)
        self.assertFalse(openssl_verify(r.ROOT, p.signed, p.signature))
        mutated_signed = self.file[:44] + b'\x08' + self.file[45:]
        p = r.parse_package(mutated_signed)
        self.assertFalse(openssl_verify(r.ROOT, p.signed, p.signature))
        self.assertNotEqual(r.parse_package(self.file).full_digest, r.parse_package(badsig).full_digest)

    def test_policy_chain_exhaustion(self):
        q = r.parse_policy(self.policy)
        self.assertEqual(r.chain([q]), q)
        rev = tuple(sorted(hashlib.sha256(bytes([i])).digest() for i in range(8)))
        self.assertEqual(len(rev), 8)
        extra = hashlib.sha256(b'extra').digest()
        with self.assertRaisesRegex(r.Refusal, 'NO_SPACE'): r.revoked_add(rev, extra)
        self.assertEqual(r.revoked_add(rev, rev[0]), rev)
        for change in [lambda x: x[:-1],
                       lambda x: x[:121]+b'\x09'+x[122:],
                       lambda x: x[:384]+b'X'+x[385:],
                       lambda x: x[:80]+b'X'+x[81:]]:
            with self.assertRaises(r.Refusal): r.parse_policy(change(self.policy))
        new_unsigned = r.signed_policy(self.id, 2, SUB_PUB, 7, 2, rev)
        new = r.parse_policy(new_unsigned + openssl_sign(RFC_SEED, r.POL_DOMAIN+new_unsigned))
        self.assertEqual(r.chain([q,new]), new)
        with self.assertRaises(r.Refusal): r.chain([new])
        with self.assertRaises(r.Refusal): r.chain([q,q])
        missing = r.parse_policy(r.signed_policy(self.id, 3, SUB_PUB, 7, 1, rev)+bytes(64))
        with self.assertRaises(r.Refusal): r.chain([q,new,missing])

    def test_bounded_mutation_fuzz(self):
        rng = random.Random(0x84_0053)
        # A parser may accept signature-only mutations: then independent
        # OpenSSL must reject them. No mutation may become a valid artifact.
        for base, parser, pub in [(self.file, r.parse_package, r.ROOT),
                                  (self.policy, r.parse_policy, r.ROOT)]:
            for i in range(5000):
                b = bytearray(base)
                action = rng.randrange(3)
                if action == 0:
                    b[rng.randrange(len(b))] ^= rng.randrange(1, 256)
                elif action == 1:
                    b = b[:rng.randrange(len(b))]
                else:
                    b.append(rng.randrange(256))
                try:
                    record = parser(bytes(b))
                except r.Refusal:
                    continue
                # Keep crypto subprocess count bounded; all valid parses in
                # this corpus differ from the original in signed bytes or sig.
                self.assertFalse(openssl_verify(pub, record.signed, record.signature), i)


if __name__ == '__main__':
    unittest.main(verbosity=2)
