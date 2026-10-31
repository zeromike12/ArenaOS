#!/usr/bin/env python3
"""Ensure an identical EFI payload produces a byte-identical FAT16 ESP."""

import hashlib
import tempfile
from pathlib import Path

from pyfatfs.PyFatFS import PyFatFS

from espimg import FAT16_SIZE, FAT_TIMESTAMP, build_esp


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main() -> int:
    payload = b"deterministic EFI payload\x00\x01"
    with tempfile.TemporaryDirectory(prefix="arena-esp-determinism-") as tmp:
        root = Path(tmp)
        payload_path = root / "BOOTX64.EFI"
        first = root / "first.img"
        second = root / "second.img"
        payload_path.write_bytes(payload)

        build_esp(first, payload_path)
        build_esp(second, payload_path)
        first_sha = sha256(first)
        second_sha = sha256(second)
        if first_sha != second_sha:
            raise AssertionError(
                f"identical EFI payloads produced different ESPs: "
                f"{first_sha} != {second_sha}")
        if first.stat().st_size != FAT16_SIZE:
            raise AssertionError(f"unexpected ESP size: {first.stat().st_size}")

        fs = PyFatFS(str(first), read_only=True, utc=True)
        try:
            with fs.openbin("/EFI/BOOT/BOOTX64.EFI", "r") as stream:
                extracted = stream.read()
            if extracted != payload:
                raise AssertionError("ESP extraction changed the EFI payload")
            modified = fs.getinfo("/EFI/BOOT/BOOTX64.EFI").raw["details"]["modified"]
            if modified != FAT_TIMESTAMP:
                raise AssertionError(
                    f"EFI timestamp is not the fixed UTC epoch: {modified}")
        finally:
            fs.close()

        print(f"[espimg] reproducible FAT16 ESP PASS sha256={first_sha}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
