#!/usr/bin/env python3
"""Isolated fail-closed tests; synthetic archives, no network or real vendor mutation."""
import stat
import tempfile
import unittest
import zipfile
from pathlib import Path
from unittest.mock import patch

from phase84_zip_unpack import unpack
from package_vendor import sha


class ArchiveZipTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.expected = {'one-1.0.0': sha(b'one'), 'two-1.0.0': sha(b'two')}
        self.patcher = patch('phase84_zip_unpack.inventory', return_value=self.expected)
        self.patcher.start()
        self.addCleanup(self.patcher.stop)

    def zipped(self, entries):
        with zipfile.ZipFile(self.root / 'input.zip', 'w') as z:
            for name, content, mode in entries:
                info = zipfile.ZipInfo(name)
                info.create_system = 3
                info.external_attr = mode << 16
                z.writestr(info, content)
        return self.root / 'input.zip'

    def good(self):
        return [('one-1.0.0.crate', b'one', stat.S_IFREG),
                ('two-1.0.0.crate', b'two', stat.S_IFREG)]

    def test_exact_then_existing_destination_refused(self):
        zipped = self.zipped(self.good())
        unpack(zipped, self.root / 'out')
        self.assertEqual((self.root / 'out' / 'one-1.0.0.crate').read_bytes(), b'one')
        with self.assertRaisesRegex(ValueError, 'existing destination'):
            unpack(zipped, self.root / 'out')

    def test_bad_hash_duplicate_path_symlink_and_extra_refuse_atomically(self):
        cases = [
            [('one-1.0.0.crate', b'BAD', stat.S_IFREG), self.good()[1]],
            [self.good()[0], self.good()[0]],
            [('a/one-1.0.0.crate', b'one', stat.S_IFREG), self.good()[1]],
            [('one-1.0.0.crate', b'one', stat.S_IFLNK), self.good()[1]],
            self.good() + [('extra.crate', b'bad', stat.S_IFREG)],
        ]
        for entries in cases:
            with self.subTest(entries=entries):
                with self.assertRaises(ValueError):
                    unpack(self.zipped(entries), self.root / 'out')
                self.assertFalse((self.root / 'out').exists())


if __name__ == '__main__':
    unittest.main()
