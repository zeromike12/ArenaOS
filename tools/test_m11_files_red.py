#!/usr/bin/env python3
"""Phase 11.6 RED controls for file capabilities (ADR-0077).

Each control removes one production mechanism from filesd and the full
guest test (`test_m11_files.py`) must fail on the property that mechanism
guards; a build failure never counts. The source is restored byte-exactly
and the kernel EFI rebuilt from it is byte-identical to the original
(deterministic image, kernel/.cargo/config.toml); the GREEN run is the
ordinary `test_m11_files.py` in the suite.
"""
import arena_env
import mtest
import test_m11_files as green

ROOT = arena_env.REPO_ROOT
BUILD = arena_env.build_dir()
FILESD = ROOT / 'userspace/filesd/src/main.rs'
CONTROLS = [
    # No attenuation: a record opened from another keeps the requested rights.
    (b'            match mint(place, object, req.rights & g.rights) {',
     b'            match mint(place, object, req.rights) {',
     'no-attenuation', ('read-only grant was written', 'probe painted red')),
    # Shallow revoke: retiring a lineage head leaves its derived records live.
    (b'    if head {\n        let mut n = 1u64;',
     b'    if head && false {\n        let mut n = 1u64;',
     'shallow-revoke', ('lineage not retired', 'probe lineage')),
]


def main():
    original = FILESD.read_bytes()
    for before, _, label, _ in CONTROLS:
        assert original.count(before) == 1, (label, original.count(before))
    esp = mtest.build('m11-files-red-base', desktop=True)
    artifacts = {p: p.read_bytes() for p in (esp, BUILD / 'arena-boot.efi')}
    reds = []
    try:
        for before, after, label, signatures in CONTROLS:
            FILESD.write_bytes(original.replace(before, after, 1))
            try:
                try:
                    green.main()
                except (AssertionError, RuntimeError) as error:
                    cause = error.__cause__ or error
                    text = f'{type(cause).__name__}: {cause}'
                    assert any(sig in text for sig in signatures), (label, text[:400])
                    reds.append(label)
                    print(f'[m11-files-red] {label}: {text[:160]} -> RED PASS', flush=True)
                else:
                    raise AssertionError(f'{label}: the guest test passed without the mechanism')
            finally:
                FILESD.write_bytes(original)
    finally:
        FILESD.write_bytes(original)
        mtest.build('m11-files-red-restored', desktop=True)
    assert FILESD.read_bytes() == original
    assert (BUILD / 'arena-boot.efi').read_bytes() == artifacts[BUILD / 'arena-boot.efi'], 'restored EFI differs'
    esp.write_bytes(artifacts[esp])
    print(f'[m11-files-red] {len(reds)} RED controls ({", ".join(reds)}); byte-exact source and EFI restore',
          flush=True)


if __name__ == '__main__':
    main()
