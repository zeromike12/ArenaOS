#!/usr/bin/env python3
"""Boot an extracted preview on stock Linux QEMU TCG; not a Windows test."""
import os
import re
import shutil
import subprocess
import sys
import tempfile
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / 'tools'))
import arena_env
import qmp
from PIL import Image

package = Path(sys.argv[1]).resolve()
evidence = Path(sys.argv[2]).resolve()
evidence.mkdir(parents=True, exist_ok=True)
with tempfile.TemporaryDirectory(prefix='p16-preview-') as tmp:
    tmp = Path(tmp)
    shutil.copyfile(package / 'preview-data-template.img', tmp / 'data.img')
    shutil.copyfile(package / 'ovmf-vars-template.fd', tmp / 'vars.fd')
    log = evidence / 'linux-tcg-serial.log'
    qpath = tmp / 'qmp.sock'
    peer = subprocess.Popen([sys.executable, '-u', str(ROOT / 'tools/network_fixture.py'), str(evidence / 'linux-tcp.log'), str(evidence / 'linux-dns.log')], stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
    proc = None
    try:
        assert peer.stdout.readline().strip() == 'READY', 'host network peers failed to bind'
        args = arena_env.qemu_cmd() + arena_env.qemu_data_args() + [
            '-name', 'ArenaOS Phase 16 DIAGNOSTIC DESKTOP PREVIEW - XF NOT RUN', '-accel', 'tcg', '-M', 'q35', '-m', '512M', '-cpu', 'qemu64,+nx,+smep,+smap',
            '-drive', f'if=pflash,format=raw,readonly=on,file={package / "ovmf-code.fd"}',
            '-drive', f'if=pflash,format=raw,file={tmp / "vars.fd"}',
            '-drive', f'if=none,id=esp0,format=raw,snapshot=on,file={package / "phase16-desktop-preview-esp.img"}',
            '-device', 'ide-hd,drive=esp0,bus=ide.0,bootindex=0',
            '-drive', f'if=none,id=scr0,format=raw,file={tmp / "data.img"}', '-device', 'virtio-blk-pci,drive=scr0',
            '-netdev', 'user,id=net0', '-device', 'virtio-net-pci,netdev=net0', '-device', 'virtio-rng-pci',
            '-device', 'virtio-keyboard-pci', '-device', 'virtio-tablet-pci', '-display', 'none',
            '-qmp', f'unix:{qpath},server=on,wait=off', '-serial', 'stdio', '-no-reboot']
        with log.open('wb') as out, (evidence / 'linux-qemu-stderr.txt').open('wb') as err:
            proc = subprocess.Popen(args, stdin=subprocess.PIPE, stdout=out, stderr=err)
            def wait(needle, seconds=240):
                until = time.monotonic() + seconds
                while time.monotonic() < until:
                    text = log.read_bytes()
                    if needle in text: return
                    if proc.poll() is not None: raise RuntimeError(f'QEMU exited {proc.returncode} waiting for {needle!r}')
                    time.sleep(.1)
                raise TimeoutError(f'waiting for {needle!r}')
            wait(b'inputd: virtio-input ready')
            c = qmp.Qmp(str(qpath))
            c.type_text('arena', gap_s=.08)
            wait(b'm15: DIAGNOSTIC PASS (5/5; XF NOT RUN)')
            wait(b'[desktop] real desktop frame presented')
            wait(b'servicemgr: packaged READY')
            wait(b'packaged: protected installed-app registry ready entries=')
            wait(b'arena>')
            def shot(path):
                c.command('screendump', filename=str(path), format='ppm')
                return Image.open(path).convert('RGB')
            before = shot(tmp / 'before.ppm')
            def point(x, y, down):
                events = [{'type':'abs','data':{'axis':'x','value':(x*32767+799)//799}}, {'type':'abs','data':{'axis':'y','value':(y*32767+599)//599}}, {'type':'btn','data':{'button':'left','down':down}}]
                c.command('input-send-event', events=events)
            point(545,570,True); point(545,570,False)
            wait(b'[desktop] real application spawned;')
            deadline = time.monotonic() + 15
            while True:
                after = shot(tmp / 'after.ppm')
                if after.crop((100,100,200,180)).tobytes() != before.crop((100,100,200,180)).tobytes(): break
                if time.monotonic() > deadline: raise TimeoutError('tablet click did not change guest pixels')
                time.sleep(.1)
            time.sleep(1)
            shot(tmp / 'final.ppm').save(evidence / 'linux-tcg-desktop.png')
            c.close()
            proc.stdin.write(b'shutdown\r'); proc.stdin.flush()
            rc = proc.wait(timeout=45)
            if rc != 0: raise RuntimeError(f'QEMU shutdown exit {rc}')
            wait(b'halting via UEFI ResetSystem(shutdown)', 1)
            text = log.read_bytes()
            assert b'm15: RESULT PASS' not in text, 'diagnostic image emitted a strict m15 marker'
            assert b'TCP_FIXTURE_PASS' in (evidence / 'linux-tcp.log').read_bytes()
            assert b'DNS_FIXTURE_QUERY' in (evidence / 'linux-dns.log').read_bytes()
        print('Linux stock TCG: marker, desktop, packaged READY, tablet dock launch/pixels, shutdown exit 0: PASS')
    finally:
        if proc and proc.poll() is None: proc.kill(); proc.wait()
        peer.terminate()
        try: peer.wait(timeout=3)
        except subprocess.TimeoutExpired: peer.kill(); peer.wait()
