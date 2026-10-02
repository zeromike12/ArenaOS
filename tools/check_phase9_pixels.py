#!/usr/bin/env python3
"""Independent QMP PPM pixel witness for ArenaOS's ring-3 GOP fallback.

One call captures one *actual guest display* after displayd's ready line;
returns a receipt only when dimension, full image length, and four spatially
distant RGB samples match the userspace renderer. A boot's serial PASS alone
cannot produce this receipt. This is a GOP smoke, not a compositor proof.
"""
import hashlib
import re
import sys
import time
from pathlib import Path
import qmp

READY = b'[displayd] ring3 GOP pixels ready (staged font v2)'
SAMPLES = (
    ((0, 0), (0x22, 0x33, 0x55)),
    ((20, 20), (0x22, 0x33, 0x55)),   # 'A' has no pixel in column zero of its first row
    ((21, 20), (0xf8, 0xee, 0xcc)),   # bitmap 'A', first row, one foreground bit
    ((0, 100), (0xe3, 0x35, 0x42)),
    ((400, 100), (0x2e, 0xc7, 0x71)),
    ((799, 599), (0x3b, 0x67, 0xe1)),
)


def verify(ppm: bytes) -> tuple[int, int, str]:
    header = re.match(rb'P6\s+(\d+)\s+(\d+)\s+255\s', ppm)
    if header is None:
        raise ValueError('not a binary P6 screenshot')
    width, height = int(header[1]), int(header[2])
    if (width, height) != (800, 600) or len(ppm) - header.end() != width * height * 3:
        raise ValueError('QMP image size/geometry disagrees with bounded GOP mode')
    for (x, y), expected in SAMPLES:
        start = header.end() + 3 * (y * width + x)
        observed = tuple(ppm[start:start + 3])
        if observed != expected:
            raise ValueError(f'wrong actual pixel at {(x,y)}: {observed} != {expected}')
    return width, height, hashlib.sha256(ppm).hexdigest()


def capture(sock: Path, serial: Path, image: Path, receipt: Path, timeout: float) -> None:
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if serial.is_file() and READY in serial.read_bytes():
            break
        time.sleep(0.05)
    else:
        raise TimeoutError('ring-3 service never painted and reached ready marker')
    conn = qmp.Qmp(str(sock), connect_timeout_s=4)
    try:
        conn.command('screendump', filename=str(image), format='ppm')
    finally:
        conn.close()
    width, height, sha = verify(image.read_bytes())
    receipt.write_text(f'{width}x{height} {sha}\n')


def main() -> int:
    if len(sys.argv) != 6:
        raise SystemExit('usage: check_phase9_pixels.py QMP_SOCKET SERIAL PPM RECEIPT TIMEOUT_S')
    sock, serial, image, receipt = map(Path, sys.argv[1:5])
    try:
        capture(sock, serial, image, receipt, float(sys.argv[5]))
    except Exception as exc:
        receipt.with_suffix(receipt.suffix + '.error').write_text(str(exc) + '\n')
        print(f'[phase9-pixels] FAIL: {exc}', file=sys.stderr)
        return 1
    print(f'[phase9-pixels] PASS: {receipt.read_text().strip()}', flush=True)
    return 0


if __name__ == '__main__':
    sys.exit(main())
