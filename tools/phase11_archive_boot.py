#!/usr/bin/env python3
"""Independently boot an extracted Phase-11 archive and assert real QMP pixels.

The Phase-10 standalone witness (real spawns, owned key raster, exact drag,
retirement, network and console fixtures) on the Phase-11 image, with its
72 MiB disk: filesd formats the AFS2 region and imports AFS1 on this boot,
and the desktop must come up with its file service and desktop surface.
Then, on the same live desktop (`check_phase11_files.py`), the desktop
menu creates a folder on AFS2 and draws its icon, double-clicking it opens
a real Files process there, Shift+N creates a folder inside it, and F8
retires Files; the disk's AFS2 region is read back on the host.
Only Python's standard library, QEMU and the extracted files are used.
"""
import subprocess
import sys

import check_phase11_files
import phase10_archive_boot as witness

PHASE11 = (
    'filesd: AFS2 formatted; AFS1 import complete',
    'filesd: AFS2 mounted',
    '[desktop] AFS2 file service online',
    '[desktop] desktop surface shows /Users/user/Desktop',
    'm11:test:badged_endpoint: PASS',
)

if __name__ == '__main__':
    try:
        if sys.argv[1:] not in ([], ['--unqualified-smoke']):
            raise ValueError('usage: phase11_archive_boot.py [--unqualified-smoke]')
        witness.boot(qualified=not sys.argv[1:], extra_markers=PHASE11, label='PHASE11',
                     after=lambda sock, serial, image, disk: check_phase11_files.capture(
                         sock, serial, image, disk, 90),
                     launches=3)
    except (OSError, ValueError, RuntimeError, subprocess.TimeoutExpired) as exc:
        sys.exit(f'EXTRACTED PHASE11 BOOT FAILED: {exc}')
