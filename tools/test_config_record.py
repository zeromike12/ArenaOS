#!/usr/bin/env python3
"""Host-only ADR-0046 record/recovery model; NOT a guest 8.1 milestone."""
import struct
import unittest

from afs1 import fnv1a64
from config_record import ConfigCorrupt, name, pack, recover, unpack


def reseal(blob: bytes) -> bytes:
    return blob[:504] + struct.pack("<Q", fnv1a64(blob[:504]))


class RecordTests(unittest.TestCase):
    def test_exact_layout_empty_and_max_payload(self):
        for data in (b"", bytes(range(32))):
            raw = pack(1, data)
            self.assertEqual(len(raw), 512)
            self.assertEqual(raw[:8], b"ARCFG8V1")
            self.assertEqual(raw[8:16], struct.pack("<II", 1, 0))
            self.assertEqual(raw[16:26], struct.pack("<QH", 1, len(data)))
            self.assertEqual(raw[26:32], b"\0" * 6)
            self.assertEqual(raw[32 + len(data):504], b"\0" * (472 - len(data)))
            self.assertEqual(unpack(raw, 1).payload, data)
        with self.assertRaises(ValueError):
            pack(1, bytes(33))
        for seq in (0, 9, (1 << 64) - 1):
            with self.assertRaises(ValueError):
                pack(seq, b"bad")

    def test_every_visible_corruption_fails_closed(self):
        good = pack(2, b"new-value")
        changes = {
            "magic": (0, 0), "version": (8, 2), "reserved-header": (12, 1),
            "sequence": (16, 3), "length": (24, 33), "reserved-short": (26, 1),
            "unused-payload": (47, 9), "tail": (503, 1),
        }
        for field, (offset, byte) in changes.items():
            with self.subTest(field=field):
                bad = bytearray(good)
                bad[offset] = byte
                with self.assertRaises(ConfigCorrupt):
                    recover([(name(1), pack(1, b"old")), (name(2), reseal(bad))])
        bad = bytearray(good)
        bad[36] ^= 1  # payload corrupted without repairing checksum
        for altered in (bytes(bad), good[:12], good + b"x"):
            with self.assertRaises(ConfigCorrupt):
                recover([(name(1), pack(1, b"old")), (name(2), altered)])

    def test_namespace_and_pending(self):
        old = (name(1), pack(1, b"old"))
        new = (name(2), pack(2, b"new"))
        self.assertEqual(recover([]).next_name, "cfg8-01")
        self.assertEqual(recover([("arena.txt", b"ordinary")]).current, None)
        first_empty = recover([(name(1), b"")])
        self.assertIsNone(first_empty.current)
        self.assertEqual(first_empty.pending, name(1))
        state = recover([new, (name(3), b""), old])
        self.assertEqual(state.current.payload, b"new")
        self.assertEqual(state.next_name, name(3))
        self.assertIsNone(recover([(name(n), pack(n, b"x")) for n in range(1, 9)]).next_name)
        malformed = [
            [old, old], [new], [old, (name(3), pack(3, b"gap"))],
            [(name(1), b""), new], [old, (name(2), b""), (name(3), b"")],
            [old, ("cfg8-00", b"")], [old, ("cfg8-09", b"")],
            [old, ("cfg8-1", b"")], [old, ("cfg8-01.bak", b"")],
            [old, ("cfg8-secret", b"")],
        ]
        for entries in malformed:
            with self.subTest(entries=entries):
                with self.assertRaises(ConfigCorrupt):
                    recover(entries)


if __name__ == "__main__":
    unittest.main()
