#!/usr/bin/env python3
"""Independent QMP witness: real base, two windows and injected key pixels.

Captures before and after sending a key to the actual virtio-input device;
checks owned, clipped scene pixels, a bitmap glyph, preserved base, and a
client-painted response pixel. Serial assertions alone never make a receipt.
"""
import hashlib
import re
import sys
import time
from pathlib import Path
import qmp

READY = b'[window_b] held-cap focused surface painted'
KEY_READY = b'[window_b] real key pixel painted'
FORGED_READY = b'[window_a] forged input token refused'
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


def pixel(ppm: bytes, x: int, y: int) -> tuple[int, int, int]:
    header = re.match(rb'P6\s+(\d+)\s+(\d+)\s+255\s', ppm)
    if header is None:
        raise ValueError('missing P6 pixel header')
    start = header.end() + 3 * (y * int(header[1]) + x)
    return tuple(ppm[start:start + 3])


def await_marker(serial: Path, markers: tuple[bytes, ...], deadline: float) -> None:
    while time.monotonic() < deadline:
        if serial.is_file() and all(m in serial.read_bytes() for m in markers):
            return
        time.sleep(0.05)
    raise TimeoutError(f'live graphics marker(s) absent: {markers!r}')


def capture(sock: Path, serial: Path, image: Path, receipt: Path, timeout: float) -> None:
    deadline = time.monotonic() + timeout
    await_marker(serial, (READY,), deadline)
    before_path = image.with_suffix('.before.ppm')
    conn = qmp.Qmp(str(sock), connect_timeout_s=4)
    try:
        conn.command('screendump', filename=str(before_path), format='ppm')
        before = before_path.read_bytes()
        _, _, before_sha = verify(before)
        for xy, rgb in (((60, 70), (0xbd, 0x53, 0x38)),
                        ((190, 160), (0x3d, 0xcf, 0x7a)),
                        ((200, 190), (0x3d, 0xcf, 0x7a)),
                        ((193, 160), (0xf8, 0xee, 0xcc))):
            if pixel(before, *xy) != rgb:
                raise ValueError(f'pre-key QMP owned window pixel {xy}: {pixel(before, *xy)} != {rgb}')
        conn.key('q')  # real keyboard interrupt; NOT serial or host-side PPM edit
        await_marker(serial, (KEY_READY, FORGED_READY), deadline)
        conn.command('screendump', filename=str(image), format='ppm')
    finally:
        conn.close()
    after = image.read_bytes()
    width, height, sha = verify(after)
    if pixel(after, 200, 190) != (0xff, 0xbb, 0x11):
        raise ValueError('injected key did not change the live client backing pixel')
    for xy in ((60, 70), (0, 100), (700, 300), (799, 599)):
        if pixel(before, *xy) != pixel(after, *xy):
            raise ValueError(f'input changed an unrelated pixel: {xy}')
    receipt.write_text(f'{width}x{height} {sha}\nINPUT {before_sha} {sha}\n')
    before_path.unlink()


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
