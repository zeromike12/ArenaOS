#!/usr/bin/env python3
"""Build a FAT16 EFI System Partition image for QEMU/EDK2 boot tests.

The image is ALWAYS created from scratch (fresh file, fresh mkfs, then files
written once): pyfatfs cannot overwrite an existing file inside an image
(cluster-chain corruption), and stale state between runs is a determinism
hazard anyway (docs/DEV-ENV.md "known quirks").

Layout: EFI/BOOT/BOOTX64.EFI — the UEFI removable-media fallback path, which
EDK2 boots automatically with no NVRAM entries (fresh OVMF_VARS every run).
"""

import argparse
import sys
from pathlib import Path

from pyfatfs.PyFat import PyFat
from pyfatfs.PyFatFS import PyFatFS

FAT16_SIZE = 8 * 1024 * 1024  # 8 MiB: plenty for a few-MiB PE image
VOLUME_LABEL = "ARENAESP"


def build_esp(image_path: Path, efi_payload: Path, dest: str = "/EFI/BOOT/BOOTX64.EFI") -> Path:
    image_path = Path(image_path)
    efi_payload = Path(efi_payload)
    if not efi_payload.exists():
        sys.exit(f"error: EFI payload not found: {efi_payload}")

    image_path.parent.mkdir(parents=True, exist_ok=True)
    if image_path.exists():
        image_path.unlink()
    with open(image_path, "wb") as f:
        f.write(b"\0" * FAT16_SIZE)

    pf = PyFat()
    pf.mkfs(str(image_path), PyFat.FAT_TYPE_FAT16, size=FAT16_SIZE, label=VOLUME_LABEL)
    pf.close()

    fs = PyFatFS(str(image_path), read_only=False)
    try:
        parts = dest.strip("/").split("/")
        for i in range(1, len(parts)):
            d = "/" + "/".join(parts[:i])
            if not fs.exists(d):
                fs.makedir(d)
        fs.setbytes(dest, efi_payload.read_bytes())
    finally:
        fs.close()
    return image_path


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("payload", type=Path, help="PE32+ EFI application to place on the ESP")
    ap.add_argument("image", type=Path, help="output image path (rebuilt from scratch)")
    ap.add_argument("--dest", default="/EFI/BOOT/BOOTX64.EFI", help="destination path inside the ESP")
    args = ap.parse_args()
    out = build_esp(args.image, args.payload, args.dest)
    print(f"built {out} ({out.stat().st_size} bytes) with {args.dest} <- {args.payload}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
