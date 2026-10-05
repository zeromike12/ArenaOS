//! AFS2 (ADR-0076): copy-on-write hierarchical filesystem engine.
//!
//! `no_std`, no allocation, bounded memory. The on-disk format is defined
//! by ADR-0076 and its host model `tools/afs2.py`; volumes written here are
//! audited by that model (`check`) and volumes it writes mount here.
//!
//! Transactions. Every mutating operation is one transaction:
//!
//! 1. File data goes to freshly allocated blocks, written immediately.
//!    A fresh block is free in the committed bitmap, so nothing the
//!    committed generation references is ever written.
//! 2. Metadata (object leaves, map nodes, directory blocks) is copied on
//!    write into a bounded pool of dirty buffers, each keyed by a fresh
//!    block number.
//! 3. At commit: dirty metadata, a fresh object root and fresh bitmap
//!    blocks are written, then the one-sector commit record, last.
//!
//! A crash before the commit sector leaves exactly the previous commit; an
//! error during an operation abandons the transaction with the committed
//! state untouched. Capacity refusals are preflighted before anything is
//! written. Mount fails closed on any inconsistency in the newest commit.
#![no_std]

pub const BLOCK: usize = 4096;
pub const SECTOR: usize = 512;
const TRAILER: usize = 24;
pub const PAYLOAD: usize = BLOCK - TRAILER; // 4072
pub const PTRS: usize = PAYLOAD / 8; // 509
pub const RECORD: usize = 128;
pub const RECORDS_PER_LEAF: usize = PAYLOAD / RECORD; // 31
pub const BITS_PER_BITMAP: u64 = (PAYLOAD * 8) as u64; // 32576
pub const FORMAT_MAX_BITMAPS: usize = 52;
pub const MAX_OBJECTS: u32 = (PTRS * RECORDS_PER_LEAF) as u32; // 15779
pub const FIRST_DATA: u64 = 3;
pub const NAME_MAX: usize = 255;
pub const MAX_FILE_BLOCKS: u64 = (PTRS * PTRS) as u64;
/// This implementation mounts volumes of up to this many bitmap blocks
/// (130,304 blocks, 509 MiB); larger valid volumes are refused honestly.
pub const MAX_BITMAPS: usize = 4;
const BITMAP_BYTES: usize = MAX_BITMAPS * PAYLOAD;
/// Dirty metadata buffers of one transaction.
pub const DIRTY: usize = 64;
const CACHE: usize = 8;
const MAGIC: &[u8; 8] = b"ARENAFS2";
const COMMIT_MAGIC: &[u8; 8] = b"AFS2COMT";
const VERSION: u32 = 1;
const KIND_OBJROOT: u32 = 1;
const KIND_OBJLEAF: u32 = 2;
const KIND_MAPNODE: u32 = 3;
const KIND_DIRBLK: u32 = 4;
const KIND_BITMAP: u32 = 5;
pub const FILE: u8 = 1;
pub const DIR: u8 = 2;
const ENTRY_HEAD: usize = 10;
/// Entry bytes one directory block holds (after its 4-byte header).
const DIR_ROOM: usize = PAYLOAD - 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    NoEnt,
    Exist,
    NotDir,
    IsDir,
    NotEmpty,
    Inval,
    NoSpc,
    FBig,
    Loop,
    Corrupt,
    Io,
    /// The volume is larger than this implementation's static bounds.
    Unsupported,
}
pub type Result<T> = core::result::Result<T, Error>;

/// Block device of the volume region (block 0 = superblock).
pub trait Device {
    fn read(&mut self, block: u64, buf: &mut [u8; BLOCK]) -> Result<()>;
    fn write(&mut self, block: u64, buf: &[u8; BLOCK]) -> Result<()>;
    /// Write only the first sector of `block` (the commit record).
    fn write_sector(&mut self, block: u64, buf: &[u8; SECTOR]) -> Result<()>;
}

pub fn fnv(data: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in data {
        h = (h ^ u64::from(*b)).wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

pub const fn oid(index: u32, generation: u32) -> u64 {
    (generation as u64) << 32 | index as u64
}
pub const fn split(object: u64) -> (u32, u32) {
    (object as u32, (object >> 32) as u32)
}

/// True when the region was never committed: a blank superblock block,
/// or no valid commit record with a commit slot still all zero. `format`
/// zeroes both slots before it writes the superblock and no commit ever
/// zeroes a slot, so this holds exactly for an interrupted format (or a
/// region never formatted); such a region is formatted, never repaired.
/// Any volume that has committed refuses this and mounts or fails closed.
pub fn never_committed<D: Device>(dev: &mut D) -> Result<bool> {
    let mut raw = [0u8; BLOCK];
    dev.read(0, &mut raw)?;
    if raw.iter().all(|b| *b == 0) {
        return Ok(true);
    }
    let mut zero_slot = false;
    for slot in [1u64, 2] {
        dev.read(slot, &mut raw)?;
        let r = &raw[..SECTOR];
        if &r[..8] == COMMIT_MAGIC && le64(r, 504) == fnv(&r[..504]) {
            return Ok(false);
        }
        zero_slot |= raw.iter().all(|b| *b == 0);
    }
    Ok(zero_slot)
}

pub fn valid_name(name: &[u8]) -> bool {
    !name.is_empty()
        && name.len() <= NAME_MAX
        && name != b"."
        && name != b".."
        && !name.iter().any(|b| *b < 0x20 || *b == 0x7f || *b == b'/')
        && core::str::from_utf8(name).is_ok()
}

fn le64(b: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(b[at..at + 8].try_into().expect("8 bytes"))
}
fn le32(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().expect("4 bytes"))
}
fn le16(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes(b[at..at + 2].try_into().expect("2 bytes"))
}
fn put64(b: &mut [u8], at: usize, v: u64) {
    b[at..at + 8].copy_from_slice(&v.to_le_bytes());
}
fn put32(b: &mut [u8], at: usize, v: u32) {
    b[at..at + 4].copy_from_slice(&v.to_le_bytes());
}
fn put16(b: &mut [u8], at: usize, v: u16) {
    b[at..at + 2].copy_from_slice(&v.to_le_bytes());
}

/// Seal a metadata payload into a block: payload, trailer {kind, pad,
/// block, checksum over everything before the checksum}.
fn seal(kind: u32, block: u64, payload: &[u8; PAYLOAD], out: &mut [u8; BLOCK]) {
    out[..PAYLOAD].copy_from_slice(payload);
    put32(out, PAYLOAD, kind);
    put32(out, PAYLOAD + 4, 0);
    put64(out, PAYLOAD + 8, block);
    let c = fnv(&out[..BLOCK - 8]);
    put64(out, BLOCK - 8, c);
}
fn unseal(raw: &[u8; BLOCK], kind: u32, block: u64) -> Result<()> {
    if le32(raw, PAYLOAD) != kind
        || le64(raw, PAYLOAD + 8) != block
        || le64(raw, BLOCK - 8) != fnv(&raw[..BLOCK - 8])
    {
        return Err(Error::Corrupt);
    }
    Ok(())
}

/// One object record (128 bytes on disk; 72 used, the rest zero).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Record {
    pub typ: u8,
    pub depth: u8,
    pub flags: u16,
    pub generation: u32,
    pub parent: u64,
    pub size: u64,
    pub map: u64,
    pub ctime: u64,
    pub mtime: u64,
    pub entries: u32,
    pub dir_blocks: u32,
    pub version: u64,
}
impl Record {
    fn unpack(b: &[u8]) -> Result<Self> {
        let r = Record {
            typ: b[0],
            depth: b[1],
            flags: le16(b, 2),
            generation: le32(b, 4),
            parent: le64(b, 8),
            size: le64(b, 16),
            map: le64(b, 24),
            ctime: le64(b, 32),
            mtime: le64(b, 40),
            entries: le32(b, 56),
            dir_blocks: le32(b, 60),
            version: le64(b, 64),
        };
        if r.typ > DIR || r.depth > 2 || le64(b, 48) != 0 || b[72..RECORD].iter().any(|x| *x != 0) {
            return Err(Error::Corrupt);
        }
        Ok(r)
    }
    fn pack(&self, b: &mut [u8]) {
        b[..RECORD].fill(0);
        b[0] = self.typ;
        b[1] = self.depth;
        put16(b, 2, self.flags);
        put32(b, 4, self.generation);
        put64(b, 8, self.parent);
        put64(b, 16, self.size);
        put64(b, 24, self.map);
        put64(b, 32, self.ctime);
        put64(b, 40, self.mtime);
        put32(b, 56, self.entries);
        put32(b, 60, self.dir_blocks);
        put64(b, 64, self.version);
    }
}

