"""AFS1 on-disk layout (M5.3, ADR-0023) — the host-side single source of truth.

AFS1 is ArenaOS's own filesystem: extent-based file data written
in-place, copy-on-write metadata (object table, allocation bitmap) and
a transactional ping-pong commit record. This module defines the layout
byte-for-byte as ``userspace/fsd`` writes it, provides ``mkfs`` (the
host formats the fresh scratch disk each run — the kernel-side mkfs is
5.4 territory) and the read-back parser ``test_m5.py`` uses to prove
the on-disk state after boot.

Layout of the 8 MiB scratch disk (16384 x 512 B sectors)::

    sector 0        superblock (static after mkfs, checksummed)
    sectors 1..2    commit records, ping-pong: slot = COMMIT_BASE + seq % 2
                    — the ONLY in-place writes on the disk; a torn 512 B
                    sector is assumed impossible (documented assumption)
    sectors 3..6    initial object table  (32 records x 64 B)
    sectors 7..10   initial allocation bitmap (16384 bits, 1 bit/sector)
    sectors 11..    free pool: data sectors, extent blocks, and each
                    commit's fresh CoW object-table/bitmap runs

Every CoW'd structure gets a NEW contiguous run each transaction; the
commit record flips atomically to point at it. Runs of the generation
BEFORE LAST are freed inside the current transaction (two-generation
delay: a crash always leaves the newest commit's blocks untouched).
"""

from __future__ import annotations

import struct

SECTOR = 512
MAGIC = b"ARENAFS1"
COMMIT_MAGIC = 0x4353_4641  # b"AFSC" little-endian
VERSION = 1

OBJ_COUNT = 32
OBJ_RECORD = 64
OBJTAB_SECTORS = OBJ_COUNT * OBJ_RECORD // SECTOR  # 4
BITMAP_SECTORS = 4  # 16384 bits — covers the 8 MiB scratch disk exactly
COMMIT_BASE = 1
COMMIT_SLOTS = 2
CHECKSUM_OFF = 504  # u64 FNV-1a over bytes [0, 504)
NAME_MAX = 32

# mkfs's fixed initial runs
OBJTAB_START = 3  # sectors 3..6
BITMAP_START = 7  # sectors 7..10
INIT_USED = BITMAP_START + BITMAP_SECTORS  # sectors 0..10 inclusive

# object record types
OBJ_FREE = 0
OBJ_FILE = 1

# extent block geometry
EXT_PER_BLOCK = (SECTOR - 8) // 8  # 62 x (start u32, len u32)

SCRATCH_MIB = 8  # mirrors arena_env.SCRATCH_MIB
SCRATCH_TOTAL_SECTORS_EXPECTED = SCRATCH_MIB * 1024 * 1024 // SECTOR


def pattern_byte(i: int) -> int:
    """The shared test pattern — mirrors userspace/abi.rs pattern_byte."""
    return ((i * 31 + 0x5A) & 0xFF) ^ ((i >> 3) & 0xFF)


def fnv1a64(data: bytes) -> int:
    """FNV-1a 64-bit — the checksum fsd computes identically in ring 3."""
    h = 0xCBF2_9CE4_8422_2325
    for b in data:
        h ^= b
        h = (h * 0x0000_0100_0000_01B3) & 0xFFFF_FFFF_FFFF_FFFF
    return h


def _sealed(rec: bytes) -> bytes:
    """Pad `rec` to 504 bytes and append its FNV-1a checksum u64."""
    assert len(rec) <= CHECKSUM_OFF
    body = rec + b"\0" * (CHECKSUM_OFF - len(rec))
    return body + struct.pack("<Q", fnv1a64(body))


def _check(sector: bytes) -> bool:
    return fnv1a64(sector[:CHECKSUM_OFF]) == struct.unpack_from("<Q", sector, CHECKSUM_OFF)[0]


# ---- record packers -----------------------------------------------------------


