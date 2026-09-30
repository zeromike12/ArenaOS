#!/usr/bin/env python3
"""Offline synthetic unit tests only; do not assert real candidate archives were obtained."""
import hashlib
import io
import json
import tarfile
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import package_vendor as v


def archive(members):
    b = io.BytesIO()
    with tarfile.open(fileobj=b, mode='w:gz') as tf:
        for name, content, typ in members:
            info = tarfile.TarInfo(name)
            info.type = typ
            info.size = len(content) if typ == tarfile.REGTYPE else 0
            tf.addfile(info, io.BytesIO(content) if typ == tarfile.REGTYPE else None)
    return b.getvalue()


class VendorTests(unittest.TestCase):
    def test_table(self):
        entries = v.inventory()
        self.assertEqual(len(entries), 23)
        self.assertEqual(entries['ed25519-dalek-2.2.0'],
                         '70e796c081cee67dc755e1a36a0a172b897fab85fc3f6bc48307991f64e4eca9')

    def test_checked_staging_and_wrong_checksum_refusal(self):
        crate = 'example-1.0.0'
        data = archive([(crate+'/Cargo.toml', b'[package]\nname="example"\nversion="1.0.0"\n', tarfile.REGTYPE),
                        (crate+'/src/lib.rs', b'// fixture\n', tarfile.REGTYPE)])
        expected = hashlib.sha256(data).hexdigest()
        with tempfile.TemporaryDirectory() as td, patch.object(v, 'inventory', return_value={crate: expected}):
            parent = Path(td)
            archives = parent / 'archives'; archives.mkdir()
            path = archives / f'{crate}.crate'
            path.write_bytes(data)
            out = parent / 'out'
            v.vendor(archives, out)
            metadata = json.loads((out / crate / '.cargo-checksum.json').read_text())
            self.assertEqual(metadata['package'], expected)
            self.assertEqual(metadata['files']['src/lib.rs'], hashlib.sha256(b'// fixture\n').hexdigest())
            with self.assertRaisesRegex(ValueError, 'existing destination'):
                v.vendor(archives, out)
            path.write_bytes(data + b'changed')
            with self.assertRaisesRegex(ValueError, 'SHA-256 differs'):
                v.vendor(archives, parent / 'new')
            self.assertFalse((parent / 'new').exists())

    def test_traversal_link_duplicate_and_missing_manifest_refuse(self):
        crate = 'example-1.0.0'
        bad = [
            [(crate+'/../escape', b'x', tarfile.REGTYPE),
             (crate+'/Cargo.toml', b'c', tarfile.REGTYPE)],
            [(crate+'/src/lib.rs', b'x', tarfile.SYMTYPE),
             (crate+'/Cargo.toml', b'c', tarfile.REGTYPE)],
            [(crate+'/Cargo.toml', b'c', tarfile.REGTYPE),
             (crate+'/Cargo.toml', b'c', tarfile.REGTYPE)],
            [(crate+'/src/lib.rs', b'x', tarfile.REGTYPE)],
        ]
        with tempfile.TemporaryDirectory() as td:
            for members in bad:
                data = archive(members)
                with self.assertRaises(ValueError):
                    v.unpack(crate, data, Path(td), hashlib.sha256(data).hexdigest())


if __name__ == '__main__':
    unittest.main(verbosity=2)
