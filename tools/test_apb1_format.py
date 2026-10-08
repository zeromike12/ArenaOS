#!/usr/bin/env python3
"""Independent APB1 wire, signature, bound, and mutation controls."""
from __future__ import annotations

import hashlib
import random
import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import apb1_format as apb

FIXTURE_DIR = Path(__file__).resolve().parents[1] / "userspace/arena-platform/tests/data"


class TestApb1Reference(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.fixtures = apb.default_fixtures()

    def test_checked_in_fixtures_are_reproducible_and_independent_signature_valid(self):
        for name, expected in self.fixtures.items():
            with self.subTest(name=name):
                self.assertEqual((FIXTURE_DIR / name).read_bytes(), expected)
                if name != "oversized-total.apb1":
                    self.assertEqual(len(expected), int.from_bytes(expected[24:32], "little")
                                     + int.from_bytes(expected[16:24], "little")
                                     + apb.HEADER_BYTES + apb.MANIFEST_BYTES + 64)
                metadata_len = int.from_bytes(expected[16:24], "little") + 64 + 512
                # Ed25519 signs the complete canonical metadata, including the
                # domain separator. Host OpenSSL is independent of the guest
                # receiver's pinned Rust crypto closure.
                self.assertTrue(apb.verify_openssl(
                    apb.RFC_PUBLIC, apb.DOMAIN + expected[:metadata_len],
                    expected[metadata_len:metadata_len+64]))
                self.assertEqual(expected[32:64], hashlib.sha256(apb.RFC_PUBLIC).digest())

    def test_large_bundle_fixture_exercises_a_nontrivial_payload(self):
        large = self.fixtures["large.apb1"]
        parsed = apb.parse_bundle(large)
        self.assertEqual(len(parsed.files), 1)
        self.assertEqual(parsed.files[0][1], b"bin/editor")
        self.assertEqual(parsed.files[0][2], 32 * 1024)
        self.assertEqual(len(parsed.payload), 32 * 1024)

    def test_valid_multifile_bundle_exactly_accounts_for_payload(self):
        valid = self.fixtures["editor.apb1"]
        parsed = apb.parse_bundle(valid)
        self.assertEqual(len(parsed.files), 3)
        self.assertEqual([row[1] for row in parsed.files],
                         [b"bin/editor", b"bin/helper", b"icons/editor.bin"])
        self.assertEqual([row[2] for row in parsed.files], [18, 7, 7])
        self.assertEqual(parsed.payload, b"fixture-main-imagehelper!PNGfake")
        self.assertEqual(parsed.bundle_digest, hashlib.sha256(valid).digest())
        self.assertEqual(parsed.payload_offset, len(parsed.metadata) + 64)

    def test_phase13_stream_and_sync_manifest_hints_are_known_but_unknown_bits_refuse(self):
        declared = apb.make_manifest(
            app_id=b"org.arenaos.phase13app",
            package_id=b"org.arena.editor",
            display_name=b"Phase13 Probe",
            version=13,
            flags=0x1F,
            requested=0,
            entry=b"bin/probe",
            icon=b"",
            width=320,
            height=180,
            associations=(b"text/plain",),
        )
        bundle = apb.build_bundle([(1, b"bin/probe", b"native image")], manifest=declared)
        self.assertEqual(apb.parse_bundle(bundle).metadata[HEADER_FLAGS_OFFSET:HEADER_FLAGS_OFFSET + 4],
                         (0x1F).to_bytes(4, "little"))
        unknown = apb.make_manifest(
            app_id=b"org.arenaos.phase13app",
            package_id=b"org.arena.editor",
            display_name=b"Phase13 Probe",
            version=13,
            flags=0x20,
            requested=0,
            entry=b"bin/probe",
            icon=b"",
            width=320,
            height=180,
            associations=(b"text/plain",),
        )
        with self.assertRaisesRegex(apb.Refusal, "reserved manifest bits"):
            apb.parse_bundle(apb.build_bundle([(1, b"bin/probe", b"native image")], manifest=unknown))

    def test_signed_hostile_metadata_controls_refuse(self):
        for name in ("traversal.apb1", "duplicate.apb1", "prefix-conflict.apb1",
                     "noncanonical.apb1", "reserved-record.apb1",
                     "oversized-file.apb1", "oversized-total.apb1"):
            with self.subTest(name=name):
                raw = self.fixtures[name]
                metadata_len = int.from_bytes(raw[16:24], "little") + 64 + 512
                self.assertTrue(apb.verify_openssl(
                    apb.RFC_PUBLIC, apb.DOMAIN + raw[:metadata_len],
                    raw[metadata_len:metadata_len+64]))
                with self.assertRaises(apb.Refusal):
                    apb.parse_bundle(raw)

    def test_signature_payload_manifest_and_exact_length_mutations_refuse(self):
        original = self.fixtures["editor.apb1"]
        parsed = apb.parse_bundle(original)
        metadata_len = len(parsed.metadata)
        signature_offset = metadata_len
        payload_offset = parsed.payload_offset

        bad_signature = bytearray(original)
        bad_signature[signature_offset] ^= 1
        mutated = apb.parse_bundle(bytes(bad_signature))
        self.assertFalse(apb.verify_openssl(
            apb.RFC_PUBLIC, apb.DOMAIN + mutated.metadata, mutated.signature))

        # The metadata signature remains authentic, but any payload mutation
        # that omits or bypasses the per-file digest must be caught.
        bad_payload = bytearray(original)
        bad_payload[-1] ^= 1
        with self.assertRaisesRegex(apb.Refusal, "file hash"):
            apb.parse_bundle(bytes(bad_payload))

        bad_metadata = bytearray(original)
        bad_metadata[HEADER_RESERVED_OFFSET] = 1
        # Offset zero in the row's reserved bytes (see APB1 row contract).
        with self.assertRaisesRegex(apb.Refusal, "record kind/length/reserved"):
            apb.parse_bundle(bytes(bad_metadata))

        trailing = original + b"x"
        truncated = original[:-1]
        with self.assertRaises(apb.Refusal):
            apb.parse_bundle(trailing)
        with self.assertRaises(apb.Refusal):
            apb.parse_bundle(truncated)

        huge = bytearray(original)
        huge[24:32] = (apb.MAX_TOTAL_BYTES + 1).to_bytes(8, "little")
        with self.assertRaises(apb.Refusal):
            apb.parse_bundle(bytes(huge))

        altered_manifest = bytearray(original)
        altered_manifest[apb.HEADER_BYTES + 104] ^= 1  # manifest version, signed
        changed = apb.parse_bundle(bytes(altered_manifest))
        self.assertFalse(apb.verify_openssl(
            apb.RFC_PUBLIC, apb.DOMAIN + changed.metadata, changed.signature))
        # Keep these offsets visibly tied to parser-derived envelope boundaries.
        self.assertGreater(payload_offset, signature_offset)

    def test_bounded_random_mutations_never_upgrade_to_valid_signed_bundle(self):
        randomizer = random.Random(0x12_0081)
        original = self.fixtures["editor.apb1"]
        parsed_original = apb.parse_bundle(original)
        accepted = 0
        for index in range(256):
            mutated = bytearray(original)
            kind = randomizer.randrange(3)
            if kind == 0:
                offset = randomizer.randrange(len(mutated))
                mutated[offset] ^= randomizer.randrange(1, 256)
            elif kind == 1:
                del mutated[randomizer.randrange(len(mutated)):]
            else:
                mutated.append(randomizer.randrange(256))
            try:
                parsed = apb.parse_bundle(bytes(mutated))
            except apb.Refusal:
                continue
            accepted += 1
            self.assertFalse(apb.verify_openssl(
                apb.RFC_PUBLIC, apb.DOMAIN + parsed.metadata, parsed.signature), index)
        self.assertGreater(accepted, 0)
        self.assertNotEqual(parsed_original.bundle_digest, hashlib.sha256(b"").digest())


# File table begins after the 64-byte header and 512-byte manifest. The first
# row's five reserved bytes begin at its byte 3.
HEADER_RESERVED_OFFSET = apb.HEADER_BYTES + apb.MANIFEST_BYTES + 3
HEADER_FLAGS_OFFSET = apb.HEADER_BYTES + 112


if __name__ == "__main__":
    unittest.main(verbosity=2)
