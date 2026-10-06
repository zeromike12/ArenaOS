#!/usr/bin/env python3
"""Phase 11.5: the Rust AFS2 engine (`userspace/afs2`) against the format's
host model (`tools/afs2.py`, ADR-0076).

1. The engine's own host proofs (`cargo test`): randomized operations
   against a reference model with remount and structural audit after every
   step, directory split/merge, crash prefixes of every mutating operation
   (every prefix exactly old or exactly new, torn commit = old), crash
   prefixes of format (never committed or mountable), refusals
   that write nothing, 10,000 objects and a 16 MiB file, fail-closed mount.
2. Both directions of the format: an image the Rust engine wrote mounts in
   the Python model, passes its independent `check` and has the same
   namespace; an image the Python model wrote mounts in the Rust engine
   with the same namespace.
3. RED controls: each removes one production mechanism from the Rust
   source; the named test must fail (a compile error never counts). The
   source is restored byte-exactly and everything is GREEN again.
"""
import subprocess
import sys
import tempfile
from pathlib import Path
import afs2
import arena_env

ROOT = arena_env.REPO_ROOT
CRATE = ROOT / 'userspace/afs2'
LIB = CRATE / 'src/lib.rs'
TARGET = 'x86_64-unknown-linux-gnu'
CONTROLS = [
    # In-place metadata: an owned copy reuses the committed block number.
    (b'        let nb = self.alloc()?;\n        self.slots[i] = Slot {\n            block: nb,',
     b'        let nb = if block != 0 { block } else { self.alloc()? };\n        self.slots[i] = Slot {\n            block: nb,',
     'in-place-metadata', 'crash_prefixes_are_exactly_old_or_new'),
    # Early free: a block the committed generation references is reusable
    # inside the same transaction.
    (b'            // The committed generation references it until commit.\n            set(&mut self.pending, block);',
     b'            // The committed generation references it until commit.\n            set(&mut self.pending, block);\n            clear(&mut self.work, block);',
     'early-free', 'crash_prefixes_are_exactly_old_or_new'),
    # A move that forgets the child's parent pointer.
    (b'            child.parent = dst_dir;\n', b'',
     'missing-parent-update', 'random_operations_match_a_model_and_survive_remount'),
    # The commit sector is written before the metadata it publishes.
    (b'        // Dirty metadata.\n        for i in 0..DIRTY {',
     b'        // Dirty metadata.\n        self.write_commit(slot, &rec)?;\n        for i in 0..DIRTY {',
     'commit-not-last', 'crash_prefixes_are_exactly_old_or_new'),
    # A damaged committed volume treated as never committed (formatted
    # over instead of failing closed).
    (b'        zero_slot |= raw.iter().all(|b| *b == 0);',
     b'        zero_slot = true;',
     'reformat-damaged', 'format_prefixes_are_never_committed_or_mountable'),
]


def run(cmd, **kw):
    return subprocess.run(cmd, cwd=CRATE, env=arena_env.rust_env(), capture_output=True,
                          text=True, **kw)


def cargo_test():
    # The host image omits rustdoc; this crate has no doctests. Keep running
    # all engine unit proofs without making the historical suite depend on it.
    return run(['cargo', 'test', '--lib', '--offline', '--release', '--target', TARGET])


def lines_of_walk(tree, vol):
    out = []
    for path, data in sorted(tree.items()):
        if data is None:
            out.append(f'{path}\tD')
        else:
            out.append(f'{path}\tF\t{len(data)}\t{afs2.fnv(data):016x}')
    s = vol.statfs()
    out.append(f"#statfs\t{s['blocks']}\t{s['free']}\t{s['objects']}")
    return sorted(out)


def tool(*args):
    r = run(['cargo', 'run', '--quiet', '--offline', '--release', '--target', TARGET,
             '--example', 'afs2tool', '--', *args])
    assert r.returncode == 0, r.stderr[-2000:]
    return r.stdout


def cross_check():
    with tempfile.TemporaryDirectory(prefix='afs2-rust-') as work:
        img = Path(work) / 'rust.img'
        tool('build', str(img))
        vol = afs2.Volume(img.read_bytes())
        afs2.check(vol)
        assert not afs2.never_committed(img.read_bytes()) and afs2.never_committed(bytes(len(vol.image())))
        expect = lines_of_walk(afs2.walk(vol), vol)
        got = sorted(tool('dump', str(img)).splitlines())
        assert got == expect, 'Rust image: namespaces differ'
        assert any('/Users/user/Documents/' in line for line in got)
        # Python-written image read by the Rust engine.
        vol = afs2.Volume(afs2.mkfs(4096))
        root = vol.root_id()
        d = vol.mkdir(root, b'py-dir')
        for i in range(300):
            f = vol.create(d, f'py-file-{i:03} name'.encode())
            vol.write(f, i * 7, bytes([i % 256]) * (i * 13 % 9000))
        big = vol.create(root, b'two-level.bin')
        vol.write(big, 0, bytes(range(256)) * (4096 * 600 // 256))
        vol.rename(d, b'py-file-007 name', root, b'renamed')
        vol.unlink(d, b'py-file-008 name')
        img.write_bytes(vol.image())
        expect = lines_of_walk(afs2.walk(vol), vol)
        got = sorted(tool('dump', str(img)).splitlines())
        assert got == expect, 'Python image: namespaces differ'
    print(f'[afs2-rust] format cross-check both directions PASS ({len(got)} namespace lines)', flush=True)


def main(red=True):
    r = cargo_test()
    assert r.returncode == 0, r.stdout[-3000:] + r.stderr[-3000:]
    print('\n'.join(l for l in r.stdout.splitlines() if l.startswith(('[afs2-rust]', 'test result'))),
          flush=True)
    cross_check()
    if not red:
        return
    original = LIB.read_bytes()
    for before, _, label, _ in CONTROLS:
        assert original.count(before) == 1, (label, original.count(before))
    reds = []
    try:
        for before, after, label, test in CONTROLS:
            LIB.write_bytes(original.replace(before, after, 1))
            try:
                r = cargo_test()
                out = r.stdout + r.stderr
                assert 'error[' not in out, f'{label}: mutant did not compile (never RED)'
                assert r.returncode != 0 and f'{test} ... FAILED' in out, f'{label}: {test} did not fail'
                reds.append(label)
                print(f'[afs2-rust-red] {label}: {test} FAILED -> RED PASS', flush=True)
            finally:
                LIB.write_bytes(original)
    finally:
        LIB.write_bytes(original)
    assert LIB.read_bytes() == original
    r = cargo_test()
    assert r.returncode == 0, 'restored source is not GREEN'
    print(f'[afs2-rust] {len(reds)} RED controls ({", ".join(reds)}); byte-exact restore; GREEN', flush=True)


if __name__ == '__main__':
    main(red='--no-red' not in sys.argv)