def pack_superblock(total_sectors: int) -> bytes:
    body = struct.pack(
        "<8sIIIIIII",
        MAGIC,
        VERSION,
        SECTOR,
        total_sectors,
        OBJ_COUNT,
        OBJ_RECORD,
        OBJTAB_SECTORS,
        BITMAP_SECTORS,
    )
    body += struct.pack("<I", COMMIT_BASE)
    return _sealed(body)


def pack_commit(seq: int, objtab_head: int, bitmap_head: int) -> bytes:
    # Layout: magic u32, version u32, seq u64, pad u64, objtab_head u32,
    # bitmap_head u32 — fsd packs the identical bytes in ring 3.
    body = struct.pack("<IIQQII", COMMIT_MAGIC, VERSION, seq, 0, objtab_head, bitmap_head)
    return _sealed(body)


def pack_object(type_: int, name: bytes, size: int, extent_head: int) -> bytes:
    # Layout: type u8, name_len u8, flags u8, pad u8, extent_head u32,
    # size u64, mtime u64 (0 — no clock in v1), reserved u64, name[32].
    assert len(name) <= NAME_MAX
    rec = struct.pack("<BBBBIQQQ", type_, len(name), 0, 0, extent_head, size, 0, 0)
    rec += name + b"\0" * (NAME_MAX - len(name))
    assert len(rec) == OBJ_RECORD
    return rec


def pack_extent_block(next_: int, extents: list[tuple[int, int]]) -> bytes:
    assert len(extents) <= EXT_PER_BLOCK
    # Each entry is (phys_start u32, len u32); an entry's FILE position
    # is the cumulative sum of the lengths before it (extents ordered
    # by file position — the implicit-offset extent tree; fsd v1
    # appends only at EOF, so the mapping always agrees with size).
    blk = struct.pack("<II", next_, len(extents))
    for start, length in extents:
        blk += struct.pack("<II", start, length)
    return blk + b"\0" * (SECTOR - len(blk))


