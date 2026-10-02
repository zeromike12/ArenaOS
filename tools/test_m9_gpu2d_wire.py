#!/usr/bin/env python3
"""Offline virtio-gpu 2D codec tests; NO actual device/service proof."""
import subprocess
from pathlib import Path
import arena_env

ROOT = Path(__file__).resolve().parent.parent
MANIFEST = ROOT / 'userspace/gpu2d/Cargo.toml'


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
    print('[m9-gpu2d-wire] 4/4 strict wire format/refusal/fuzz tests, fmt/clippy, '
          'bare-metal no_std build PASS; device transport/guest pixels pending')


if __name__ == '__main__':
    main()