/// What `stat` reports.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stat {
    pub typ: u8,
    pub size: u64,
    pub parent: u64,
    pub ctime: u64,
    pub mtime: u64,
    pub entries: u32,
    pub version: u64,
}

/// One listed directory entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Entry {
    pub name: [u8; NAME_MAX],
    pub len: u8,
    pub object: u64,
    pub typ: u8,
}
impl Entry {
    pub const EMPTY: Entry = Entry {
        name: [0; NAME_MAX],
        len: 0,
        object: 0,
        typ: 0,
    };
    pub fn name(&self) -> &[u8] {
        &self.name[..usize::from(self.len)]
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StatFs {
    pub blocks: u64,
    pub free: u64,
    pub objects: u32,
    pub max_objects: u32,
    pub seq: u64,
}

/// A directory entry found by name: (index, generation, type).
type Found = (u32, u32, u8);

#[derive(Clone, Copy)]
struct Slot {
    block: u64,
    kind: u32,
    data: [u8; PAYLOAD],
}
const EMPTY_SLOT: Slot = Slot {
    block: 0,
    kind: 0,
    data: [0; PAYLOAD],
};
#[derive(Clone, Copy)]
struct Cached {
    block: u64,
    data: [u8; BLOCK],
}
const EMPTY_CACHE: Cached = Cached {
    block: 0,
    data: [0; BLOCK],
};

/// A mounted volume. Large (a few hundred KiB): keep it in static memory.
pub struct Volume<D: Device> {
    dev: Option<D>,
    total: u64,
    nbitmaps: usize,
    volume_id: u64,
    seq: u64,
    root: u64,
    free_blocks: u64,
    used_objects: u32,
    wall_us: u64,
    bitmap_blocks: [u64; MAX_BITMAPS],
    bitmap: [u8; BITMAP_BYTES],
    cursor: u64,
    hint: u32,
    cache: [Cached; CACHE],
    cache_next: usize,
    // ---- transaction
    work: [u8; BITMAP_BYTES],
    fresh: [u8; BITMAP_BYTES],
    pending: [u8; BITMAP_BYTES],
    tx_root: [u64; PTRS],
    slots: [Slot; DIRTY],
    tx_used: u32,
    tx_free: u64,
    tx_hint: u32,
    /// Device writes issued by the last operation (tests, receipts).
    pub writes: u64,
    /// Adversarial allocation order for the crash proofs (which must hold
    /// for any order): first try the blocks this transaction freed most
    /// recently, then the lowest free block. Correct code never hands them
    /// out (the committed generation still references them); an early-free
    /// bug would reuse one at once. Never needed for correctness.
    pub lowest_first: bool,
    recent: [u64; 8],
}

fn bit(bm: &[u8], b: u64) -> bool {
    bm[(b >> 3) as usize] >> (b & 7) & 1 == 1
}
fn set(bm: &mut [u8], b: u64) {
    bm[(b >> 3) as usize] |= 1 << (b & 7);
}
fn clear(bm: &mut [u8], b: u64) {
    bm[(b >> 3) as usize] &= !(1 << (b & 7));
}

impl<D: Device> Volume<D> {
    pub const fn empty() -> Self {
        Volume {
            dev: None,
            total: 0,
            nbitmaps: 0,
            volume_id: 0,
            seq: 0,
            root: 0,
            free_blocks: 0,
            used_objects: 0,
            wall_us: 0,
            bitmap_blocks: [0; MAX_BITMAPS],
            bitmap: [0; BITMAP_BYTES],
            cursor: FIRST_DATA,
            hint: 1,
            cache: [EMPTY_CACHE; CACHE],
            cache_next: 0,
            work: [0; BITMAP_BYTES],
            fresh: [0; BITMAP_BYTES],
            pending: [0; BITMAP_BYTES],
            tx_root: [0; PTRS],
            slots: [EMPTY_SLOT; DIRTY],
            tx_used: 0,
            tx_free: 0,
            tx_hint: 1,
            writes: 0,
            lowest_first: false,
            recent: [0; 8],
        }
    }

    /// `*self = Self::empty()` with `dev`, in place: the volume is far too
    /// large (metadata pool, cache, bitmaps) to build as a stack temporary
    /// in a ring-3 service.
    fn reset(&mut self, dev: D) {
        self.dev = Some(dev);
        self.total = 0;
        self.nbitmaps = 0;
        self.volume_id = 0;
        self.seq = 0;
        self.root = 0;
        self.free_blocks = 0;
        self.used_objects = 0;
        self.wall_us = 0;
        self.bitmap_blocks.fill(0);
        self.bitmap.fill(0);
        self.cursor = FIRST_DATA;
        self.hint = 1;
        for c in self.cache.iter_mut() {
            c.block = 0;
            c.data.fill(0);
        }
        self.cache_next = 0;
        self.work.fill(0);
        self.fresh.fill(0);
        self.pending.fill(0);
        self.tx_root.fill(0);
        for sl in self.slots.iter_mut() {
            sl.block = 0;
            sl.kind = 0;
            sl.data.fill(0);
        }
        self.tx_used = 0;
        self.tx_free = 0;
        self.tx_hint = 1;
        self.writes = 0;
        self.recent = [0; 8];
    }

    pub fn device(&mut self) -> Option<&mut D> {
        self.dev.as_mut()
    }

    fn dev(&mut self) -> Result<&mut D> {
        self.dev.as_mut().ok_or(Error::Io)
    }

    // ---- raw block access ------------------------------------------------

    fn raw_read(&mut self, block: u64, out: &mut [u8; BLOCK]) -> Result<()> {
        if let Some(c) = self.cache.iter().find(|c| c.block == block && block != 0) {
            out.copy_from_slice(&c.data);
            return Ok(());
        }
        self.dev()?.read(block, out)?;
        let i = self.cache_next;
        self.cache_next = (i + 1) % CACHE;
        self.cache[i].block = block;
        self.cache[i].data.copy_from_slice(out);
        Ok(())
    }

    fn raw_write(&mut self, block: u64, data: &[u8; BLOCK]) -> Result<()> {
        for c in self.cache.iter_mut().filter(|c| c.block == block) {
            c.block = 0;
        }
        self.writes += 1;
        self.dev()?.write(block, data)
    }

    /// Payload of committed metadata block `block`, verified.
    fn committed_meta(&mut self, block: u64, kind: u32, out: &mut [u8; PAYLOAD]) -> Result<()> {
        if !(FIRST_DATA..self.total).contains(&block) {
            return Err(Error::Corrupt);
        }
        let mut raw = [0u8; BLOCK];
        self.raw_read(block, &mut raw)?;
        unseal(&raw, kind, block)?;
        out.copy_from_slice(&raw[..PAYLOAD]);
        Ok(())
    }

    fn slot_of(&self, block: u64) -> Option<usize> {
        if block == 0 {
            return None;
        }
        self.slots.iter().position(|s| s.block == block)
    }

    /// Payload of metadata block `block` as this transaction sees it.
    fn meta(&mut self, block: u64, kind: u32, out: &mut [u8; PAYLOAD]) -> Result<()> {
        if let Some(i) = self.slot_of(block) {
            if self.slots[i].kind != kind {
                return Err(Error::Corrupt);
            }
            out.copy_from_slice(&self.slots[i].data);
            return Ok(());
        }
        self.committed_meta(block, kind, out)
    }

    fn ptr(&mut self, block: u64, index: usize) -> Result<u64> {
        if let Some(i) = self.slot_of(block) {
            return Ok(le64(&self.slots[i].data, index * 8));
        }
        let mut p = [0u8; PAYLOAD];
        self.committed_meta(block, KIND_MAPNODE, &mut p)?;
        Ok(le64(&p, index * 8))
    }

    // ---- mount / format --------------------------------------------------------

    /// Mount `dev` (fail-closed: any inconsistency of the newest valid
    /// commit is corruption, never a silent fallback).
    pub fn mount(&mut self, dev: D) -> Result<()> {
        self.reset(dev);
        let mut raw = [0u8; BLOCK];
        self.dev()?.read(0, &mut raw)?;
        let sb = &raw[..SECTOR];
        if &sb[..8] != MAGIC
            || le32(sb, 8) != VERSION
            || le32(sb, 12) != BLOCK as u32
            || le32(sb, 28) != MAX_OBJECTS
            || le64(sb, 504) != fnv(&sb[..504])
        {
            return Err(Error::Corrupt);
        }
        let total = le64(sb, 16);
        let nbitmaps = le32(sb, 24) as usize;
        if total < 64 || nbitmaps as u64 != total.div_ceil(BITS_PER_BITMAP) {
            return Err(Error::Corrupt);
        }
        if nbitmaps > MAX_BITMAPS {
            return Err(Error::Unsupported);
        }
        self.total = total;
        self.nbitmaps = nbitmaps;
        self.volume_id = le64(sb, 32);
        let mut best: Option<(u64, [u8; SECTOR])> = None;
        for slot in [1u64, 2] {
            self.dev()?.read(slot, &mut raw)?;
            let r: [u8; SECTOR] = raw[..SECTOR].try_into().expect("sector");
            if &r[..8] != COMMIT_MAGIC || le64(&r, 504) != fnv(&r[..504]) {
                continue;
            }
            let seq = le64(&r, 16);
            if le32(&r, 8) != VERSION || le32(&r, 56) as usize != nbitmaps || slot != 1 + seq % 2 {
                continue;
            }
            if best.is_none_or(|(s, _)| seq > s) {
                best = Some((seq, r));
            }
        }
        let (seq, r) = best.ok_or(Error::Corrupt)?;
        self.seq = seq;
        self.root = le64(&r, 24);
        self.free_blocks = le64(&r, 32);
        self.used_objects = le32(&r, 40);
        self.wall_us = le64(&r, 48);
        let mut payload = [0u8; PAYLOAD];
        for i in 0..nbitmaps {
            let b = le64(&r, 64 + i * 8);
            self.bitmap_blocks[i] = b;
            self.committed_meta(b, KIND_BITMAP, &mut payload)?;
            self.bitmap[i * PAYLOAD..(i + 1) * PAYLOAD].copy_from_slice(&payload);
        }
        // Bits past the end of the volume must be clear.
        let bytes = total.div_ceil(8) as usize;
        if self.bitmap[bytes..].iter().any(|b| *b != 0)
            || (!total.is_multiple_of(8) && self.bitmap[bytes - 1] >> (total % 8) != 0)
        {
            return Err(Error::Corrupt);
        }
        self.committed_meta(self.root, KIND_OBJROOT, &mut payload)?;
        let used: u64 = self.bitmap[..bytes]
            .iter()
            .map(|b| u64::from(b.count_ones()))
            .sum();
        if self.free_blocks != total - used || (0..FIRST_DATA).any(|b| !bit(&self.bitmap, b)) {
            return Err(Error::Corrupt);
        }
        self.begin();
        let root = self.get_index(0)?;
        if root.typ != DIR {
            return Err(Error::Corrupt);
        }
        Ok(())
    }

    /// Format `dev` as a fresh volume of `total` blocks and mount it.
    /// Not crash-atomic (a format is not a transaction): both commit slots
    /// are cleared first, so a torn format never mounts as a volume.
    pub fn format(&mut self, dev: D, total: u64, volume_id: u64, wall_us: u64) -> Result<()> {
        if total < 64 || total > MAX_BITMAPS as u64 * BITS_PER_BITMAP {
            return Err(Error::Inval);
        }
        self.reset(dev);
        let zero = [0u8; SECTOR];
        self.dev()?.write_sector(1, &zero)?;
        self.dev()?.write_sector(2, &zero)?;
        let nbitmaps = total.div_ceil(BITS_PER_BITMAP) as usize;
        let mut sb = [0u8; BLOCK];
        sb[..8].copy_from_slice(MAGIC);
        put32(&mut sb, 8, VERSION);
        put32(&mut sb, 12, BLOCK as u32);
        put64(&mut sb, 16, total);
        put32(&mut sb, 24, nbitmaps as u32);
        put32(&mut sb, 28, MAX_OBJECTS);
        put64(&mut sb, 32, volume_id);
        let c = fnv(&sb[..504]);
        put64(&mut sb, 504, c);
        self.raw_write(0, &sb)?;
        self.total = total;
        self.nbitmaps = nbitmaps;
        self.volume_id = volume_id;
        for b in 0..FIRST_DATA {
            set(&mut self.bitmap, b);
        }
        self.free_blocks = total - FIRST_DATA;
        self.begin();
        let rec = Record {
            typ: DIR,
            generation: 1,
            parent: oid(0, 1),
            ctime: wall_us,
            mtime: wall_us,
            version: 1,
            ..Record::default()
        };
        self.put_index(0, &rec)?;
        self.tx_used = 1;
        self.commit(wall_us)
    }

    // ---- transaction state -----------------------------------------------------------

    fn begin(&mut self) {
        self.work = self.bitmap;
        self.fresh = [0; BITMAP_BYTES];
        self.pending = [0; BITMAP_BYTES];
        self.recent = [0; 8];
        for s in self.slots.iter_mut() {
            s.block = 0;
        }
        self.tx_used = self.used_objects;
        self.tx_free = self.free_blocks;
        self.tx_hint = self.hint;
        self.tx_root = [0; PTRS];
        if self.root != 0 {
            let mut p = [0u8; PAYLOAD];
            if self.committed_meta(self.root, KIND_OBJROOT, &mut p).is_ok() {
                for (i, r) in self.tx_root.iter_mut().enumerate() {
                    *r = le64(&p, i * 8);
                }
            }
        }
        self.writes = 0;
    }

    /// Run one transaction: commit on success, abandon on error.
    fn run<T>(&mut self, wall_us: u64, f: impl FnOnce(&mut Self) -> Result<T>) -> Result<T> {
        if self.dev.is_none() {
            return Err(Error::Io);
        }
        self.begin();
        match f(self) {
            Ok(v) => {
                self.commit(wall_us)?;
                Ok(v)
            }
            Err(e) => {
                self.begin();
                Err(e)
            }
        }
    }

    fn alloc(&mut self) -> Result<u64> {
        let span = self.total - FIRST_DATA;
        if self.lowest_first {
            for r in self.recent {
                if r != 0 && !bit(&self.work, r) {
                    set(&mut self.work, r);
                    set(&mut self.fresh, r);
                    self.tx_free -= 1;
                    return Ok(r);
                }
            }
            self.cursor = FIRST_DATA;
        }
        for i in 0..span {
            let b = FIRST_DATA + (self.cursor - FIRST_DATA + i) % span;
            if !bit(&self.work, b) {
                set(&mut self.work, b);
                set(&mut self.fresh, b);
                self.cursor = b + 1;
                if self.cursor >= self.total {
                    self.cursor = FIRST_DATA;
                }
                self.tx_free -= 1;
                return Ok(b);
            }
        }
        Err(Error::NoSpc)
    }

    fn free(&mut self, block: u64) {
        if block == 0 {
            return;
        }
        if bit(&self.fresh, block) {
            // Never referenced by the committed generation: reusable now.
            clear(&mut self.fresh, block);
            clear(&mut self.work, block);
            if let Some(i) = self.slot_of(block) {
                self.slots[i].block = 0;
            }
            self.tx_free += 1;
        } else {
            // The committed generation references it until commit.
            set(&mut self.pending, block);
            self.recent.rotate_right(1);
            self.recent[0] = block;
        }
    }

    /// A dirty copy of metadata block `block` (0 = a new zeroed block),
    /// returned as the fresh block number holding it.
    fn own(&mut self, block: u64, kind: u32) -> Result<u64> {
        if let Some(i) = self.slot_of(block) {
            if self.slots[i].kind != kind {
                return Err(Error::Corrupt);
            }
            return Ok(block);
        }
        let i = self
            .slots
            .iter()
            .position(|s| s.block == 0)
            .ok_or(Error::NoSpc)?;
        let mut payload = [0u8; PAYLOAD];
        if block != 0 {
            self.committed_meta(block, kind, &mut payload)?;
        }
        let nb = self.alloc()?;
        self.slots[i] = Slot {
            block: nb,
            kind,
            data: payload,
        };
        self.free(block);
        Ok(nb)
    }

    fn slot_mut(&mut self, block: u64) -> &mut [u8; PAYLOAD] {
        let i = self.slot_of(block).expect("owned block has a dirty slot");
        &mut self.slots[i].data
    }

    // ---- records -------------------------------------------------------------------------

    fn get_index(&mut self, index: u32) -> Result<Record> {
        if index >= MAX_OBJECTS {
            return Err(Error::NoEnt);
        }
        let leaf = self.tx_root[index as usize / RECORDS_PER_LEAF];
        if leaf == 0 {
            return Ok(Record::default());
        }
        let mut p = [0u8; PAYLOAD];
        self.meta(leaf, KIND_OBJLEAF, &mut p)?;
        let at = (index as usize % RECORDS_PER_LEAF) * RECORD;
        Record::unpack(&p[at..at + RECORD])
    }

    fn put_index(&mut self, index: u32, rec: &Record) -> Result<()> {
        let li = index as usize / RECORDS_PER_LEAF;
        let leaf = self.own(self.tx_root[li], KIND_OBJLEAF)?;
        self.tx_root[li] = leaf;
        let at = (index as usize % RECORDS_PER_LEAF) * RECORD;
        rec.pack(&mut self.slot_mut(leaf)[at..at + RECORD]);
        Ok(())
    }

    fn get(&mut self, object: u64) -> Result<Record> {
        let (index, generation) = split(object);
        let r = self.get_index(index)?;
        if r.typ == 0 || r.generation != generation {
            return Err(Error::NoEnt);
        }
        Ok(r)
    }

    fn put(&mut self, object: u64, rec: &Record) -> Result<()> {
        self.put_index(split(object).0, rec)
    }

    // ---- block maps -------------------------------------------------------------------------

    fn map_get(&mut self, rec: &Record, logical: u64) -> Result<u64> {
        match rec.depth {
            0 => Ok(0),
            1 => {
                if logical >= PTRS as u64 {
                    Ok(0)
                } else {
                    self.ptr(rec.map, logical as usize)
                }
            }
            _ => {
                let top = self.ptr(rec.map, (logical / PTRS as u64) as usize)?;
                if top == 0 {
                    Ok(0)
                } else {
                    self.ptr(top, (logical % PTRS as u64) as usize)
                }
            }
        }
    }

    fn map_set(&mut self, rec: &mut Record, logical: u64, block: u64) -> Result<()> {
        if logical >= MAX_FILE_BLOCKS {
            return Err(Error::FBig);
        }
        if rec.depth == 0 {
            if block == 0 {
                return Ok(());
            }
            rec.map = self.own(0, KIND_MAPNODE)?;
            rec.depth = 1;
        }
        if rec.depth == 1 && logical >= PTRS as u64 {
            if block == 0 {
                return Ok(());
            }
            let top = self.own(0, KIND_MAPNODE)?;
            let old = rec.map;
            put64(self.slot_mut(top), 0, old);
            rec.map = top;
            rec.depth = 2;
        }
        rec.map = self.own(rec.map, KIND_MAPNODE)?;
        if rec.depth == 1 {
            put64(self.slot_mut(rec.map), logical as usize * 8, block);
            return Ok(());
        }
        let ci = (logical / PTRS as u64) as usize;
        let child = le64(self.slot_mut(rec.map), ci * 8);
        if child == 0 && block == 0 {
            return Ok(());
        }
        let child = self.own(child, KIND_MAPNODE)?;
        put64(self.slot_mut(rec.map), ci * 8, child);
        put64(
            self.slot_mut(child),
            (logical % PTRS as u64) as usize * 8,
            block,
        );
        Ok(())
    }

    /// Free every data block at logical >= `keep` and nodes left empty.
    fn drop_map(&mut self, rec: &mut Record, keep: u64) -> Result<()> {
        match rec.depth {
            0 => {}
            1 => {
                let mut owned = false;
                for i in keep.min(PTRS as u64)..PTRS as u64 {
                    let b = self.ptr(rec.map, i as usize)?;
                    if b != 0 {
                        if !owned {
                            rec.map = self.own(rec.map, KIND_MAPNODE)?;
                            owned = true;
                        }
                        self.free(b);
                        put64(self.slot_mut(rec.map), i as usize * 8, 0);
                    }
                }
                if keep == 0 {
                    self.free(rec.map);
                    rec.depth = 0;
                    rec.map = 0;
                }
            }
            _ => {
                for c in 0..PTRS {
                    let child = self.ptr(rec.map, c)?;
                    if child == 0 {
                        continue;
                    }
                    let first = (c * PTRS) as u64;
                    if first + PTRS as u64 <= keep {
                        continue;
                    }
                    if keep <= first {
                        for i in 0..PTRS {
                            let b = self.ptr(child, i)?;
                            self.free(b);
                        }
                        rec.map = self.own(rec.map, KIND_MAPNODE)?;
                        self.free(child);
                        put64(self.slot_mut(rec.map), c * 8, 0);
                        continue;
                    }
                    let nc = self.own(child, KIND_MAPNODE)?;
                    rec.map = self.own(rec.map, KIND_MAPNODE)?;
                    put64(self.slot_mut(rec.map), c * 8, nc);
                    for i in (keep - first) as usize..PTRS {
                        let b = le64(self.slot_mut(nc), i * 8);
                        if b != 0 {
                            self.free(b);
                            put64(self.slot_mut(nc), i * 8, 0);
                        }
                    }
                }
                if keep == 0 {
                    self.free(rec.map);
                    rec.depth = 0;
                    rec.map = 0;
                }
            }
        }
        Ok(())
    }

    /// Blocks a file of `blocks` logical blocks may need for its map.
    fn map_blocks(blocks: u64) -> u64 {
        if blocks <= PTRS as u64 {
            1
        } else {
            1 + blocks.div_ceil(PTRS as u64)
        }
    }

    /// Refuse before writing anything when the volume cannot hold `blocks`
    /// more blocks plus commit overhead (leaves, root, bitmaps).
    fn preflight(&self, blocks: u64) -> Result<()> {
        let overhead = 8 + self.nbitmaps as u64;
        if self.tx_free < blocks + overhead {
            return Err(Error::NoSpc);
        }
        Ok(())
    }

    // ---- directories -----------------------------------------------------------------------

    /// Entries of directory block `block`: calls `f(name, index, gen, typ)`
    /// in order; stops when `f` returns false.
    fn each_entry(
        &mut self,
        block: u64,
        mut f: impl FnMut(&[u8], u32, u32, u8) -> bool,
    ) -> Result<()> {
        let mut p = [0u8; PAYLOAD];
        self.meta(block, KIND_DIRBLK, &mut p)?;
        let count = le16(&p, 0) as usize;
        let used = le16(&p, 2) as usize;
        let mut at = 4;
        let mut prev: [u8; NAME_MAX] = [0; NAME_MAX];
        let mut prev_len = 0usize;
        for k in 0..count {
            if at + ENTRY_HEAD > PAYLOAD {
                return Err(Error::Corrupt);
            }
            let index = le32(&p, at);
            let generation = le32(&p, at + 4);
            let typ = p[at + 8];
            let n = p[at + 9] as usize;
            let name = p
                .get(at + ENTRY_HEAD..at + ENTRY_HEAD + n)
                .ok_or(Error::Corrupt)?;
            if (typ != FILE && typ != DIR)
                || !valid_name(name)
                || (k > 0 && name <= &prev[..prev_len])
            {
                return Err(Error::Corrupt);
            }
            prev[..n].copy_from_slice(name);
            prev_len = n;
            at += ENTRY_HEAD + n;
            if !f(name, index, generation, typ) {
                return Ok(());
            }
        }
        if at != used || p[at..].iter().any(|x| *x != 0) {
            return Err(Error::Corrupt);
        }
        Ok(())
    }

    /// Position of `name` in directory `rec`: (logical block, found entry
    /// (index, gen, typ)) where the block is the one that holds or should
    /// hold it.
    fn dir_find(&mut self, rec: &Record, name: &[u8]) -> Result<(u32, Option<Found>)> {
        let n = rec.dir_blocks;
        for i in 0..n {
            let b = self.map_get(rec, u64::from(i))?;
            let mut hit = None;
            let mut last_ge = false;
            self.each_entry(b, |e, index, generation, typ| {
                if e == name {
                    hit = Some((index, generation, typ));
                }
                last_ge = e >= name;
                true
            })?;
            if hit.is_some() || last_ge || i + 1 == n {
                return Ok((i, hit));
            }
        }
        Ok((0, None))
    }

    pub fn lookup(&mut self, dir: u64, name: &[u8]) -> Result<(u64, u8)> {
        self.begin();
        let rec = self.get(dir)?;
        if rec.typ != DIR {
            return Err(Error::NotDir);
        }
        match self.dir_find(&rec, name)?.1 {
            Some((index, generation, typ)) => Ok((oid(index, generation), typ)),
            None => Err(Error::NoEnt),
        }
    }

    /// Entries strictly after `after` (a name cursor), at most `out.len()`.
    pub fn list(&mut self, dir: u64, after: &[u8], out: &mut [Entry]) -> Result<usize> {
        self.begin();
        let rec = self.get(dir)?;
        if rec.typ != DIR {
            return Err(Error::NotDir);
        }
        let mut n = 0;
        for i in 0..rec.dir_blocks {
            if n == out.len() {
                break;
            }
            let b = self.map_get(&rec, u64::from(i))?;
            self.each_entry(b, |name, index, generation, typ| {
                if name > after && n < out.len() {
                    let e = &mut out[n];
                    e.name[..name.len()].copy_from_slice(name);
                    e.len = name.len() as u8;
                    e.object = oid(index, generation);
                    e.typ = typ;
                    n += 1;
                }
                n < out.len()
            })?;
        }
        Ok(n)
    }

    /// Insert an entry into directory `rec` (kept globally sorted; a full
    /// block splits in two).
    fn dir_insert(
        &mut self,
        rec: &mut Record,
        name: &[u8],
        index: u32,
        generation: u32,
        typ: u8,
    ) -> Result<()> {
        if rec.dir_blocks == 0 {
            let b = self.own(0, KIND_DIRBLK)?;
            let p = self.slot_mut(b);
            single_entry(p, name, index, generation, typ);
            self.map_set(rec, 0, b)?;
            rec.dir_blocks = 1;
        } else {
            let (k, _) = self.dir_find(rec, name)?;
            let old = self.map_get(rec, u64::from(k))?;
            let b = self.own(old, KIND_DIRBLK)?;
            if b != old {
                self.map_set(rec, u64::from(k), b)?;
            }
            let mut p = *self.slot_mut(b);
            let used = le16(&p, 2) as usize;
            if used + ENTRY_HEAD + name.len() <= PAYLOAD {
                insert_entry(&mut p, name, index, generation, typ);
                *self.slot_mut(b) = p;
            } else {
                // Split: entries in name order, the new one included; the
                // lower half stays, the upper half moves to a new block
                // inserted right after this one.
                let (low, high) = split_block(&p, name, index, generation, typ)?;
                *self.slot_mut(b) = low;
                let nb = self.own(0, KIND_DIRBLK)?;
                *self.slot_mut(nb) = high;
                let count = rec.dir_blocks;
                let mut l = count;
                while l > k + 1 {
                    let prev = self.map_get(rec, u64::from(l - 1))?;
                    self.map_set(rec, u64::from(l), prev)?;
                    l -= 1;
                }
                self.map_set(rec, u64::from(k + 1), nb)?;
                rec.dir_blocks += 1;
            }
        }
        rec.entries += 1;
        rec.size = u64::from(rec.dir_blocks) * BLOCK as u64;
        Ok(())
    }

    /// Remove `name` from directory `rec`; an emptied block is dropped.
    fn dir_remove(&mut self, rec: &mut Record, name: &[u8]) -> Result<()> {
        let (k, hit) = self.dir_find(rec, name)?;
        if hit.is_none() {
            return Err(Error::NoEnt);
        }
        let old = self.map_get(rec, u64::from(k))?;
        let b = self.own(old, KIND_DIRBLK)?;
        if b != old {
            self.map_set(rec, u64::from(k), b)?;
        }
        let mut p = *self.slot_mut(b);
        remove_entry(&mut p, name)?;
        let empty = le16(&p, 0) == 0;
        *self.slot_mut(b) = p;
        rec.entries -= 1;
        if empty {
            self.free(b);
            let count = rec.dir_blocks;
            for l in k..count - 1 {
                let next = self.map_get(rec, u64::from(l + 1))?;
                self.map_set(rec, u64::from(l), next)?;
            }
            self.map_set(rec, u64::from(count - 1), 0)?;
            rec.dir_blocks -= 1;
            if rec.dir_blocks == 0 {
                self.drop_map(rec, 0)?;
            }
        }
        rec.size = u64::from(rec.dir_blocks) * BLOCK as u64;
        Ok(())
    }

    fn free_index(&mut self) -> Result<u32> {
        if self.tx_used >= MAX_OBJECTS {
            return Err(Error::NoSpc);
        }
        for index in self.tx_hint.max(1)..MAX_OBJECTS {
            if self.get_index(index)?.typ == 0 {
                self.tx_hint = index + 1;
                return Ok(index);
            }
        }
        Err(Error::NoSpc)
    }

    // ---- public operations ------------------------------------------------------------------

    pub fn root_id(&mut self) -> Result<u64> {
        self.begin();
        Ok(oid(0, self.get_index(0)?.generation))
    }

    pub fn statfs(&self) -> StatFs {
        StatFs {
            blocks: self.total,
            free: self.free_blocks,
            objects: self.used_objects,
            max_objects: MAX_OBJECTS,
            seq: self.seq,
        }
    }

    pub fn stat(&mut self, object: u64) -> Result<Stat> {
        self.begin();
        let r = self.get(object)?;
        Ok(Stat {
            typ: r.typ,
            size: r.size,
            parent: r.parent,
            ctime: r.ctime,
            mtime: r.mtime,
            entries: r.entries,
            version: r.version,
        })
    }

    /// Read file bytes at `offset` into `out`; returns the count.
    pub fn read(&mut self, object: u64, offset: u64, out: &mut [u8]) -> Result<usize> {
        self.begin();
        let rec = self.get(object)?;
        if rec.typ != FILE {
            return Err(Error::IsDir);
        }
        let end = rec.size.min(offset.saturating_add(out.len() as u64));
        let mut pos = offset;
        let mut buf = [0u8; BLOCK];
        while pos < end {
            let logical = pos / BLOCK as u64;
            let within = (pos % BLOCK as u64) as usize;
            let n = (BLOCK - within).min((end - pos) as usize);
            let b = self.map_get(&rec, logical)?;
            let at = (pos - offset) as usize;
            if b == 0 {
                out[at..at + n].fill(0);
            } else {
                if !(FIRST_DATA..self.total).contains(&b) {
                    return Err(Error::Corrupt);
                }
                self.raw_read(b, &mut buf)?;
                out[at..at + n].copy_from_slice(&buf[within..within + n]);
            }
            pos += n as u64;
        }
        Ok(end.saturating_sub(offset) as usize)
    }

    fn new_object(&mut self, dir: u64, name: &[u8], typ: u8, wall_us: u64) -> Result<u64> {
        if !valid_name(name) {
            return Err(Error::Inval);
        }
        let mut parent = self.get(dir)?;
        if parent.typ != DIR {
            return Err(Error::NotDir);
        }
        if self.dir_find(&parent, name)?.1.is_some() {
            return Err(Error::Exist);
        }
        self.preflight(8)?;
        let index = self.free_index()?;
        let old = self.get_index(index)?;
        let generation = old.generation.wrapping_add(1).max(1);
        let rec = Record {
            typ,
            generation,
            parent: dir,
            ctime: wall_us,
            mtime: wall_us,
            version: 1,
            ..Record::default()
        };
        self.put_index(index, &rec)?;
        self.tx_used += 1;
        self.dir_insert(&mut parent, name, index, generation, typ)?;
        parent.mtime = wall_us;
        parent.version += 1;
        self.put(dir, &parent)?;
        Ok(oid(index, generation))
    }

    pub fn create(&mut self, dir: u64, name: &[u8], wall_us: u64) -> Result<u64> {
        self.run(wall_us, |v| v.new_object(dir, name, FILE, wall_us))
    }

    pub fn mkdir(&mut self, dir: u64, name: &[u8], wall_us: u64) -> Result<u64> {
        self.run(wall_us, |v| v.new_object(dir, name, DIR, wall_us))
    }

    pub fn write(&mut self, object: u64, offset: u64, data: &[u8], wall_us: u64) -> Result<usize> {
        self.run(wall_us, |v| v.write_tx(object, offset, data, wall_us))
    }

    fn write_tx(&mut self, object: u64, offset: u64, data: &[u8], wall_us: u64) -> Result<usize> {
        let mut rec = self.get(object)?;
        if rec.typ != FILE {
            return Err(Error::IsDir);
        }
        let end = offset.checked_add(data.len() as u64).ok_or(Error::FBig)?;
        if end.div_ceil(BLOCK as u64) > MAX_FILE_BLOCKS {
            return Err(Error::FBig);
        }
        let first = offset / BLOCK as u64;
        let touched = end.div_ceil(BLOCK as u64).saturating_sub(first);
        self.preflight(touched + Self::map_blocks(end.div_ceil(BLOCK as u64)) + 2)?;
        let mut pos = offset;
        let mut buf = [0u8; BLOCK];
        while pos < end {
            let logical = pos / BLOCK as u64;
            let within = (pos % BLOCK as u64) as usize;
            let n = (BLOCK - within).min((end - pos) as usize);
            let old = self.map_get(&rec, logical)?;
            let src = (pos - offset) as usize;
            if within == 0 && n == BLOCK {
                buf.copy_from_slice(&data[src..src + n]);
            } else {
                if old == 0 {
                    buf.fill(0);
                } else {
                    self.raw_read(old, &mut buf)?;
                    // Bytes beyond the size of a partial old block are never
                    // visible; keep them zero so holes stay holes.
                    let base = logical * BLOCK as u64;
                    if rec.size < base + BLOCK as u64 {
                        let tail = rec.size.saturating_sub(base) as usize;
                        buf[tail..].fill(0);
                    }
                }
                buf[within..within + n].copy_from_slice(&data[src..src + n]);
            }
            let nb = self.alloc()?;
            self.raw_write(nb, &buf)?;
            self.map_set(&mut rec, logical, nb)?;
            self.free(old);
            pos += n as u64;
        }
        rec.size = rec.size.max(end);
        rec.mtime = wall_us;
        rec.version += 1;
        self.put(object, &rec)?;
        Ok(data.len())
    }

    pub fn truncate(&mut self, object: u64, size: u64, wall_us: u64) -> Result<()> {
        self.run(wall_us, |v| {
            let mut rec = v.get(object)?;
            if rec.typ != FILE {
                return Err(Error::IsDir);
            }
            if size.div_ceil(BLOCK as u64) > MAX_FILE_BLOCKS {
                return Err(Error::FBig);
            }
            v.preflight(4)?;
            if size < rec.size {
                let keep = size.div_ceil(BLOCK as u64);
                v.drop_map(&mut rec, keep)?;
                if !size.is_multiple_of(BLOCK as u64) && rec.depth != 0 {
                    let last = v.map_get(&rec, size / BLOCK as u64)?;
                    if last != 0 {
                        let mut buf = [0u8; BLOCK];
                        v.raw_read(last, &mut buf)?;
                        buf[(size % BLOCK as u64) as usize..].fill(0);
                        let nb = v.alloc()?;
                        v.raw_write(nb, &buf)?;
                        v.map_set(&mut rec, size / BLOCK as u64, nb)?;
                        v.free(last);
                    }
                }
            }
            rec.size = size;
            rec.mtime = wall_us;
            rec.version += 1;
            v.put(object, &rec)
        })
    }

    fn remove(&mut self, dir: u64, name: &[u8], typ: u8, wall_us: u64) -> Result<()> {
        let mut parent = self.get(dir)?;
        if parent.typ != DIR {
            return Err(Error::NotDir);
        }
        let (_, hit) = self.dir_find(&parent, name)?;
        let (index, generation, _) = hit.ok_or(Error::NoEnt)?;
        let child_id = oid(index, generation);
        let mut child = self.get(child_id)?;
        if child.typ != typ {
            return Err(if child.typ == DIR {
                Error::IsDir
            } else {
                Error::NotDir
            });
        }
        if typ == DIR && child.entries != 0 {
            return Err(Error::NotEmpty);
        }
        self.preflight(4)?;
        self.drop_map(&mut child, 0)?;
        self.put_index(
            index,
            &Record {
                generation,
                ..Record::default()
            },
        )?;
        self.tx_hint = self.tx_hint.min(index);
        self.tx_used -= 1;
        self.dir_remove(&mut parent, name)?;
        parent.mtime = wall_us;
        parent.version += 1;
        self.put(dir, &parent)
    }

    pub fn unlink(&mut self, dir: u64, name: &[u8], wall_us: u64) -> Result<()> {
        self.run(wall_us, |v| v.remove(dir, name, FILE, wall_us))
    }

    pub fn rmdir(&mut self, dir: u64, name: &[u8], wall_us: u64) -> Result<()> {
        self.run(wall_us, |v| v.remove(dir, name, DIR, wall_us))
    }

    /// Atomic rename/move; refuses an existing target and moving a
    /// directory into itself or a descendant.
    pub fn rename(
        &mut self,
        src_dir: u64,
        name: &[u8],
        dst_dir: u64,
        new_name: &[u8],
        wall_us: u64,
    ) -> Result<u64> {
        self.run(wall_us, |v| {
            if !valid_name(new_name) {
                return Err(Error::Inval);
            }
            let mut src = v.get(src_dir)?;
            let dst = v.get(dst_dir)?;
            if src.typ != DIR || dst.typ != DIR {
                return Err(Error::NotDir);
            }
            let (_, hit) = v.dir_find(&src, name)?;
            let (index, generation, typ) = hit.ok_or(Error::NoEnt)?;
            let child_id = oid(index, generation);
            if src_dir == dst_dir && name == new_name {
                return Ok(child_id);
            }
            if v.dir_find(&dst, new_name)?.1.is_some() {
                return Err(Error::Exist);
            }
            if typ == DIR {
                let root = oid(0, v.get_index(0)?.generation);
                let mut cur = dst_dir;
                loop {
                    if cur == child_id {
                        return Err(Error::Loop);
                    }
                    if cur == root {
                        break;
                    }
                    cur = v.get(cur)?.parent;
                }
            }
            v.preflight(8)?;
            v.dir_remove(&mut src, name)?;
            src.mtime = wall_us;
            src.version += 1;
            v.put(src_dir, &src)?;
            let mut dst = v.get(dst_dir)?;
            v.dir_insert(&mut dst, new_name, index, generation, typ)?;
            dst.mtime = wall_us;
            dst.version += 1;
            v.put(dst_dir, &dst)?;
            let mut child = v.get(child_id)?;
            child.parent = dst_dir;
            child.version += 1;
            v.put(child_id, &child)?;
            Ok(child_id)
        })
    }

    pub fn set_times(&mut self, object: u64, ctime: u64, mtime: u64, wall_us: u64) -> Result<()> {
        self.run(wall_us, |v| {
            let mut rec = v.get(object)?;
            v.preflight(2)?;
            rec.ctime = ctime;
            rec.mtime = mtime;
            rec.version += 1;
            v.put(object, &rec)
        })
    }

    // ---- commit ---------------------------------------------------------------------------------

    fn commit(&mut self, wall_us: u64) -> Result<()> {
        // A fresh object root; the old root and bitmap blocks are freed.
        let root = self.alloc()?;
        if self.root != 0 {
            self.free(self.root);
        }
        let mut new_bitmaps = [0u64; MAX_BITMAPS];
        for nb in new_bitmaps.iter_mut().take(self.nbitmaps) {
            *nb = self.alloc()?;
        }
        for i in 0..self.nbitmaps {
            let b = self.bitmap_blocks[i];
            if b != 0 {
                self.free(b);
            }
        }
        // Final bitmap: this transaction's allocations minus blocks the new
        // generation no longer references.
        let mut fin = self.work;
        for (f, p) in fin.iter_mut().zip(self.pending.iter()) {
            *f &= !p;
        }
        let bytes = self.total.div_ceil(8) as usize;
        let used: u64 = fin[..bytes].iter().map(|b| u64::from(b.count_ones())).sum();
        let free_blocks = self.total - used;
        // The commit record publishing all of it (written last, below).
        let seq = self.seq + 1;
        let mut rec = [0u8; SECTOR];
        rec[..8].copy_from_slice(COMMIT_MAGIC);
        put32(&mut rec, 8, VERSION);
        put64(&mut rec, 16, seq);
        put64(&mut rec, 24, root);
        put64(&mut rec, 32, free_blocks);
        put32(&mut rec, 40, self.tx_used);
        put64(&mut rec, 48, wall_us);
        put32(&mut rec, 56, self.nbitmaps as u32);
        for (i, nb) in new_bitmaps.iter().take(self.nbitmaps).enumerate() {
            put64(&mut rec, 64 + i * 8, *nb);
        }
        let c = fnv(&rec[..504]);
        put64(&mut rec, 504, c);
        let slot = 1 + seq % 2;
        let mut out = [0u8; BLOCK];
        // Dirty metadata.
        for i in 0..DIRTY {
            if self.slots[i].block == 0 {
                continue;
            }
            let (block, kind) = (self.slots[i].block, self.slots[i].kind);
            let data = self.slots[i].data;
            seal(kind, block, &data, &mut out);
            self.raw_write(block, &out)?;
        }
        let mut p = [0u8; PAYLOAD];
        for (i, r) in self.tx_root.iter().enumerate() {
            put64(&mut p, i * 8, *r);
        }
        seal(KIND_OBJROOT, root, &p, &mut out);
        self.raw_write(root, &out)?;
        for (i, nb) in new_bitmaps.iter().take(self.nbitmaps).enumerate() {
            p.copy_from_slice(&fin[i * PAYLOAD..(i + 1) * PAYLOAD]);
            seal(KIND_BITMAP, *nb, &p, &mut out);
            self.raw_write(*nb, &out)?;
        }
        // Last: the one-sector commit record.
        self.write_commit(slot, &rec)?;
        // The new generation is durable: adopt it.
        self.seq = seq;
        self.root = root;
        self.free_blocks = free_blocks;
        self.used_objects = self.tx_used;
        self.wall_us = wall_us;
        self.bitmap_blocks = new_bitmaps;
        self.bitmap = fin;
        self.hint = self.tx_hint;
        let writes = self.writes;
        self.begin();
        self.writes = writes;
        Ok(())
    }

    fn write_commit(&mut self, slot: u64, rec: &[u8; SECTOR]) -> Result<()> {
        for c in self.cache.iter_mut().filter(|c| c.block == slot) {
            c.block = 0;
        }
        self.writes += 1;
        self.dev()?.write_sector(slot, rec)
    }
}

// ---- directory block helpers (payload = u16 count, u16 used, entries) ----

/// A directory block holding exactly one entry.
fn single_entry(p: &mut [u8; PAYLOAD], name: &[u8], index: u32, generation: u32, typ: u8) {
    p.fill(0);
    put16(p, 2, 4);
    insert_entry(p, name, index, generation, typ);
}

/// Offsets of the entries of a block, in order.
fn offsets(p: &[u8; PAYLOAD], out: &mut [usize; DIR_ROOM / ENTRY_HEAD + 1]) -> usize {
    let count = le16(p, 0) as usize;
    let mut at = 4;
    for o in out.iter_mut().take(count) {
        *o = at;
        at += ENTRY_HEAD + p[at + 9] as usize;
    }
    count
}

fn entry_name(p: &[u8], at: usize) -> &[u8] {
    &p[at + ENTRY_HEAD..at + ENTRY_HEAD + p[at + 9] as usize]
}

/// Insert keeping name order (the caller checked it fits).
fn insert_entry(p: &mut [u8; PAYLOAD], name: &[u8], index: u32, generation: u32, typ: u8) {
    let count = le16(p, 0) as usize;
    let used = le16(p, 2) as usize;
    let mut at = 4;
    for _ in 0..count {
        if entry_name(p, at) > name {
            break;
        }
        at += ENTRY_HEAD + p[at + 9] as usize;
    }
    let size = ENTRY_HEAD + name.len();
    p.copy_within(at..used, at + size);
    put32(p, at, index);
    put32(p, at + 4, generation);
    p[at + 8] = typ;
    p[at + 9] = name.len() as u8;
    p[at + ENTRY_HEAD..at + size].copy_from_slice(name);
    put16(p, 0, (count + 1) as u16);
    put16(p, 2, (used + size) as u16);
}

/// Split a full block plus one new entry into two blocks in name order:
/// the lower half (by count) and the upper half.
fn split_block(
    p: &[u8; PAYLOAD],
    name: &[u8],
    index: u32,
    generation: u32,
    typ: u8,
) -> Result<([u8; PAYLOAD], [u8; PAYLOAD])> {
    let mut offs = [0usize; DIR_ROOM / ENTRY_HEAD + 1];
    let count = offsets(p, &mut offs);
    let total = count + 1;
    let half = total / 2;
    let mut low = [0u8; PAYLOAD];
    let mut high = [0u8; PAYLOAD];
    put16(&mut low, 2, 4);
    put16(&mut high, 2, 4);
    let mut placed = false;
    let mut k = 0usize;
    let mut i = 0usize;
    while k < total {
        let (ename, eindex, egen, etyp): (&[u8], u32, u32, u8) =
            if !placed && (i == count || entry_name(p, offs[i]) > name) {
                placed = true;
                (name, index, generation, typ)
            } else {
                let at = offs[i];
                i += 1;
                (entry_name(p, at), le32(p, at), le32(p, at + 4), p[at + 8])
            };
        let target = if k < half { &mut low } else { &mut high };
        let used = le16(target, 2) as usize;
        if used + ENTRY_HEAD + ename.len() > PAYLOAD {
            return Err(Error::Corrupt);
        }
        insert_entry(target, ename, eindex, egen, etyp);
        k += 1;
    }
    Ok((low, high))
}

fn remove_entry(p: &mut [u8; PAYLOAD], name: &[u8]) -> Result<()> {
    let count = le16(p, 0) as usize;
    let used = le16(p, 2) as usize;
    let mut at = 4;
    for _ in 0..count {
        let size = ENTRY_HEAD + p[at + 9] as usize;
        if entry_name(p, at) == name {
            p.copy_within(at + size..used, at);
            p[used - size..used].fill(0);
            put16(p, 0, (count - 1) as u16);
            put16(p, 2, (used - size) as u16);
            return Ok(());
        }
        at += size;
    }
    Err(Error::NoEnt)
}

#[cfg(test)]
mod tests;
