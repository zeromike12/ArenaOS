#!/usr/bin/env python3
"""Independently boot an extracted Phase-9 archive and assert real QMP pixels.

Only Python's standard library, QEMU and the named files in the extracted
archive are required. No checkout, build tree, source import or signing key.
The test UDP/TCP peers are included with the archive; they are not external
public DNS. This is a boot/graphics witness, NOT an arbitrary DNS claim.
"""
import hashlib
import os
import shutil
import subprocess
import sys
import tempfile
import time
from pathlib import Path

from check_phase9_pixels import capture
import qmp

ROOT = Path(__file__).resolve().parent


def verified_inputs() -> str:
    manifest = (ROOT / 'sha256sums.txt').read_text().splitlines()
    for line in manifest:
        digest, name = line.split(maxsplit=1)
        path = ROOT / name
        if path.resolve().parent != ROOT or hashlib.sha256(path.read_bytes()).hexdigest() != digest:
            raise ValueError(f'extracted archive entry hash mismatch: {name}')
    efi_sha = hashlib.sha256((ROOT / 'arena-boot.efi').read_bytes()).hexdigest()
    if (ROOT / 'stability-receipt.txt').read_text().split() != [efi_sha, '100/100']:
        raise ValueError('qualified 100/100 receipt does not name extracted EFI')
    return efi_sha


def qemu_cmd() -> list[str]:
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


def boot() -> None:
    sha = verified_inputs()
    with tempfile.TemporaryDirectory(prefix='arena-phase9-extracted-') as tmp:
        work = Path(tmp)
        vars_image, scratch = work / 'vars.img', work / 'scratch.img'
        shutil.copyfile(ROOT / 'ovmf-vars-template.img', vars_image)
        shutil.copyfile(ROOT / 'scratch-template.img', scratch)
        serial = work / 'serial.log'
        sock = work / 'qmp.sock'
        image, receipt = work / 'screen.ppm', work / 'pixels.txt'
        tcp_log, dns_log = work / 'tcp.log', work / 'dns.log'
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
                # QMP 'q' reached both the client and the console. Erase it
                # before shutdown; a forged shell command cannot count green.
                guest.stdin.write(b'\x08shutdown\r')
                guest.stdin.flush()
                rc = guest.wait(timeout=60)
            text = serial.read_text(errors='replace')
            if (rc != 0 or '[arena ERROR halt]' in text or 'PANIC' in text
                    or 'm7: RESULT PASS (2/2)' not in text
                    or '[window_b] real key pixel painted' not in text
                    or '[window_a] forged input token refused' not in text
                    or 'halting via UEFI ResetSystem(shutdown)' not in text
                    or dns_log.read_text().count('DNS_FIXTURE_QUERY ') != 3
                    or 'TCP_FIXTURE_PASS request=arena-tcp bytes=200 eof=True' not in tcp_log.read_text()):
                raise RuntimeError(f'extracted image boot failed semantic gate: rc={rc}, '
                                   f'serial={serial}, dns={dns_log}, tcp={tcp_log}')
            print(f'EXTRACTED PHASE9 PIXELS PASS: EFI SHA-256 {sha}; {receipt.read_text().strip()}')
        finally:
            if guest is not None and guest.poll() is None:
                guest.kill()
                guest.wait()
            actor.terminate()
            try:
                actor.wait(timeout=3)
            except subprocess.TimeoutExpired:
                actor.kill()
                actor.wait()


if __name__ == '__main__':
    try:
        boot()
    except (OSError, ValueError, RuntimeError, subprocess.TimeoutExpired) as exc:
        sys.exit(f'EXTRACTED PHASE9 BOOT FAILED: {exc}')
