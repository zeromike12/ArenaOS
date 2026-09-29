#!/usr/bin/env python3
"""ADR-0046 research probe: AFS1's valid-commit fallback hides lost history.

Not an 8.1 guest proof or a config implementation. Use the existing host
parser and the actual mkfs/commit-record encoding to exhibit an ambiguity
that MUST be resolved before promising fail-closed detection of arbitrary
committed-sector corruption. The second record deliberately reuses the
initial empty metadata: only commit selection, not allocation, is at issue.
"""
from pathlib import Path
from tempfile import TemporaryDirectory

import afs1


def main() -> None:
    with TemporaryDirectory() as tmp:
        disk = Path(tmp) / "scratch.img"
        afs1.mkfs(disk, afs1.INIT_USED + 128)
        raw = bytearray(disk.read_bytes())
        initial = afs1.Disk(bytes(raw)).commit()
        assert initial == (1, afs1.OBJTAB_START, afs1.BITMAP_START)
        newer = afs1.pack_commit(2, afs1.OBJTAB_START, afs1.BITMAP_START)
        slot = afs1.COMMIT_BASE * afs1.SECTOR  # seq 2: other ping-pong slot
        raw[slot:slot + afs1.SECTOR] = newer
        assert afs1.Disk(bytes(raw)).commit()[0] == 2
        raw[slot + afs1.SECTOR - 1] ^= 1  # after-commit checksum corruption
        hidden = afs1.Disk(bytes(raw)).commit()
        assert hidden == initial, hidden
        assert not afs1.audit(_save(disk, raw))  # older metadata still looks healthy
        print("ADR-0046 probe PASS: corrupted newer AFS1 commit silently selects older valid metadata")
        print("A config service given only fsd's namespace cannot distinguish this from no newer commit.")


def _save(path: Path, data: bytes) -> Path:
    path.write_bytes(data)
    return path


if __name__ == "__main__":
    main()
