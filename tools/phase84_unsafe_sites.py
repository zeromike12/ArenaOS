#!/usr/bin/env python3
"""Inventory every lexical unsafe site in the exact 23-crate ADR-0053 closure.

Not a static proof: matches unsafe blocks/functions/traits/impls, even when cfg
excludes them, and deliberately does NOT label sites safe. Diff this report
against the pinned source + reviewed build graph before accepting a change.
"""
import re
from pathlib import Path
from package_vendor import inventory

ROOT = Path(__file__).resolve().parent.parent
VENDOR = ROOT / 'vendor/phase84'
MATCH = re.compile(r'\bunsafe\s*(?:\{|fn\b|impl\b|trait\b)')
DORMANT = {'curve25519-dalek-derive', 'fiat-crypto', 'libc',
           'proc-macro2', 'quote', 'syn', 'unicode-ident'}


def classify(package: str, relative: str) -> str:
    if package in DORMANT:
        return 'locked-conditional-not-compiled'
    if package == 'semver':
        return 'host-build-only'
    if package == 'curve25519-dalek' and (
        '/vector/' in '/' + relative or '/serial/u32/' in '/' + relative or relative == 'src/constants.rs'
    ):
        return 'target-cfg-or-feature-excluded'
    if package == 'cpufeatures':
        return 'hardware-detection-gated-on-none-or-other-arch'
    if package == 'sha2' and (relative.startswith('src/sha256/') or relative.startswith('src/sha512/')):
        return 'hardware-gated-on-none-or-other-arch'
    return 'compiled-runtime-source'


def main() -> None:
    print('crate\tsource\tline\tclassification\tstatement')
    count = 0
    for full in sorted(inventory()):
        package = full.rsplit('-', 1)[0]
        for file in sorted((VENDOR / full / 'src').rglob('*.rs')):
            rel = str(file.relative_to(VENDOR / full))
            for lineno, line in enumerate(file.read_text(errors='replace').splitlines(), 1):
                code = line.strip()
                if not code.startswith(('//', '*')) and MATCH.search(code):
                    print(f'{full}\t{rel}\t{lineno}\t{classify(package, rel)}\t{code.replace(chr(9), " ")}')
                    count += 1
    if count != 724:
        raise ValueError(f'unsafe inventory drift: expected 724, got {count}')


if __name__ == '__main__':
    main()
