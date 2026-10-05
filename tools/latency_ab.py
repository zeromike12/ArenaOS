#!/usr/bin/env python3
"""Same-host, interleaved latency comparison of two source commits.

Builds the production desktop image of each commit in its own worktree
and runs `profile_desktop.py --production` on them alternately (A, B, A,
B, ...), so host drift affects both equally. Reports, per commit, every
run's pointer motion-to-photon and Terminal key-to-photon p50/p95 plus
the median over runs, and the screendump cost that bounds the method's
resolution. Measurement only; not a qualification gate.

Usage: python3 tools/latency_ab.py BASE_COMMIT NEW_COMMIT [--runs N] [--json OUT]
"""
import argparse
import json
import shutil
import statistics
import subprocess
import sys
from pathlib import Path
import arena_env

ROOT = arena_env.REPO_ROOT


def worktree(commit):
    path = arena_env.build_dir() / f'ab-{commit[:12]}'
    if not path.exists():
        subprocess.check_call(['git', 'worktree', 'add', '--detach', str(path), commit], cwd=ROOT)
    return path


def run(tree, label, out):
    # This profiler, copied into the worktree, measures that tree's own
    # production image (built into the worktree's build directory), so
    # both commits are measured by exactly the same method.
    shutil.copyfile(ROOT / 'tools/profile_desktop.py', tree / 'tools/profile_desktop_ab.py')
    subprocess.check_call([sys.executable, str(tree / 'tools/profile_desktop_ab.py'), '--production',
                           '--label', label, '--json', str(out)], cwd=tree / 'tools')
    r = json.loads(out.read_text())
    return {'pointer': r['cursor-latency']['motion_to_photon_ms'],
            'key': r['key-latency']['key_to_photon_ms']}


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('base')
    ap.add_argument('new')
    ap.add_argument('--runs', type=int, default=5)
    ap.add_argument('--json')
    args = ap.parse_args()
    commits = {name: subprocess.check_output(['git', 'rev-parse', c], cwd=ROOT, text=True).strip()
               for name, c in (('base', args.base), ('new', args.new))}
    trees = {name: worktree(c) for name, c in commits.items()}
    results = {name: [] for name in commits}
    for i in range(args.runs):
        for name in ('base', 'new'):
            out = arena_env.build_dir() / f'ab-{name}-{i}.json'
            results[name].append(run(trees[name], f'ab-{name}-{i}', out))
            print(f'[ab] {name} run {i}: {results[name][-1]}', flush=True)
    summary = {}
    for name, runs in results.items():
        summary[name] = {
            'commit': commits[name],
            'pointer_p50_median': statistics.median(r['pointer']['median'] for r in runs),
            'pointer_p95_median': statistics.median(r['pointer']['p95'] for r in runs),
            'key_p50_median': statistics.median(r['key']['median'] for r in runs),
            'key_p95_median': statistics.median(r['key']['p95'] for r in runs),
            'screendump_ms_median': statistics.median(r['pointer']['screendump_ms'] for r in runs),
            'runs': runs,
        }
        s = summary[name]
        print(f"[ab] {name} {commits[name][:12]}: pointer p50 {s['pointer_p50_median']} p95 "
              f"{s['pointer_p95_median']} ms; key p50 {s['key_p50_median']} p95 {s['key_p95_median']} ms; "
              f"screendump {s['screendump_ms_median']} ms ({len(runs)} runs)", flush=True)
    if args.json:
        Path(args.json).write_text(json.dumps(summary, indent=1) + '\n')


if __name__ == '__main__':
    main()
