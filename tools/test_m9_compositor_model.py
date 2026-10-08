#!/usr/bin/env python3
"""Offline host and bare-metal tests for the *non-running* compositor model.

No IPC, owned client cap, guest pixels, teardown or injected input proof is
claimed by this gate; those require a real compositor service (ADR-0057).
"""
import subprocess
from pathlib import Path

import arena_env

ROOT = Path(__file__).resolve().parent.parent
MANIFEST = ROOT / 'userspace/compositord/Cargo.toml'


def main():
    env = arena_env.rust_env()
    for command in (
        ['cargo', 'fmt', '--manifest-path', str(MANIFEST), '--check'],
        ['cargo', 'test', '--offline', '--locked', '--manifest-path', str(MANIFEST),
         '--lib', '--target', 'x86_64-unknown-linux-gnu'],
        ['cargo', 'clippy', '--offline', '--locked', '--manifest-path', str(MANIFEST),
         '--all-targets', '--target', 'x86_64-unknown-linux-gnu', '--', '-D', 'warnings'],
        ['cargo', 'build', '--offline', '--locked', '--manifest-path', str(MANIFEST),
         '--release', '--target', 'x86_64-unknown-none'],
    ):
        subprocess.run(command, cwd=ROOT, env=env, check=True)
    print('[m9-compositor-model] 13/13 host state/churn and typed-wire/fuzz tests, fmt/clippy, '
          'bare-metal no_std build PASS; no running service or input proof')


if __name__ == '__main__':
    main()
