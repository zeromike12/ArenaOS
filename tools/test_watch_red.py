#!/usr/bin/env python3
"""filesd directory watches: host GREEN plus RED controls (ADR-0079).

Each mutant must make its specific host test fail; the source is restored
byte-exactly and GREEN again afterwards.
* stale-generation: a watch matches by directory index alone, so a removed
  directory's index reused by a new directory signals the old watcher.
* rename-parent: a rename reports only its source directory, so a watcher
  of the destination is never told.
"""
import subprocess
import arena_env

ROOT = arena_env.REPO_ROOT
SOURCE = ROOT / 'userspace/filesd/src/watch.rs'
BIN = arena_env.build_dir() / 'filesd-watch-tests'
MUTANTS = (
    ('stale-generation', b'pub fn same(a: u64, b: u64) -> bool {\n    a == b\n}',
     b'pub fn same(a: u64, b: u64) -> bool {\n    a as u32 == b as u32\n}',
     'a_reused_directory_index_never_signals_the_old_watcher'),
    ('rename-parent', b'        Some(dst) if !same(dst, dir) => [Some(dir), Some(dst)],',
     b'        Some(dst) if !same(dst, dir) => [Some(dir), None],',
     'a_rename_changes_both_parents'),
)


def run():
    env = arena_env.rust_env()
    subprocess.run(['rustc', '--test', '--edition', '2024', str(SOURCE), '-o', str(BIN)], env=env, check=True)
    return subprocess.run([str(BIN)], capture_output=True, text=True)


def main():
    original = SOURCE.read_bytes()
    green = run()
    assert green.returncode == 0, green.stdout
    try:
        for label, before, after, test in MUTANTS:
            assert original.count(before) == 1, label
            SOURCE.write_bytes(original.replace(before, after, 1))
            red = run()
            assert red.returncode != 0 and f'{test} ... FAILED' in red.stdout, (label, red.stdout)
            print(f'[watch-red] {label}: {test} FAILED -> RED PASS', flush=True)
            SOURCE.write_bytes(original)
    finally:
        SOURCE.write_bytes(original)
    assert SOURCE.read_bytes() == original
    again = run()
    assert again.returncode == 0, again.stdout
    print('[watch-red] directory watch table GREEN; stale-generation and rename-parent RED; source restored byte-exact PASS')


if __name__ == '__main__':
    main()
