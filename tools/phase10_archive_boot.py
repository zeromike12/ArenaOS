#!/usr/bin/env python3
"""Independently boot an extracted Phase-10 archive and assert real QMP pixels.

Only Python's standard library, QEMU and the named files in the extracted
archive are required. No checkout, build tree, source import or signing key.
The test UDP/TCP peers are included with the archive; they are not external
public DNS. This is a boot/graphics witness, NOT an arbitrary DNS claim.
"""
import hashlib
import os
import shlex
import shutil
import subprocess
import sys
import tempfile
import time
from pathlib import Path

from check_phase10_pixels import capture
import qmp

ROOT = Path(__file__).resolve().parent


def verified_inputs(qualified: bool = True) -> str:
    manifest = (ROOT / 'sha256sums.txt').read_text().splitlines()
    for line in manifest:
        digest, name = line.split(maxsplit=1)
        path = ROOT / name
        if path.resolve().parent != ROOT or hashlib.sha256(path.read_bytes()).hexdigest() != digest:
            raise ValueError(f'extracted archive entry hash mismatch: {name}')
    efi_sha = hashlib.sha256((ROOT / 'arena-boot.efi').read_bytes()).hexdigest()
    if qualified and (ROOT / 'stability-receipt.txt').read_text().split() != [efi_sha, '100/100']:
        raise ValueError('qualified 100/100 receipt does not name extracted EFI')
    return efi_sha


def qemu_cmd() -> list[str]:
    configured = os.environ.get('ARENA_QEMU')
    if configured:
        return shlex.split(configured)
    executable = shutil.which('qemu-system-x86_64')
    if executable:
        return [executable]
    portable = Path('/opt/qemu/bin/qemu-system-x86_64')
    loader = Path('/opt/musl/lib/libc.so')
    if portable.exists() and loader.exists():
        return ['env', 'LD_LIBRARY_PATH=/opt/qemu/lib', str(loader), str(portable),
                '-L', '/opt/qemu/share/qemu']
    raise FileNotFoundError('install QEMU qemu-system-x86_64 before booting this archive')


def type_historical_input_fixture(sock: Path, serial: Path) -> None:
    # The historical M6 service-mode inputd blocks until five *real*
    # virtio-keyboard events arrive. Without this early step an extracted
    # image can never reach the compositor, regardless of perfect pixels.
    deadline = time.monotonic() + 30
    while time.monotonic() < deadline:
        if serial.is_file() and b'inputd: virtio-input ready' in serial.read_bytes():
            break
        time.sleep(0.05)
    else:
        raise TimeoutError('historical M6 keyboard driver never became ready')
    conn = qmp.Qmp(str(sock), connect_timeout_s=4)
    try:
        for ch in 'arena':
            conn.key(ch)
    finally:
        conn.close()


