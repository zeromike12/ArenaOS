#!/usr/bin/env python3
"""AFS2 (ADR-0076): host model and single source of truth for the format.

Pure Python, no dependencies. A `Volume` mounts an image (bytes of the AFS2
volume region; block 0 = superblock), and every mutating operation is one
transaction: all writes are buffered and only issued at commit, in order
(new metadata/data blocks, new bitmap blocks, then the one-sector commit
record). `Volume.last_writes` keeps that sequence so the crash-prefix
proofs can replay any prefix onto the pre-operation image.

Errors are `FsError(code)` with codes: ENOENT EEXIST ENOTDIR EISDIR
ENOTEMPTY EINVAL ENOSPC EFBIG ELOOP CORRUPT.
"""
import struct

BLOCK = 4096
SECTOR = 512
MAGIC = b"ARENAFS2"
COMMIT_MAGIC = b"AFS2COMT"
VERSION = 1
TRAILER = 24
PAYLOAD = BLOCK - TRAILER  # 4072
PTRS = PAYLOAD // 8  # 509
RECORD = 128
RECORDS_PER_LEAF = PAYLOAD // RECORD  # 31
BITS_PER_BITMAP = PAYLOAD * 8  # 32576
MAX_BITMAPS = 52
MAX_OBJECTS = PTRS * RECORDS_PER_LEAF  # 15779
FIRST_DATA = 3
NAME_MAX = 255
MAX_DEPTH = 2
MAX_FILE_BLOCKS = PTRS * PTRS
KIND_OBJROOT, KIND_OBJLEAF, KIND_MAPNODE, KIND_DIRBLK, KIND_BITMAP = 1, 2, 3, 4, 5
FILE, DIR = 1, 2
ROOT_INDEX = 0


class FsError(Exception):
    def __init__(self, code):
        super().__init__(code)
        self.code = code


def fnv(data):
    h = 0xCBF29CE484222325
    for b in data:
        h = ((h ^ b) * 0x100000001B3) & 0xFFFFFFFFFFFFFFFF
    return h


def oid(index, generation):
    return (generation << 32) | index


def split(object_id):
    return object_id & 0xFFFFFFFF, object_id >> 32


def valid_name(name):
    if not isinstance(name, (bytes, bytearray)) or not 1 <= len(name) <= NAME_MAX:
        return False
    if name in (b".", b".."):
        return False
    if any(b < 0x20 or b == 0x7F or b == ord("/") for b in name):
        return False
    try:
        name.decode("utf-8")
    except UnicodeDecodeError:
        return False
    return True


def seal(kind, block_no, payload):
    """A metadata block: payload (≤ 4072 bytes) + trailer."""
    assert len(payload) <= PAYLOAD
    body = bytes(payload) + bytes(PAYLOAD - len(payload))
    head = body + struct.pack("<IIQ", kind, 0, block_no)
    return head + struct.pack("<Q", fnv(head))


def unseal(raw, kind, block_no):
    if len(raw) != BLOCK:
        raise FsError("CORRUPT")
    k, _, b = struct.unpack_from("<IIQ", raw, PAYLOAD)
    (c,) = struct.unpack_from("<Q", raw, BLOCK - 8)
    if k != kind or b != block_no or c != fnv(raw[: BLOCK - 8]):
        raise FsError("CORRUPT")
    return raw[:PAYLOAD]


# ---- records ---------------------------------------------------------------

REC = struct.Struct("<BBHIQQQQQQIIQ")  # 72 bytes used, padded to 128


class Record:
    __slots__ = ("type", "depth", "flags", "generation", "parent", "size", "map",
                 "ctime", "mtime", "entries", "dir_blocks", "version")

    def __init__(self, type=0, depth=0, flags=0, generation=0, parent=0, size=0, map=0,
                 ctime=0, mtime=0, entries=0, dir_blocks=0, version=0):
        self.type, self.depth, self.flags, self.generation = type, depth, flags, generation
        self.parent, self.size, self.map = parent, size, map
        self.ctime, self.mtime, self.entries = ctime, mtime, entries
        self.dir_blocks, self.version = dir_blocks, version

    def pack(self):
        raw = REC.pack(self.type, self.depth, self.flags, self.generation, self.parent,
                       self.size, self.map, self.ctime, self.mtime, 0, self.entries,
                       self.dir_blocks, self.version)
        return raw + bytes(RECORD - len(raw))

    @classmethod
    def unpack(cls, raw):
        (t, d, f, g, p, s, m, c, mt, _, e, db, v) = REC.unpack_from(raw)
        if t not in (0, FILE, DIR) or d > MAX_DEPTH or any(raw[REC.size:RECORD]):
            raise FsError("CORRUPT")
        return cls(t, d, f, g, p, s, m, c, mt, e, db, v)

    def copy(self):
        return Record(*(getattr(self, k) for k in self.__slots__))


