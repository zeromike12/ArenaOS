#!/usr/bin/env python3
"""Create the qualified phase11-complete archive and boot it independently.

Inputs: the complete-suite log and the 100-boot stability log, both of the
exact source commit and EFI being archived. The archive holds the EFI and
ESP, firmware, a 72 MiB disk template (AFS1 formatted, AFS2 region blank:
filesd formats it and imports AFS1 on the first boot), the standalone
witness scripts, the Phase-11 documents and ADRs, the logs and the RED
evidence. It is then extracted into a fresh directory and booted with an
isolated Python that sees only the extracted files.
"""
import argparse
import hashlib
import json
import os
import re
import shutil
import subprocess
import sys
import tarfile
import tempfile
from pathlib import Path
import afs1
import arena_env
from pyfatfs.PyFatFS import PyFatFS

ROOT = arena_env.REPO_ROOT
BUILD = arena_env.build_dir()
NAME = 'phase11-complete'
SCRIPTS = ('phase11_archive_boot.py', 'phase10_archive_boot.py', 'check_phase10_pixels.py',
           'check_phase11_files.py', 'afs2.py', 'qmp.py',
           'network_fixture.py', 'tcp_fixture.py', 'udp_dns_fixture.py', 'vcon.py')
DOCS = ('docs/phase11/PLAN.md', 'docs/phase11/PROGRESS.md', 'docs/phase11/FINAL-REPORT.md') + tuple(
    f'docs/adr/{p.name}' for p in sorted((ROOT / 'docs/adr').glob('007[1-9]-*.md')))