def boot(qualified: bool = True) -> None:
    sha = verified_inputs(qualified)
    with tempfile.TemporaryDirectory(prefix='arena-phase10-extracted-') as tmp:
        work = Path(tmp)
        vars_image, scratch = work / 'vars.img', work / 'scratch.img'
        shutil.copyfile(ROOT / 'ovmf-vars-template.img', vars_image)
        shutil.copyfile(ROOT / 'scratch-template.img', scratch)
        serial = work / 'serial.log'
        sock = work / 'qmp.sock'
        image, receipt = work / 'screen.ppm', work / 'pixels.txt'
        tcp_log, dns_log = work / 'tcp.log', work / 'dns.log'
        console_log, console_sock = work / 'console.log', work / 'console.sock'
        console = subprocess.Popen([sys.executable, '-u', str(ROOT / 'vcon.py'),
                                    str(console_sock), 'contest: hello from ArenaOS',
                                    'host-says-hello\n', '60', str(console_log)], cwd=ROOT)
        actor = subprocess.Popen([sys.executable, '-u', str(ROOT / 'network_fixture.py'),
                                  str(tcp_log), str(dns_log)], cwd=ROOT,
                                 stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        guest = None
        try:
            if actor.stdout is None or actor.stdout.readline().strip() != b'READY':
                raise RuntimeError('archive-owned TCP/UDP fixture failed to bind')
            with serial.open('wb') as serial_fd, (work / 'qemu.err').open('wb') as err_fd:
                cmd = qemu_cmd() + [
                    '-M', 'q35', '-m', '512M', '-cpu', 'qemu64,+nx,+smep,+smap',
                    '-boot', 'order=c',
                    '-drive', f'if=pflash,format=raw,readonly=on,file={ROOT / "edk2-x86_64-code.fd"}',
                    '-drive', f'if=pflash,format=raw,file={vars_image}',
                    '-drive', f'format=raw,file={ROOT / "arena-esp.img"}',
                    '-drive', f'file={scratch},format=raw,if=none,id=scr0',
                    '-device', 'virtio-blk-pci,drive=scr0',
                    '-netdev', 'user,id=net0', '-device', 'virtio-net-pci,netdev=net0',
                    '-device', 'virtio-rng-pci', '-device', 'virtio-keyboard-pci',
                    '-device', 'virtio-tablet-pci',
                    '-device', 'virtio-serial-pci,max_ports=1',
                    '-chardev', f'socket,id=vc0,path={console_sock},server=on,wait=on',
                    '-device', 'virtconsole,chardev=vc0',
                    '-qmp', f'unix:{sock},server=on,wait=off', '-display', 'none',
                    '-chardev', 'stdio,id=con0,signal=off', '-serial', 'chardev:con0',
                    '-no-reboot',
                ]
                guest = subprocess.Popen(cmd, cwd=work, stdin=subprocess.PIPE,
                                         stdout=serial_fd, stderr=err_fd)
                type_historical_input_fixture(sock, serial)
                capture(sock, serial, image, receipt, 60)
                if guest.stdin is None:
                    raise RuntimeError('no QEMU serial feeder')
                # Graphical input is routed independently of the serial shell.
                guest.stdin.write(b'shutdown\r')
                guest.stdin.flush()
                rc = guest.wait(timeout=60)
            text = serial.read_text(errors='replace')
            if (rc != 0 or '[arena ERROR halt]' in text or 'PANIC' in text
                    or 'm7: RESULT PASS (2/2)' not in text
                    or 'm6: RESULT PASS (6/6)' not in text
                    or 'contest: PASS — the port carried bytes BOTH ways:' not in text
                    or 'contest: hello from ArenaOS' not in console_log.read_text(errors='replace')
                    or 'servicemgr: full fixture notification budget 25/25; twenty-sixth refused' not in text
                    or text.count('[desktop] real application spawned;') != 2
                    or text.count('[desktop] application retired:') != 2
                    or 'halting via UEFI ResetSystem(shutdown)' not in text
                    or dns_log.read_text().count('DNS_FIXTURE_QUERY ') != 3
                    or 'TCP_FIXTURE_PASS request=arena-tcp bytes=200 eof=True' not in tcp_log.read_text()):
                raise RuntimeError(f'extracted image boot failed semantic gate: rc={rc}, '
                                   f'serial={serial}, dns={dns_log}, tcp={tcp_log}')
            evidence = os.environ.get('ARENA_EXTRACTED_EVIDENCE')
            if evidence:
                target = Path(evidence).resolve(); target.mkdir(parents=True, exist_ok=True)
                for p in (serial, image, receipt, tcp_log, dns_log, console_log):
                    shutil.copyfile(p, target / p.name)
            prefix='EXTRACTED PHASE10 PIXELS PASS' if qualified else 'UNQUALIFIED EXTRACTED PHASE10 PREFLIGHT PASS'
            print(f'{prefix}: EFI SHA-256 {sha}; {receipt.read_text().strip()}')
        finally:
            if guest is not None and guest.poll() is None:
                guest.kill()
                guest.wait()
            actor.terminate()
            console.terminate()
            try:
                console.wait(timeout=3)
            except subprocess.TimeoutExpired:
                console.kill()
                console.wait()
            try:
                actor.wait(timeout=3)
            except subprocess.TimeoutExpired:
                actor.kill()
                actor.wait()


if __name__ == '__main__':
    try:
        if sys.argv[1:] not in ([], ['--unqualified-smoke']):
            raise ValueError('usage: phase10_archive_boot.py [--unqualified-smoke]')
        boot(qualified=not sys.argv[1:])
    except (OSError, ValueError, RuntimeError, subprocess.TimeoutExpired) as exc:
        sys.exit(f'EXTRACTED PHASE10 BOOT FAILED: {exc}')
