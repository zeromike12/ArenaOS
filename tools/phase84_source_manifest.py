#!/usr/bin/env python3
"""Recompute ADR-0053 vendored-source provenance; never invent an audit PASS.

Usage: python3 tools/phase84_source_manifest.py > audit/phase84-crypto/SOURCE-MANIFEST.tsv
Reads only the 23 exact source archives recorded in ADR-0053 and the vendored
sources independently extracted from those archives by package_vendor.py.
"""
import hashlib
import json
import sys
import tomllib
from pathlib import Path

from package_vendor import inventory

ROOT = Path(__file__).resolve().parent.parent
VENDOR = ROOT / 'vendor/phase84'


def digest(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def main() -> None:
    candidates = inventory()
    actual = {p.name for p in VENDOR.iterdir() if p.is_dir()}
    if actual != set(candidates):
        raise ValueError(f'vendor closure mismatch: missing={set(candidates)-actual}, extra={actual-set(candidates)}')
    print('crate\tarchive_sha256\textracted_tree_sha256\tupstream_git_sha1\tlicense_declared\tlicense_files\tfiles')
    for name, archive_hash in sorted(candidates.items()):
        crate = VENDOR / name
        sums = json.loads((crate / '.cargo-checksum.json').read_text())
        if sums['package'] != archive_hash:
            raise ValueError(f'{name}: archive checksum drift')
        source_files = sorted(p for p in crate.rglob('*') if p.is_file() and p.name != '.cargo-checksum.json')
        actual_hashes = {str(p.relative_to(crate)): digest(p.read_bytes()) for p in source_files}
        if actual_hashes != sums['files']:
            raise ValueError(f'{name}: extracted file hash drift')
        h = hashlib.sha256()
        for rel, value in sorted(actual_hashes.items()):
            h.update(rel.encode() + b'\0' + bytes.fromhex(value))
        manifest = tomllib.loads((crate / 'Cargo.toml').read_text())
        vcs = crate / '.cargo_vcs_info.json'
        revision = json.loads(vcs.read_text()).get('git', {}).get('sha1', '-') if vcs.exists() else '-'
        licenses = ','.join(sorted(p.name for p in crate.iterdir() if p.name.lower().startswith(('license', 'copying'))))
        print('\t'.join((name, archive_hash, h.hexdigest(), revision,
                         manifest['package'].get('license', 'UNSPECIFIED'), licenses, str(len(source_files)))))


if __name__ == '__main__':
    try:
        main()
    except Exception as exc:
        print(f'ADR-0053 provenance refused: {exc}', file=sys.stderr)
        sys.exit(1)
