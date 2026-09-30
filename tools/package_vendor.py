#!/usr/bin/env python3
"""Fail-closed, offline archive acquisition helper for proposed ADR-0053.

Usage: python3 tools/package_vendor.py --archives /path/to/exact-crates --out build/phase84-sources

Only accepts all 23 exact name-version.crate archives whose SHA-256 matches the
registry archive checksums already recorded in ADR-0053. Unpacks atomically,
rejects links/traversal, and creates cargo-vendor-compatible checksums. This
is NOT a feature/unsafe/license audit and does NOT accept ADR-0053.
"""
from __future__ import annotations

import argparse
import hashlib
import io
import json
import re
import shutil
import tarfile
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
ADR = ROOT / 'docs/adr/0053-signed-package-trust-and-staging.md'
ROW = re.compile(r'^\| `([a-z0-9_-]+)` \| `([0-9]+\.[0-9]+\.[0-9]+)` \| `([a-f0-9]{64})` \|$', re.M)
MAX_FILE = 32 * 1024 * 1024
MAX_ARCHIVE = 128 * 1024 * 1024


def sha(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def inventory() -> dict[str, str]:
    rows = ROW.findall(ADR.read_text())
    if len(rows) != 23 or len({(name, version) for name, version, _ in rows}) != 23:
        raise ValueError('ADR-0053 archive registry table missing/ambiguous')
    return {f'{name}-{version}': checksum for name, version, checksum in rows}


def verified(crate: str, archive: Path, expected: str) -> bytes:
    if archive.stat().st_size > MAX_ARCHIVE:
        raise ValueError(f'{crate}: archive too large')
    data = archive.read_bytes()
    if sha(data) != expected:
        raise ValueError(f'{crate}: archive SHA-256 differs from ADR-0053 registry checksum')
    return data


def unpack(crate: str, data: bytes, dest: Path, checksum: str) -> None:
    checksums: dict[str, str] = {}
    prefix = crate + '/'
    total = 0
    with tarfile.open(fileobj=io.BytesIO(data), mode='r:gz') as tf:
        for member in tf:
            if member.isdir():
                continue
            if not member.isfile() or member.size > MAX_FILE or not member.name.startswith(prefix):
                raise ValueError(f'{crate}: invalid member {member.name!r}')
            rel = member.name[len(prefix):]
            parts = rel.split('/')
            if (not rel or any(part in ('..', '.', '') for part in parts) or
                    '\\' in rel or '\x00' in rel or Path(rel).is_absolute()):
                raise ValueError(f'{crate}: invalid path {rel!r}')
            total += member.size
            if total > MAX_ARCHIVE or len(checksums) >= 10000:
                raise ValueError(f'{crate}: expanded archive budget exceeded')
            if rel in checksums or rel == '.cargo-checksum.json':
                raise ValueError(f'{crate}: repeated or reserved member {rel!r}')
            source = tf.extractfile(member)
            if source is None:
                raise ValueError(f'{crate}: unreadable member {rel!r}')
            content = source.read(MAX_FILE + 1)
            if len(content) != member.size:
                raise ValueError(f'{crate}: truncated member {rel!r}')
            f = dest / rel
            f.parent.mkdir(parents=True, exist_ok=True)
            f.write_bytes(content)
            checksums[rel] = sha(content)
    if 'Cargo.toml' not in checksums or not checksums:
        raise ValueError(f'{crate}: missing manifest')
    (dest / '.cargo-checksum.json').write_text(json.dumps(
        {'files': dict(sorted(checksums.items())), 'package': checksum}, sort_keys=True) + '\n')


def vendor(archives: Path, out: Path) -> None:
    entries = inventory()
    # Verify all before creating ANY destination. Never silently substitute a
    # GitHub source tag or a mismatching future release.
    data = {crate: verified(crate, archives / f'{crate}.crate', checksum)
            for crate, checksum in entries.items()}
    if out.exists():
        raise ValueError(f'refusing existing destination: {out}')
    out.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='.phase84-vendor-', dir=out.parent) as tmp:
        staged = Path(tmp) / 'source'
        staged.mkdir()
        for crate, checksum in entries.items():
            dest = staged / crate
            dest.mkdir()
            unpack(crate, data[crate], dest, checksum)
        shutil.move(str(staged), str(out))
    print(f'verified {len(entries)} registry archives and staged exact extracted sources at {out}')
    print('NOT AUDITED: inspect features, licenses, upstream revisions, build/proc macro and unsafe code')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--archives', required=True, type=Path)
    parser.add_argument('--out', required=True, type=Path)
    args = parser.parse_args()
    vendor(args.archives, args.out)
