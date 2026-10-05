//! filesd request wire (Phase 11.5/11.6, ADR-0077).
//!
//! Authority is the badged endpoint capability the call arrives through
//! (ADR-0074): filesd maps each badge to one capability record naming an
//! AFS2 object, a generation and rights. Requests carry only names
//! relative to that object (never paths); names and data travel in the
//! caller's registered I/O page. Nothing in a request grants anything.
#![allow(dead_code)]

pub const BYTES: usize = 64;
const MAGIC: &[u8; 4] = b"AF2Q";

/// Operations. The called capability is "self".
pub const OP_SESSION: u8 = 1; // landed SharedRegion: page `offset` becomes the lineage's I/O page
pub const OP_STAT: u8 = 2; // self, or the child named in the I/O page
pub const OP_LIST: u8 = 3; // entries after the cursor name; packed into the I/O page
pub const OP_OPEN: u8 = 4; // child (or self when no name) with requested rights; reply carries a new badged cap
pub const OP_CREATE: u8 = 5;
pub const OP_MKDIR: u8 = 6;
pub const OP_READ: u8 = 7; // offset, len <= PAGE: data into the I/O page
pub const OP_WRITE: u8 = 8; // offset, len <= PAGE: data from the I/O page
pub const OP_TRUNCATE: u8 = 9; // offset = new size
pub const OP_UNLINK: u8 = 10;
pub const OP_RMDIR: u8 = 11;
/// Name (name_len) then new name (len) in the I/O page; destination
/// directory = the landed badged cap of this same service, or self.
pub const OP_RENAME: u8 = 12;
pub const OP_RELEASE: u8 = 13; // retire the called record (stale from now on)
pub const OP_STATFS: u8 = 14;
/// Release another record of this service that the caller lends (the
/// broker retiring a dead application's grants).
pub const OP_REVOKE: u8 = 15;

/// Rights carried by a capability record.
pub const R_READ: u8 = 1;
pub const R_WRITE: u8 = 2;
pub const R_LIST: u8 = 4;
pub const R_CREATE: u8 = 8;
pub const R_DELETE: u8 = 16;
pub const R_RENAME: u8 = 32;
pub const R_ALL: u8 = 63;

/// Status words (reply word 0).
pub const S_OK: u64 = 0;
pub const S_NOENT: u64 = 1;
pub const S_EXIST: u64 = 2;
pub const S_NOTDIR: u64 = 3;
pub const S_ISDIR: u64 = 4;
pub const S_NOTEMPTY: u64 = 5;
pub const S_INVAL: u64 = 6;
pub const S_NOSPC: u64 = 7;
pub const S_FBIG: u64 = 8;
pub const S_LOOP: u64 = 9;
pub const S_CORRUPT: u64 = 10;
pub const S_IO: u64 = 11;
/// The badge names no live record, the record lacks the right, or the
/// object it named is gone (stale after delete).
pub const S_DENIED: u64 = 12;
pub const S_NO_SESSION: u64 = 13;
pub const S_FULL: u64 = 14;
pub const S_OFFLINE: u64 = 15;

/// The I/O page size (names, listings, data).
pub const PAGE: usize = 4096;

/// Kernel-minted badge of the desktop broker's grant: `/Users/user`, all
/// rights. Record 1, record generation 1 (`badge = index | gen << 16`).
pub const USER_ROOT_BADGE: u32 = 1 | 1 << 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Request {
    pub op: u8,
    /// Requested rights (OPEN).
    pub rights: u8,
    pub offset: u64,
    pub len: u32,
    pub name_len: u16,
}

impl Request {
    pub const fn new(op: u8) -> Self {
        Request {
            op,
            rights: 0,
            offset: 0,
            len: 0,
            name_len: 0,
        }
    }
    pub fn encode(&self) -> [u8; BYTES] {
        let mut b = [0u8; BYTES];
        b[..4].copy_from_slice(MAGIC);
        b[4] = self.op;
        b[5] = self.rights;
        b[8..16].copy_from_slice(&self.offset.to_le_bytes());
        b[16..20].copy_from_slice(&self.len.to_le_bytes());
        b[20..22].copy_from_slice(&self.name_len.to_le_bytes());
        b
    }
    /// Canonical decode: every reserved byte must be zero.
    pub fn decode(b: &[u8; BYTES]) -> Option<Self> {
        if &b[..4] != MAGIC || b[6..8] != [0, 0] || b[22..].iter().any(|x| *x != 0) {
            return None;
        }
        let r = Request {
            op: b[4],
            rights: b[5],
            offset: u64::from_le_bytes(b[8..16].try_into().ok()?),
            len: u32::from_le_bytes(b[16..20].try_into().ok()?),
            name_len: u16::from_le_bytes(b[20..22].try_into().ok()?),
        };
        (r.op >= OP_SESSION && r.op <= OP_REVOKE && r.rights & !R_ALL == 0).then_some(r)
    }
}

/// STAT reply bytes: type, size, mtime (wall µs, 0 = unknown), ctime,
/// entries, version.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct StatReply {
    pub typ: u8,
    pub size: u64,
    pub mtime: u64,
    pub ctime: u64,
    pub entries: u32,
    pub version: u64,
}
impl StatReply {
    pub fn encode(&self) -> [u8; BYTES] {
        let mut b = [0u8; BYTES];
        b[0] = self.typ;
        b[8..16].copy_from_slice(&self.size.to_le_bytes());
        b[16..24].copy_from_slice(&self.mtime.to_le_bytes());
        b[24..32].copy_from_slice(&self.ctime.to_le_bytes());
        b[32..36].copy_from_slice(&self.entries.to_le_bytes());
        b[40..48].copy_from_slice(&self.version.to_le_bytes());
        b
    }
    pub fn decode(b: &[u8; BYTES]) -> Self {
        let q = |a: usize| u64::from_le_bytes(b[a..a + 8].try_into().unwrap_or([0; 8]));
        StatReply {
            typ: b[0],
            size: q(8),
            mtime: q(16),
            ctime: q(24),
            entries: u32::from_le_bytes(b[32..36].try_into().unwrap_or([0; 4])),
            version: q(40),
        }
    }
}

/// LIST packs entries into the I/O page: `typ u8, name_len u8, size u64,
/// mtime u64, name`. The reply value is the count; reply byte 0 = 1 when
/// more entries follow the last one.
pub const LIST_HEAD: usize = 18;
