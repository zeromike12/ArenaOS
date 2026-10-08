#!/usr/bin/env python3
"""Isolated phase11-complete extraction-tool smoke; deliberately no 100/100 claim.

Stages exactly what tools/phase11_checkpoint_bundle.py ships (the 72 MiB
disk template with a blank AFS2 region included) and boots it with an
isolated Python: the first boot formats AFS2, imports AFS1 and brings up
the desktop file service and surface. The strict archival gate must refuse
the unqualified image.
"""
import hashlib
import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path
import afs1
import arena_env
import mtest
from phase11_checkpoint_bundle import SCRIPTS

ROOT = arena_env.REPO_ROOT
BUILD = arena_env.build_dir()


def main():
    esp = mtest.build('m11-archive-preflight', desktop=True)
    with tempfile.TemporaryDirectory(prefix='arena-independent-preflight11-') as directory:
        stage = Path(directory)
        shutil.copyfile(esp, stage / 'arena-esp.img')
        shutil.copyfile(BUILD / 'arena-boot.efi', stage / 'arena-boot.efi')
        shutil.copyfile(arena_env.ovmf_code(), stage / 'edk2-x86_64-code.fd')
        shutil.copyfile(arena_env.ovmf_vars_template(), stage / 'ovmf-vars-template.img')
        template = stage / 'scratch-template.img'
        afs1.mkfs(template, arena_env.SCRATCH_MIB * 1024 * 1024 // afs1.SECTOR)
        with open(template, 'r+b') as f:
            f.truncate(arena_env.AFS2_DISK_MIB * 1024 * 1024)
        for script in SCRIPTS:
            shutil.copyfile(ROOT / 'tools' / script, stage / script)
        (stage / 'sha256sums.txt').write_text(''.join(
            f'{hashlib.sha256(p.read_bytes()).hexdigest()}  {p.name}\n' for p in sorted(stage.iterdir())))
        env = os.environ.copy()
        env.pop('PYTHONPATH', None)
        env['ARENA_EXTRACTED_EVIDENCE'] = str(BUILD / 'phase11-independent-preflight')
        runner = ('import runpy,sys;sys.path.insert(0,sys.argv.pop(1));'
                  'runpy.run_path(sys.path[0]+"/phase11_archive_boot.py",run_name="__main__")')
        command = [sys.executable, '-I', '-c', runner, str(stage)]
        strict = subprocess.run(command, cwd=stage, env=env, text=True, capture_output=True)
        assert strict.returncode != 0 and 'EXTRACTED PHASE11 BOOT FAILED:' in strict.stderr, strict.stderr
        result = subprocess.run(command + ['--unqualified-smoke'], cwd=stage, env=env, text=True, capture_output=True)
        (BUILD / 'm11-archive-preflight.log').write_text(result.stdout + result.stderr)
        assert result.returncode == 0 and 'UNQUALIFIED EXTRACTED PHASE11 PREFLIGHT PASS:' in result.stdout, \
            result.stdout + result.stderr
        assert 'EXTRACTED PHASE11 PIXELS PASS:' not in result.stdout
        # The Phase-11 step ran on the same live desktop (AFS2 read back).
        assert 'PHASE11 DESKTOP-MENU-FOLDER FILES-OPEN FILES-NEW-FOLDER FILES-CLOSE' in result.stdout, result.stdout
    print('[m11-archive-preflight] independently staged phase11-complete tools, firmware and 72 MiB disk template; '
          'first boot formats AFS2 and brings up the desktop file service and surface; real two-spawn/retire '
          'graphical workflow, then desktop-menu folder, Files opened from its icon, Ctrl+Shift+N folder and F8 '
          'retirement read back from AFS2 PASS; strict qualification refuses absent 100/100 receipt')


if __name__ == '__main__':
    main()