# Host blocks of tools/run_tests.sh that count as suites besides test_m*.py.
HOST_SUITES = 14
REQUIRED = (
    '[m11-explorer]', '[m11-desk]', '[m11-files]', '[m11-afs2]', '[m11-wm]',
    '[m10-files] AFS2 explorer:', '[m10-boundaries-red] 12 real production RED controls;',
    '[m10-dynamic]', '[m10-service-death] real compositor death answers in-flight input CALL',
    '[m10-handoff] actual desktop, all six real apps',
)


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('suite_log', type=Path)
    parser.add_argument('stability_log', type=Path)
    parser.add_argument('--source', help='qualified source commit (before documentation-only commits)')
    args = parser.parse_args()
    commit = subprocess.check_output(['git', 'rev-parse', args.source or 'HEAD'], cwd=ROOT, text=True).strip()
    assert not subprocess.check_output(['git', 'status', '--porcelain'], cwd=ROOT), 'commit everything first'
    changed = subprocess.check_output(['git', 'diff', '--name-only', commit, 'HEAD'], cwd=ROOT, text=True).splitlines()
    assert all(p.startswith(('docs/phase11/', 'releases/checkpoints/' + NAME + '/')) for p in changed), \
        'production, tests or tools differ from the qualified source'
    log = args.suite_log.read_text()
    count = len(list((ROOT / 'tools').glob('test_m*.py'))) + HOST_SUITES
    for marker in (f'ALL TESTS PASSED ({count} test suites)', f'QUALIFICATION SOURCE COMMIT: {commit}',
                   'QUALIFICATION SOURCE CLEAN: yes', f'QUALIFICATION SOURCE END: {commit}') + REQUIRED:
        assert marker in log, f'suite log lacks: {marker}'
    assert (BUILD / 'graphics-profile.txt').read_text().strip() == 'desktop'
    efi = sha(BUILD / 'arena-boot.efi')
    assert (BUILD / 'stability-receipt.txt').read_text().split() == [efi, '100/100']
    fs = PyFatFS(str(BUILD / 'arena-esp.img'), read_only=True)
    try:
        assert fs.getbytes('/EFI/BOOT/BOOTX64.EFI') == (BUILD / 'arena-boot.efi').read_bytes()
    finally:
        fs.close()
    stability = args.stability_log.read_text()
    assert efi in stability and '100/100' in stability and 'FAIL' not in stability, 'invalid stability attempt'
    destination = ROOT / 'releases/checkpoints' / NAME
    destination.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='phase11-bundle-', dir=BUILD) as tmp:
        stage = Path(tmp)
        for filename in ('arena-boot.efi', 'arena-esp.img', 'stability-receipt.txt'):
            shutil.copyfile(BUILD / filename, stage / filename)
        for script in SCRIPTS:
            shutil.copyfile(ROOT / 'tools' / script, stage / script)
        for doc in DOCS:
            shutil.copyfile(ROOT / doc, stage / Path(doc).name)
        shutil.copyfile(arena_env.ovmf_code(), stage / 'edk2-x86_64-code.fd')
        shutil.copyfile(arena_env.ovmf_vars_template(), stage / 'ovmf-vars-template.img')
        template = stage / 'scratch-template.img'
        afs1.mkfs(template, arena_env.SCRATCH_MIB * 1024 * 1024 // afs1.SECTOR)
        with open(template, 'r+b') as f:
            f.truncate(arena_env.AFS2_DISK_MIB * 1024 * 1024)
        shutil.copyfile(args.suite_log, stage / 'full-suite.log')
        shutil.copyfile(args.stability_log, stage / '100-boot.log')
        for proof in sorted(BUILD.glob('m1[01]-*-red.log')):
            shutil.copyfile(proof, stage / ('evidence-' + proof.name))
        receipts = re.findall(r'\[m10-apps\].*baseline=.*', log)
        (stage / 'QUALIFICATION.json').write_text(json.dumps(dict(
            checkpoint=NAME, source_commit=commit, branch='arena/phase11-desktop-maturity', suite_count=count,
            efi_sha256=efi, stability='100/100', resource_receipts=receipts), indent=2) + '\n')
        (stage / 'RUNNING.md').write_text('''# ArenaOS phase11-complete

Verify the outer archive checksum and the extracted `sha256sums.txt`, then
install QEMU and Python 3 and run `python3 phase11_archive_boot.py` here.
No repository or build tree is needed. The script copies fresh OVMF
variables and the 72 MiB disk template; on that boot filesd formats the
AFS2 region and imports AFS1. It binds the included virtual-network test
peers, launches real graphical applications, injects keyboard and tablet
input, checks exact owned raster movement and process retirement, requires
the file service and the desktop surface online, then (same boot) creates a
folder from the desktop menu, opens it in a real Files process by
double-clicking its icon, creates a folder inside it with Shift+N, closes
Files with F8, and reads both folders back from the disk's AFS2 region on
the host. It then shuts down through the serial shell. Set ARENA_QEMU to a QEMU invocation if it is not on PATH.

The boot fixture still requires the virtual keyboard input `arena` before
the desktop starts (the historical inputd service mode); the script types it.
See FINAL-REPORT.md for what Phase 11 delivered and what it did not.
''')
        files = sorted(p.name for p in stage.iterdir())
        (stage / 'sha256sums.txt').write_text(''.join(f'{sha(stage / n)}  {n}\n' for n in files))
        files.append('sha256sums.txt')
        archive = destination / f'arenaos-{NAME}-qemu-x86_64.tar.gz'
        with tarfile.open(archive, 'w:gz') as tar:
            for name in files:
                tar.add(stage / name, arcname=name)
        digest = sha(archive)
        (destination / (archive.name + '.sha256')).write_text(f'{digest}  {archive.name}\n')
        with tempfile.TemporaryDirectory(prefix='phase11-independent-', dir=BUILD) as extracted:
            target = Path(extracted)
            with tarfile.open(archive, 'r:gz') as tar:
                assert {m.name for m in tar} == set(files) and all(m.isfile() for m in tar)
                for member in tar:
                    stream = tar.extractfile(member)
                    assert stream is not None
                    (target / member.name).write_bytes(stream.read())
            env = os.environ.copy()
            env.pop('PYTHONPATH', None)
            env['ARENA_EXTRACTED_EVIDENCE'] = str(destination / 'extracted-evidence')
            runner = ('import runpy,sys;sys.path.insert(0,sys.argv[1]);'
                      'runpy.run_path(sys.argv[1]+"/phase11_archive_boot.py",run_name="__main__")')
            result = subprocess.run([sys.executable, '-I', '-c', runner, str(target)], cwd=target, env=env,
                                    text=True, capture_output=True)
            (destination / 'independent-extracted-boot.log').write_text(result.stdout + result.stderr)
            assert result.returncode == 0 and 'EXTRACTED PHASE11 PIXELS PASS:' in result.stdout, \
                result.stdout + result.stderr
    print(f'{NAME}: source={commit} suites={count}/{count} EFI={efi} stability=100/100 archive={digest}; '
          'independently extracted graphical boot PASS')


if __name__ == '__main__':
    main()