def pack_bitmap(used: set[int], total_sectors: int) -> bytes:
    bm = bytearray(BITMAP_SECTORS * SECTOR)
    for s in used:
        assert 0 <= s < min(total_sectors, len(bm) * 8)
        bm[s // 8] |= 1 << (s % 8)
    return bytes(bm)


# ---- mkfs -----------------------------------------------------------------------


def mkfs(path, total_sectors: int) -> None:
    """Format `path` (already truncated to size) as a fresh empty AFS1."""
    used = set(range(INIT_USED))
    disk = bytearray(total_sectors * SECTOR)
    disk[0:SECTOR] = pack_superblock(total_sectors)
    # first commit: seq 1 → slot sector COMMIT_BASE + 1 = 2 (sector 1
    # stays zeroed = invalid, so mount can never pick a stale slot).
    commit = pack_commit(1, OBJTAB_START, BITMAP_START)
    slot = COMMIT_BASE + (1 % COMMIT_SLOTS)
    disk[slot * SECTOR : (slot + 1) * SECTOR] = commit
    objtab = pack_object(OBJ_FREE, b"", 0, 0) * OBJ_COUNT
    disk[OBJTAB_START * SECTOR : (OBJTAB_START + OBJTAB_SECTORS) * SECTOR] = objtab
    bitmap = pack_bitmap(used, total_sectors)
    disk[BITMAP_START * SECTOR : (BITMAP_START + BITMAP_SECTORS) * SECTOR] = bitmap
    with open(path, "wb") as f:
        f.write(disk)


# ---- read-back parser (test_m5.py's on-disk proof) --------------------------------


class Afs1Error(Exception):
    """Raised when the image violates the layout — a real test failure."""


class Disk:
    """Read-only view of a formatted AFS1 image."""

    def __init__(self, data: bytes):
        self.data = data

    def sector(self, n: int) -> bytes:
        return self.data[n * SECTOR : (n + 1) * SECTOR]

    def superblock(self) -> dict:
        sb = self.sector(0)
        if sb[:8] != MAGIC:
            raise Afs1Error(f"bad superblock magic {sb[:8]!r}")
        if not _check(sb):
            raise Afs1Error("superblock checksum mismatch")
        (_, version, block, total, ocount, orec, otabs, bms) = struct.unpack_from(
            "<8sIIIIIII", sb, 0
        )
        (commit_base,) = struct.unpack_from("<I", sb, 36)
        if (version, block, ocount, orec, otabs, bms, commit_base) != (
            VERSION, SECTOR, OBJ_COUNT, OBJ_RECORD, OBJTAB_SECTORS, BITMAP_SECTORS, COMMIT_BASE,
        ):
            raise Afs1Error(f"unexpected superblock geometry {sb[:40].hex()}")
        return {"total_sectors": total}

    def commit(self) -> tuple[int, int, int]:
        """The newest valid commit: (seq, objtab_head, bitmap_head)."""
        best = None
        for i in range(COMMIT_SLOTS):
            rec = self.sector(COMMIT_BASE + i)
            magic, version = struct.unpack_from("<II", rec, 0)
            if magic != COMMIT_MAGIC or version != VERSION or not _check(rec):
                continue
            seq, _, ot, bm = struct.unpack_from("<QQII", rec, 8)
            if best is None or seq > best[0]:
                best = (seq, ot, bm)
        if best is None:
            raise Afs1Error("no valid commit record")
        return best

    def objects(self, objtab_head: int) -> list[dict]:
        raw = b"".join(self.sector(objtab_head + i) for i in range(OBJTAB_SECTORS))
        objs = []
        for i in range(OBJ_COUNT):
            rec = raw[i * OBJ_RECORD : (i + 1) * OBJ_RECORD]
            type_, nlen, _, _, ehead, size = struct.unpack_from("<BBBBIQ", rec, 0)
            name = rec[32 : 32 + nlen]
            objs.append(
                {"type": type_, "name": name, "size": size, "extent_head": ehead}
            )
        return objs

    def bitmap(self, bitmap_head: int) -> bytearray:
        raw = bytearray()
        for i in range(BITMAP_SECTORS):
            raw += self.sector(bitmap_head + i)
        return raw

    def extents(self, head: int) -> list[tuple[int, int]]:
        """Walk the extent chain from `head` (0 = empty file)."""
        out: list[tuple[int, int]] = []
        cur = head
        seen = set()
        while cur != 0:
            if cur in seen:
                raise Afs1Error(f"extent chain loop at sector {cur}")
            seen.add(cur)
            blk = self.sector(cur)
            next_, count = struct.unpack_from("<II", blk, 0)
            if count > EXT_PER_BLOCK:
                raise Afs1Error(f"extent count {count} over block capacity")
            for i in range(count):
                start, length = struct.unpack_from("<II", blk, 8 + i * 8)
                out.append((start, length))
            cur = next_
        return out

    def find(self, name: bytes, objtab_head: int) -> dict | None:
        for obj in self.objects(objtab_head):
            if obj["type"] == OBJ_FILE and obj["name"] == name:
                return obj
        return None


if __name__ == "__main__":
    import sys

    if len(sys.argv) == 3 and sys.argv[1] == "mkfs":
        size = 8 * 1024 * 1024
        mkfs(sys.argv[2], size // SECTOR)
        print(f"afs1: formatted {sys.argv[2]} ({size // SECTOR} sectors)")
    elif len(sys.argv) == 3 and sys.argv[1] == "dump":
        d = Disk(open(sys.argv[2], "rb").read())
        geo = d.superblock()
        seq, ot, bm = d.commit()
        print(f"total_sectors={geo['total_sectors']} commit: seq={seq} objtab@{ot} bitmap@{bm}")
        for i, o in enumerate(d.objects(ot)):
            if o["type"] != OBJ_FREE:
                print(f"  obj[{i}] {o['name'].decode()} size={o['size']} extents={d.extents(o['extent_head'])}")
    else:
        print("usage: afs1.py mkfs IMAGE | afs1.py dump IMAGE")
