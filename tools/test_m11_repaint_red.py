#!/usr/bin/env python3
"""Phase 11.1 RED/GREEN controls for partial repaint and partial publication.

Each control removes one production mechanism from the real desktop library
source and requires the host equivalence tests (partial repaint/composition
versus a full reference redraw, pixel for pixel) to fail with the named
test. A compile error never counts as RED. Source is restored byte-exactly
and the complete library test suite must then pass (GREEN).
"""
import subprocess
from pathlib import Path
import arena_env

ROOT = arena_env.REPO_ROOT
CRATE = ROOT / 'userspace/desktop'
SCENE = CRATE / 'src/apps/scene.rs'
COMPOSE = CRATE / 'src/compose.rs'
CONTROLS = [
    # Old caret: a row's key forgets where the caret sits in it, so moving
    # the caret along a row leaves the old caret drawn.
    (SCENE, b'                        rows[row - self.top].u64(0xC0).u64(column as u64);\n',
     b'', 'old-caret', 'editor_partial_repaint_equals_full_redraw'),
    # Newly exposed region: transcript row keys ignore the scrollback
    # offset, so lines scrolled into view are never repainted.
    (SCENE, b'let start = t.count.saturating_sub(l::TERMINAL_ROWS).saturating_sub(self.top);',
     b'let start = t.count.saturating_sub(l::TERMINAL_ROWS);',
     'exposed-rows', 'terminal_partial_repaint_equals_full_redraw'),
    # Incomplete client merge: joining two dirty runs keeps only the first.
    (SCENE, b'        runs[best - 1].1 = runs[best].1;',
     b'        runs[best - 1].1 = runs[best - 1].1;',
     'client-merge', 'dirty_runs_merge_and_stay_bounded'),
    # Incomplete compositor merge: overlapping published regions merge to
    # the first rectangle instead of their union.
    (COMPOSE, b'                    cur = region_union(self.r[i], cur);',
     b'                    cur = self.r[i];',
     'region-merge', 'damage_composition_matches_full_redraw'),
    # Published regions damaged at the wrong screen place (window offset lost).
    (COMPOSE, b'                        x: b.x + i32::from(r[0]),',
     b'                        x: i32::from(r[0]),',
     'region-offset', 'damage_composition_matches_full_redraw'),
]


def cargo_test():
    return subprocess.run(
        ['cargo', 'test', '--offline', '--lib', '--target', 'x86_64-unknown-linux-gnu'],
        cwd=CRATE, env=arena_env.rust_env(), capture_output=True, text=True)


def main():
    sources = {p: p.read_bytes() for p in {c[0] for c in CONTROLS}}
    for path, before, _, label, _ in CONTROLS:
        assert sources[path].count(before) == 1, (label, sources[path].count(before))
    reds = []
    try:
        for path, before, after, label, test in CONTROLS:
            original = sources[path]
            path.write_bytes(original.replace(before, after, 1))
            try:
                r = cargo_test()
                out = r.stdout + r.stderr
                assert 'error[' not in out, f'{label}: mutant did not compile (never RED)'
                assert r.returncode != 0 and f'{test} ... FAILED' in out, \
                    f'{label}: {test} did not fail'
                reds.append(label)
                print(f'[m11-repaint-red] {label}: {test} FAILED → RED PASS', flush=True)
            finally:
                path.write_bytes(original)
    finally:
        for p, data in sources.items():
            p.write_bytes(data)
    assert all(p.read_bytes() == d for p, d in sources.items())
    r = cargo_test()
    assert r.returncode == 0, r.stdout[-3000:] + r.stderr[-3000:]
    print(f'[m11-repaint-red] {len(reds)} RED controls ({", ".join(reds)}); '
          'byte-exact restore; library suite GREEN', flush=True)


if __name__ == '__main__':
    main()