# ---- directory blocks ---------------------------------------------------------

ENTRY_HEAD = struct.Struct("<IIBB")  # index, generation, type, name_len


def entry_size(name):
    return ENTRY_HEAD.size + len(name)


def pack_dir(entries):
    """entries: list of (name, index, generation, type), sorted by name."""
    body = bytearray(struct.pack("<HH", len(entries), 0))
    for name, index, gen, typ in entries:
        body += ENTRY_HEAD.pack(index, gen, typ, len(name)) + name
    if len(body) > PAYLOAD:
        raise FsError("EINVAL")
    struct.pack_into("<H", body, 2, len(body))
    return bytes(body)


def unpack_dir(payload):
    count, used = struct.unpack_from("<HH", payload)
    at, out = 4, []
    for _ in range(count):
        if at + ENTRY_HEAD.size > PAYLOAD:
            raise FsError("CORRUPT")
        index, gen, typ, n = ENTRY_HEAD.unpack_from(payload, at)
        at += ENTRY_HEAD.size
        name = bytes(payload[at:at + n])
        at += n
        if len(name) != n or typ not in (FILE, DIR) or not valid_name(name):
            raise FsError("CORRUPT")
        out.append((name, index, gen, typ))
    if at != used or any(payload[at:]):
        raise FsError("CORRUPT")
    if [e[0] for e in out] != sorted(e[0] for e in out):
        raise FsError("CORRUPT")
    return out


DIR_SPLIT = PAYLOAD - 4  # entry bytes that fit one block


# ---- format ----------------------------------------------------------------------

