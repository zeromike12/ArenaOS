#!/usr/bin/env python3
"""Fail-closed, offline extraction of the exact ADR-0053 archive ZIP.

Usage: python3 tools/phase84_zip_unpack.py --zip crateFiles.zip --out build/phase84-crates
Then: python3 tools/package_vendor.py --archives build/phase84-crates --out build/phase84-sources
Never write any extracted file until all 23 entry names, types, sizes and
SHA-256 archive checksums match the *preexisting* ADR inventory.
"""
import argparse
import stat
import tempfile
import zipfile
from pathlib import Path

from package_vendor import inventory, sha

MAX_TOTAL = 128 * 1024 * 1024
MAX_ENTRY = 32 * 1024 * 1024


def unpack(zpath: Path, out: Path) -> None:
    expected = {name + '.crate': digest for name, digest in inventory().items()}
    data = {}
    total = 0
    with zipfile.ZipFile(zpath) as z:
        entries = z.infolist()
        if len(entries) != len(expected) or {x.filename for x in entries} != set(expected):
            raise ValueError('ZIP entries differ from ADR-0053 inventory or are repeated')
        for info in entries:
            mode = stat.S_IFMT(info.external_attr >> 16)
            if info.is_dir() or (info.create_system == 3 and mode not in (stat.S_IFREG, 0)):
                raise ValueError(f'{info.filename}: not a regular file')
            if info.file_size > MAX_ENTRY or total + info.file_size > MAX_TOTAL:
                raise ValueError(f'{info.filename}: expanded size budget exceeded')
            total += info.file_size
            with z.open(info) as stream:
                content = stream.read(MAX_ENTRY + 1)
                if len(content) != info.file_size or stream.read(1):
                    raise ValueError(f'{info.filename}: truncated/oversized member')
            if sha(content) != expected[info.filename]:
                raise ValueError(f'{info.filename}: archive SHA-256 differs from ADR-0053')
            data[info.filename] = content
    if out.exists():
        raise ValueError(f'refusing existing destination: {out}')
    out.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='.phase84-crates-', dir=out.parent) as tmp:
        staged = Path(tmp) / 'archives'
        staged.mkdir()
        for name, content in sorted(data.items()):
            (staged / name).write_bytes(content)
        staged.rename(out)
    print(f'verified and extracted {len(data)} exact ADR-0053 archives to {out}')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--zip', required=True, type=Path)
    parser.add_argument('--out', required=True, type=Path)
    args = parser.parse_args()
    unpack(args.zip, args.out)