def mkfs(total_blocks, volume_id=0x4152454E41324653, wall_us=0):
    """A fresh volume image: superblock, empty root directory, commit seq 1."""
    if total_blocks < 64 or total_blocks > MAX_BITMAPS * BITS_PER_BITMAP:
        raise FsError("EINVAL")
    bitmaps = -(-total_blocks // BITS_PER_BITMAP)
    img = bytearray(total_blocks * BLOCK)
    sb = bytearray(SECTOR)
    struct.pack_into("<8sIIQIIQ", sb, 0, MAGIC, VERSION, BLOCK, total_blocks, bitmaps,
                     MAX_OBJECTS, volume_id)
    struct.pack_into("<Q", sb, 504, fnv(sb[:504]))
    img[:SECTOR] = sb
    vol = Volume.__new__(Volume)
    vol._init(img, total_blocks, bitmaps, volume_id)
    vol.seq, vol.root, vol.bitmap_blocks = 0, 0, []
    vol.bitmap = bytearray(-(-total_blocks // 8))
    for b in range(FIRST_DATA):
        vol._set_bit(vol.bitmap, b)
    vol.free_blocks = total_blocks - FIRST_DATA
    vol.used_objects = 0
    vol._cache = {}
    tx = Tx(vol)
    rec = Record(type=DIR, generation=1, parent=oid(ROOT_INDEX, 1), ctime=wall_us,
                 mtime=wall_us, version=1)
    tx.set_record(ROOT_INDEX, rec)
    tx.used_objects = 1
    tx.commit(wall_us)
    return bytes(vol.dev)


class Volume:
    """A mounted AFS2 volume over a bytearray image (the volume region)."""

    def __init__(self, image):
        img = bytearray(image)
        if len(img) < BLOCK * 64 or len(img) % BLOCK:
            raise FsError("CORRUPT")
        sb = img[:SECTOR]
        magic, version, bs, total, bitmaps, objects, vid = struct.unpack_from("<8sIIQIIQ", sb, 0)
        if (magic != MAGIC or version != VERSION or bs != BLOCK or objects != MAX_OBJECTS
                or struct.unpack_from("<Q", sb, 504)[0] != fnv(sb[:504])
                or total * BLOCK > len(img) or bitmaps != -(-total // BITS_PER_BITMAP)):
            raise FsError("CORRUPT")
        self._init(img, total, bitmaps, vid)
        best = None
        for slot in (1, 2):
            rec = self._commit_record(slot)
            if rec is not None and (best is None or rec[0] > best[0]):
                best = rec
        if best is None:
            raise FsError("CORRUPT")
        self.seq, self.root, self.free_blocks, self.used_objects, self.wall_us, bm = best
        if len(bm) != bitmaps:
            raise FsError("CORRUPT")
        self.bitmap_blocks = bm
        self.bitmap = bytearray()
        for i, b in enumerate(bm):
            self.bitmap += self._meta(b, KIND_BITMAP)
        self.bitmap = self.bitmap[: -(-total // 8)]
        # Fail closed: the newest commit's own structures must verify.
        self._meta(self.root, KIND_OBJROOT)
        if self.stat_index(ROOT_INDEX).type != DIR:
            raise FsError("CORRUPT")
        if self.free_blocks != total - sum(bin(x).count("1") for x in self.bitmap):
            raise FsError("CORRUPT")

    def _init(self, img, total, bitmaps, vid):
        self.dev, self.total, self.nbitmaps, self.volume_id = img, total, bitmaps, vid
        self.last_writes = []
        self._cache = {}
        # Smallest index that may be free (an allocation hint, never state).
        self.free_hint = 1

    def _commit_record(self, slot):
        raw = self.dev[slot * BLOCK: slot * BLOCK + SECTOR]
        if raw[:8] != COMMIT_MAGIC or struct.unpack_from("<Q", raw, 504)[0] != fnv(raw[:504]):
            return None
        version, seq, root, free, used, wall, count = struct.unpack_from("<I4xQQQI4xQI", raw, 8)
        if version != VERSION or count > MAX_BITMAPS or slot != 1 + seq % 2:
            return None
        bm = list(struct.unpack_from(f"<{count}Q", raw, 64))
        return seq, root, free, used, wall, bm

    # -- raw block access
    def _read(self, block):
        if not FIRST_DATA <= block < self.total:
            raise FsError("CORRUPT")
        return bytes(self.dev[block * BLOCK:(block + 1) * BLOCK])

    def _meta(self, block, kind):
        key = (block, kind)
        if key not in self._cache:
            self._cache[key] = unseal(self._read(block), kind, block)
        return self._cache[key]

    @staticmethod
    def _bit(bm, b):
        return bm[b >> 3] >> (b & 7) & 1

    @staticmethod
    def _set_bit(bm, b):
        bm[b >> 3] |= 1 << (b & 7)

    @staticmethod
    def _clear_bit(bm, b):
        bm[b >> 3] &= ~(1 << (b & 7)) & 0xFF

    # -- committed reads
    def _root_ptrs(self):
        return struct.unpack_from(f"<{PTRS}Q", self._meta(self.root, KIND_OBJROOT))

    def stat_index(self, index, tx=None):
        if tx is not None and index in tx.records:
            return tx.records[index]
        if not 0 <= index < MAX_OBJECTS:
            raise FsError("ENOENT")
        leaf = (tx.root if tx is not None else self._root_ptrs())[index // RECORDS_PER_LEAF]
        if leaf == 0:
            return Record()
        payload = self._meta(leaf, KIND_OBJLEAF)
        at = (index % RECORDS_PER_LEAF) * RECORD
        return Record.unpack(payload[at:at + RECORD])

    def record(self, object_id, tx=None):
        index, gen = split(object_id)
        rec = self.stat_index(index, tx)
        if rec.type == 0 or rec.generation != gen:
            raise FsError("ENOENT")
        return rec

    def _map_get(self, rec, logical, tx=None):
        if rec.depth == 0:
            return 0
        if rec.depth == 1:
            return 0 if logical >= PTRS else self._node(rec.map, tx)[logical]
        top = self._node(rec.map, tx)[logical // PTRS]
        return 0 if top == 0 else self._node(top, tx)[logical % PTRS]

    def _node(self, block, tx=None):
        if tx is not None and block in tx.nodes:
            return tx.nodes[block]
        return list(struct.unpack_from(f"<{PTRS}Q", self._meta(block, KIND_MAPNODE)))

    def _data(self, block, tx=None):
        if tx is not None and block in tx.data:
            return tx.data[block]
        return self._read(block)

    def read(self, object_id, offset=0, length=None, tx=None):
        rec = self.record(object_id, tx)
        if rec.type != FILE:
            raise FsError("EISDIR")
        end = rec.size if length is None else min(rec.size, offset + length)
        out = bytearray()
        pos = offset
        while pos < end:
            logical, within = divmod(pos, BLOCK)
            n = min(BLOCK - within, end - pos)
            b = self._map_get(rec, logical, tx)
            out += bytes(n) if b == 0 else self._data(b, tx)[within:within + n]
            pos += n
        return bytes(out)

    def dir_entries(self, object_id, tx=None):
        rec = self.record(object_id, tx)
        if rec.type != DIR:
            raise FsError("ENOTDIR")
        out = []
        for i in range(rec.dir_blocks):
            b = self._map_get(rec, i, tx)
            if tx is not None and b in tx.dirs:
                out.append(tx.dirs[b])
            else:
                out.append(self.dir_entries_block(b))
        return out

    def list(self, dir_id, after=b"", limit=64):
        """Entries strictly after `after` (name cursor), at most `limit`."""
        out = []
        for block in self.dir_entries(dir_id):
            for name, index, gen, typ in block:
                if name > after:
                    out.append((name, oid(index, gen), typ))
                    if len(out) == limit:
                        return out
        return out

    def lookup(self, dir_id, name):
        for block in self.dir_entries(dir_id):
            for n, index, gen, typ in block:
                if n == name:
                    return oid(index, gen), typ
        raise FsError("ENOENT")

    def resolve(self, path):
        """Host convenience: '/a/b' → object id (paths are UI, not authority)."""
        cur = oid(ROOT_INDEX, self.stat_index(ROOT_INDEX).generation)
        for part in [p for p in path.split("/") if p]:
            cur, _ = self.lookup(cur, part.encode())
        return cur

    def root_id(self):
        return oid(ROOT_INDEX, self.stat_index(ROOT_INDEX).generation)

    def stat(self, object_id):
        r = self.record(object_id)
        return {"type": r.type, "size": r.size, "parent": r.parent, "ctime": r.ctime,
                "mtime": r.mtime, "entries": r.entries, "version": r.version}

    def statfs(self):
        return {"blocks": self.total, "free": self.free_blocks, "objects": self.used_objects,
                "max_objects": MAX_OBJECTS, "seq": self.seq}

    # -- mutations: one transaction each
    def _run(self, fn, wall_us=0):
        tx = Tx(self)
        result = fn(tx)
        tx.commit(wall_us)
        return result

    def create(self, dir_id, name, wall_us=0):
        return self._run(lambda tx: tx.new_object(dir_id, name, FILE, wall_us), wall_us)

    def mkdir(self, dir_id, name, wall_us=0):
        return self._run(lambda tx: tx.new_object(dir_id, name, DIR, wall_us), wall_us)

    def write(self, object_id, offset, data, wall_us=0):
        return self._run(lambda tx: tx.write(object_id, offset, data, wall_us), wall_us)

    def truncate(self, object_id, size, wall_us=0):
        return self._run(lambda tx: tx.truncate(object_id, size, wall_us), wall_us)

    def unlink(self, dir_id, name, wall_us=0):
        return self._run(lambda tx: tx.remove(dir_id, name, FILE, wall_us), wall_us)

    def rmdir(self, dir_id, name, wall_us=0):
        return self._run(lambda tx: tx.remove(dir_id, name, DIR, wall_us), wall_us)

    def rename(self, src_dir, name, dst_dir, new_name, wall_us=0):
        return self._run(lambda tx: tx.rename(src_dir, name, dst_dir, new_name, wall_us), wall_us)

    def set_times(self, object_id, ctime, mtime, wall_us=0):
        def fn(tx):
            rec = tx.get(object_id)
            rec.ctime, rec.mtime = ctime, mtime
            rec.version += 1
            tx.put(object_id, rec)
        return self._run(fn, wall_us)

    def image(self):
        return bytes(self.dev)


class Tx:
    """One transaction: buffered, copy-on-write, committed by one sector."""

    def __init__(self, vol):
        self.vol = vol
        self.root = list(vol._root_ptrs()) if vol.root else [0] * PTRS
        self.records = {}   # index -> Record (changed this tx)
        self.nodes = {}     # fresh map-node block -> pointer list
        self.dirs = {}      # fresh dir block -> entries list
        self.data = {}      # fresh data block -> bytes
        self.work = bytearray(vol.bitmap)  # committed + this tx's allocations
        self.pending_free = set()
        self.fresh = set()
        self.cursor = FIRST_DATA
        self.used_objects = vol.used_objects
        self.hint = vol.free_hint

    # -- allocation: never a block the committed generation references
    def alloc(self):
        total = self.vol.total
        for i in range(total):
            b = FIRST_DATA + (self.cursor - FIRST_DATA + i) % (total - FIRST_DATA)
            if not Volume._bit(self.work, b):
                Volume._set_bit(self.work, b)
                self.fresh.add(b)
                self.cursor = b + 1
                return b
        raise FsError("ENOSPC")

    def free(self, block):
        if block == 0:
            return
        if block in self.fresh:
            # Never referenced by the committed generation: reusable now.
            self.fresh.discard(block)
            self.nodes.pop(block, None)
            self.dirs.pop(block, None)
            self.data.pop(block, None)
            Volume._clear_bit(self.work, block)
        else:
            # Committed generation still references it until commit.
            self.pending_free.add(block)

    # -- records
    def get(self, object_id):
        return self.vol.record(object_id, self).copy()

    def put(self, object_id, rec):
        index, _ = split(object_id)
        self.set_record(index, rec)

    def set_record(self, index, rec):
        self.records[index] = rec

    def now(self, wall_us):
        return wall_us

    # -- block map (copy-on-write)
    def _own_node(self, block, kind_fresh=True):
        """Return a writable (fresh) copy of map node `block` (0 = new)."""
        if block in self.nodes:
            return block
        ptrs = [0] * PTRS if block == 0 else self.vol._node(block)
        if block:
            self.free(block)
        nb = self.alloc()
        self.nodes[nb] = list(ptrs)
        return nb

    def map_set(self, rec, logical, block):
        if logical >= MAX_FILE_BLOCKS:
            raise FsError("EFBIG")
        if rec.depth == 0:
            if block == 0:
                return
            rec.depth, rec.map = 1, 0
            rec.map = self._own_node(0)
        if rec.depth == 1 and logical >= PTRS:
            if block == 0:
                return
            top = self.alloc()
            self.nodes[top] = [0] * PTRS
            self.nodes[top][0] = rec.map
            rec.depth, rec.map = 2, top
        rec.map = self._own_node(rec.map)
        if rec.depth == 1:
            self.nodes[rec.map][logical] = block
            return
        top = self.nodes[rec.map]
        child = top[logical // PTRS]
        if child == 0 and block == 0:
            return
        child = self._own_node(child)
        top[logical // PTRS] = child
        self.nodes[child][logical % PTRS] = block

    def map_get(self, rec, logical):
        return self.vol._map_get(rec, logical, self)

    def drop_map(self, rec, keep_blocks):
        """Free every data block at logical ≥ keep_blocks and empty nodes."""
        if rec.depth == 0:
            return
        if rec.depth == 1:
            node = self.vol._node(rec.map, self)
            changed = False
            for i in range(keep_blocks, PTRS):
                if node[i]:
                    if not changed:
                        rec.map = self._own_node(rec.map)
                        node = self.nodes[rec.map]
                        changed = True
                    self.free(node[i])
                    node[i] = 0
            if keep_blocks == 0:
                self.free(rec.map)
                rec.depth, rec.map = 0, 0
            return
        top = self.vol._node(rec.map, self)
        for c in range(PTRS):
            child = top[c]
            if child == 0:
                continue
            first = c * PTRS
            if first + PTRS <= keep_blocks:
                continue
            node = self.vol._node(child, self)
            if keep_blocks <= first:
                for b in node:
                    self.free(b)
                rec.map = self._own_node(rec.map)
                top = self.nodes[rec.map]
                self.free(child)
                top[c] = 0
                continue
            nc = self._own_node(child)
            rec.map = self._own_node(rec.map)
            top = self.nodes[rec.map]
            top[c] = nc
            for i in range(keep_blocks - first, PTRS):
                if self.nodes[nc][i]:
                    self.free(self.nodes[nc][i])
                    self.nodes[nc][i] = 0
        if keep_blocks == 0:
            self.free(rec.map)
            rec.depth, rec.map = 0, 0

    # -- file data
    def write(self, object_id, offset, data, wall_us):
        rec = self.get(object_id)
        if rec.type != FILE:
            raise FsError("EISDIR")
        end = offset + len(data)
        if -(-end // BLOCK) > MAX_FILE_BLOCKS:
            raise FsError("EFBIG")
        pos = offset
        while pos < end:
            logical, within = divmod(pos, BLOCK)
            n = min(BLOCK - within, end - pos)
            old = self.map_get(rec, logical)
            if within == 0 and n == BLOCK:
                content = bytes(data[pos - offset:pos - offset + n])
            else:
                base = bytearray(BLOCK) if old == 0 else bytearray(self.vol._data(old, self))
                # Bytes beyond the current size of a partial old block are
                # never visible; keep them zero so holes stay holes.
                if old and rec.size < (logical + 1) * BLOCK:
                    tail = max(0, rec.size - logical * BLOCK)
                    base[tail:] = bytes(BLOCK - tail)
                base[within:within + n] = data[pos - offset:pos - offset + n]
                content = bytes(base)
            nb = self.alloc()
            self.data[nb] = content
            self.map_set(rec, logical, nb)
            if old:
                self.free(old)
            pos += n
        rec.size = max(rec.size, end)
        rec.mtime = wall_us
        rec.version += 1
        self.put(object_id, rec)
        return len(data)

    def truncate(self, object_id, size, wall_us):
        rec = self.get(object_id)
        if rec.type != FILE:
            raise FsError("EISDIR")
        if -(-size // BLOCK) > MAX_FILE_BLOCKS:
            raise FsError("EFBIG")
        if size < rec.size:
            keep = -(-size // BLOCK)
            self.drop_map(rec, keep)
            if size % BLOCK and rec.depth:
                last = self.map_get(rec, size // BLOCK)
                if last:
                    content = bytearray(self.vol._data(last, self))
                    content[size % BLOCK:] = bytes(BLOCK - size % BLOCK)
                    nb = self.alloc()
                    self.data[nb] = bytes(content)
                    self.map_set(rec, size // BLOCK, nb)
                    self.free(last)
        rec.size = size
        rec.mtime = wall_us
        rec.version += 1
        self.put(object_id, rec)

    # -- directories
    def entries(self, dir_id):
        return [list(b) for b in self.vol.dir_entries(dir_id, self)]

    def store_dir(self, dir_id, rec, blocks):
        """Write the directory's block list (copy-on-write, minimal)."""
        old_count = rec.dir_blocks
        old = [self.map_get(rec, i) for i in range(old_count)]
        for i, entries in enumerate(blocks):
            payload = entries
            prev = old[i] if i < old_count else 0
            if prev and prev not in self.dirs and self.vol.dir_entries_block(prev) == entries:
                continue
            if prev in self.dirs:
                self.dirs[prev] = list(entries)
                continue
            nb = self.alloc()
            self.dirs[nb] = list(payload)
            self.map_set(rec, i, nb)
            if prev:
                self.free(prev)
        for i in range(len(blocks), old_count):
            prev = old[i]
            self.map_set(rec, i, 0)
            self.free(prev)
        if not blocks:
            self.drop_map(rec, 0)
        rec.dir_blocks = len(blocks)
        rec.size = len(blocks) * BLOCK
        rec.entries = sum(len(b) for b in blocks)

    @staticmethod
    def find(blocks, name):
        """(block, entry) position of `name`, by the global sort order."""
        for bi, b in enumerate(blocks):
            if b and b[-1][0] >= name:
                lo, hi = 0, len(b)
                while lo < hi:
                    mid = (lo + hi) // 2
                    if b[mid][0] < name:
                        lo = mid + 1
                    else:
                        hi = mid
                return (bi, lo) if lo < len(b) and b[lo][0] == name else None
        return None

    @staticmethod
    def place(blocks, entry):
        """Insert `entry` keeping global order; split a full block in two."""
        name = entry[0]
        if not blocks:
            return [[entry]]
        k = len(blocks) - 1
        for i, b in enumerate(blocks):
            if b and b[-1][0] >= name:
                k = i
                break
        b = sorted(blocks[k] + [entry])
        if sum(entry_size(e[0]) for e in b) <= DIR_SPLIT:
            return blocks[:k] + [b] + blocks[k + 1:]
        half = len(b) // 2
        return blocks[:k] + [b[:half], b[half:]] + blocks[k + 1:]

    def new_object(self, dir_id, name, typ, wall_us):
        if not valid_name(name):
            raise FsError("EINVAL")
        parent = self.get(dir_id)
        if parent.type != DIR:
            raise FsError("ENOTDIR")
        blocks = self.entries(dir_id)
        if self.find(blocks, name) is not None:
            raise FsError("EEXIST")
        index = self.free_index()
        old = self.vol.stat_index(index, self)
        gen = (old.generation + 1) & 0xFFFFFFFF or 1
        rec = Record(type=typ, generation=gen, parent=dir_id, ctime=wall_us, mtime=wall_us,
                     version=1)
        self.set_record(index, rec)
        self.used_objects += 1
        blocks = self.place(blocks, (bytes(name), index, gen, typ))
        self.store_dir(dir_id, parent, blocks)
        parent.mtime = wall_us
        parent.version += 1
        self.put(dir_id, parent)
        return oid(index, gen)

    def free_index(self):
        if self.used_objects >= MAX_OBJECTS:
            raise FsError("ENOSPC")
        for index in range(max(1, self.hint), MAX_OBJECTS):
            if self.vol.stat_index(index, self).type == 0:
                self.hint = index + 1
                return index
        raise FsError("ENOSPC")

    def remove(self, dir_id, name, typ, wall_us):
        parent = self.get(dir_id)
        if parent.type != DIR:
            raise FsError("ENOTDIR")
        blocks = self.entries(dir_id)
        for bi, b in enumerate(blocks):
            for ei, e in enumerate(b):
                if e[0] == name:
                    child_id = oid(e[1], e[2])
                    child = self.get(child_id)
                    if child.type != typ:
                        raise FsError("EISDIR" if child.type == DIR else "ENOTDIR")
                    if typ == DIR and child.entries:
                        raise FsError("ENOTEMPTY")
                    self.drop_map(child, 0)
                    gen = child.generation
                    self.set_record(e[1], Record(generation=gen))
                    self.hint = min(self.hint, e[1])
                    self.used_objects -= 1
                    del b[ei]
                    if not b:
                        del blocks[bi]
                    self.store_dir(dir_id, parent, blocks)
                    parent.mtime = wall_us
                    parent.version += 1
                    self.put(dir_id, parent)
                    return
        raise FsError("ENOENT")

    def rename(self, src_dir, name, dst_dir, new_name, wall_us):
        if not valid_name(new_name):
            raise FsError("EINVAL")
        src = self.get(src_dir)
        dst = self.get(dst_dir)
        if src.type != DIR or dst.type != DIR:
            raise FsError("ENOTDIR")
        sblocks = self.entries(src_dir)
        found = None
        for bi, b in enumerate(sblocks):
            for ei, e in enumerate(b):
                if e[0] == name:
                    found = (bi, ei, e)
        if found is None:
            raise FsError("ENOENT")
        bi, ei, e = found
        child_id = oid(e[1], e[2])
        if src_dir == dst_dir and name == new_name:
            return child_id
        dblocks = sblocks if src_dir == dst_dir else self.entries(dst_dir)
        if any(x[0] == new_name for b in dblocks for x in b):
            raise FsError("EEXIST")
        if e[3] == DIR:
            # Cycle: dst_dir must not be child_id or one of its descendants.
            cur = dst_dir
            root = self.vol.root_id() if self.vol.root else oid(ROOT_INDEX, 1)
            while True:
                if cur == child_id:
                    raise FsError("ELOOP")
                if cur == root:
                    break
                cur = self.get(cur).parent
        del sblocks[bi][ei]
        if not sblocks[bi]:
            del sblocks[bi]
        entry = (bytes(new_name), e[1], e[2], e[3])
        if src_dir == dst_dir:
            sblocks = self.place(sblocks, entry)
            self.store_dir(src_dir, src, sblocks)
            src.mtime = wall_us
            src.version += 1
            self.put(src_dir, src)
        else:
            self.store_dir(src_dir, src, sblocks)
            src.mtime = wall_us
            src.version += 1
            self.put(src_dir, src)
            dst = self.get(dst_dir)
            dblocks = self.place(self.entries(dst_dir), entry)
            self.store_dir(dst_dir, dst, dblocks)
            dst.mtime = wall_us
            dst.version += 1
            self.put(dst_dir, dst)
        child = self.get(child_id)
        child.parent = dst_dir
        child.version += 1
        self.put(child_id, child)
        return child_id

    # -- commit
    def commit(self, wall_us):
        vol = self.vol
        writes = []
        # Changed records → fresh leaves; then a fresh root.
        leaves = {}
        for index, rec in self.records.items():
            leaves.setdefault(index // RECORDS_PER_LEAF, []).append((index, rec))
        for leaf_i, recs in leaves.items():
            old = self.root[leaf_i]
            payload = bytearray(vol._meta(old, KIND_OBJLEAF)[:RECORDS_PER_LEAF * RECORD]
                                if old else bytes(RECORDS_PER_LEAF * RECORD))
            for index, rec in recs:
                at = (index % RECORDS_PER_LEAF) * RECORD
                payload[at:at + RECORD] = rec.pack()
            nb = self.alloc()
            writes.append((nb, ("leaf", bytes(payload))))
            self.root[leaf_i] = nb
            if old:
                self.free(old)
        root = self.alloc()
        writes.append((root, ("root", None)))
        if vol.root:
            self.free(vol.root)
        new_bitmaps = [self.alloc() for _ in range(vol.nbitmaps)]
        for b in vol.bitmap_blocks:
            self.free(b)
        # Final bitmap: this transaction's allocations, minus blocks the
        # new generation no longer references.
        final = bytearray(self.work)
        for b in self.pending_free:
            Volume._clear_bit(final, b)
        free_blocks = vol.total - sum(bin(x).count("1") for x in final)
        out = []
        for b, ptrs in self.nodes.items():
            out.append((b, seal(KIND_MAPNODE, b, struct.pack(f"<{PTRS}Q", *ptrs))))
        for b, entries in self.dirs.items():
            out.append((b, seal(KIND_DIRBLK, b, pack_dir(entries))))
        for b, content in self.data.items():
            out.append((b, bytes(content)))
        for b, (what, payload) in writes:
            if what == "leaf":
                out.append((b, seal(KIND_OBJLEAF, b, payload)))
            else:
                out.append((b, seal(KIND_OBJROOT, b, struct.pack(f"<{PTRS}Q", *self.root))))
        full = bytes(final) + bytes(vol.nbitmaps * PAYLOAD - len(final))
        for i, b in enumerate(new_bitmaps):
            out.append((b, seal(KIND_BITMAP, b, full[i * PAYLOAD:(i + 1) * PAYLOAD])))
        seq = vol.seq + 1
        rec = bytearray(SECTOR)
        struct.pack_into("<8sI4xQQQI4xQI", rec, 0, COMMIT_MAGIC, VERSION, seq, root, free_blocks,
                         self.used_objects, wall_us, vol.nbitmaps)
        struct.pack_into(f"<{vol.nbitmaps}Q", rec, 64, *new_bitmaps)
        struct.pack_into("<Q", rec, 504, fnv(rec[:504]))
        slot = 1 + seq % 2
        out.append((slot * BLOCK, bytes(rec)))  # byte offset marker: sector write
        # Issue: blocks first, the commit sector last.
        vol.last_writes = []
        for b, data in out[:-1]:
            vol.dev[b * BLOCK:(b + 1) * BLOCK] = data
            vol.last_writes.append((b * BLOCK, data))
        off, data = out[-1]
        vol.dev[off:off + SECTOR] = data
        vol.last_writes.append((off, data))
        vol.seq, vol.root, vol.free_blocks = seq, root, free_blocks
        vol.used_objects, vol.wall_us = self.used_objects, wall_us
        vol.bitmap_blocks, vol.bitmap = new_bitmaps, bytearray(final[:len(vol.bitmap)])
        vol.free_hint = self.hint
        # Cached blocks stay valid (copy-on-write); drop freed ones.
        for key in [k for k in vol._cache if k[0] in self.pending_free]:
            del vol._cache[key]


def _dir_entries_block(self, block):
    # Written blocks are immutable (copy-on-write): parse each once.
    key = (block, "parsed")
    if key not in self._cache:
        self._cache[key] = unpack_dir(self._meta(block, KIND_DIRBLK))
    return self._cache[key]


Volume.dir_entries_block = _dir_entries_block


def walk(vol):
    """Whole-namespace snapshot: {path: bytes | None (directory)}."""
    out = {}

    def rec(dir_id, prefix):
        out[prefix or "/"] = None
        after = b""
        while True:
            batch = vol.list(dir_id, after, 64)
            if not batch:
                break
            for name, child, typ in batch:
                path = prefix + "/" + name.decode()
                if typ == DIR:
                    rec(child, path)
                else:
                    out[path] = vol.read(child)
            after = batch[-1][0]

    rec(vol.root_id(), "")
    return out


def check(vol):
    """Independent structural audit: every referenced block allocated once,
    bitmap equals the reference set, parents and counts consistent."""
    seen = {}

    def claim(b, what):
        if b in seen:
            raise FsError("CORRUPT")
        seen[b] = what

    claim(vol.root, "objroot")
    for b in vol.bitmap_blocks:
        claim(b, "bitmap")
    ptrs = vol._root_ptrs()
    live = 0
    for leaf_i, leaf in enumerate(ptrs):
        if not leaf:
            continue
        claim(leaf, "objleaf")
        for j in range(RECORDS_PER_LEAF):
            index = leaf_i * RECORDS_PER_LEAF + j
            r = vol.stat_index(index)
            if r.type == 0:
                continue
            live += 1
            if r.depth == 1:
                claim(r.map, "map")
                for b in vol._node(r.map):
                    if b:
                        claim(b, "data")
            elif r.depth == 2:
                claim(r.map, "map")
                for c in vol._node(r.map):
                    if c:
                        claim(c, "map")
                        for b in vol._node(c):
                            if b:
                                claim(b, "data")
            if r.type == DIR:
                entries = 0
                for i in range(r.dir_blocks):
                    blk = vol._map_get(r, i)
                    seen[blk] = "dirblk"
                    for name, ci, cg, ct in vol.dir_entries_block(blk):
                        entries += 1
                        child = vol.stat_index(ci)
                        if child.type != ct or child.generation != cg or child.parent != oid(index, r.generation):
                            raise FsError("CORRUPT")
                if entries != r.entries:
                    raise FsError("CORRUPT")
    if live != vol.used_objects:
        raise FsError("CORRUPT")
    used = {b for b in range(vol.total) if Volume._bit(vol.bitmap, b)}
    if used != set(seen) | set(range(FIRST_DATA)):
        raise FsError("CORRUPT")
    return True
