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
/// S_OK when the lent record is in the same lineage as the called one
/// (the broker attributing an offered file capability to a session).
pub const OP_SAME_LINEAGE: u8 = 16;
/// Through the root record only: a new lineage head naming the root
/// object with no rights (the broker's per-application anchor).
pub const OP_NEW_LINEAGE: u8 = 17;
/// Watch the called directory record (ADR-0079): the landed notification
/// (WRITE) is signalled with bit `offset` (0..=63) when the directory
/// changes. One watch per record; it ends with the record.
pub const OP_WATCH: u8 = 18;
pub const OP_UNWATCH: u8 = 19;
/// Changes since last asked (value) and whether the directory is gone
/// (reply byte 0); clears the count. The badge is a hint, this is the fact.
pub const OP_WATCHED: u8 = 20;
/// Phase 12's restricted APB1 subprotocol uses IPC w0/w1 so its complete
/// 64-byte key/digest records fit without changing the legacy 64-byte request
/// layout or reinterpreting any existing filesd operation.
pub const CALL_APB1_PROBE: u64 = 0x4150_4231_0000_0000;
pub const CALL_APB1_INSPECT: u64 = 0x4150_4231_0000_0001;
pub const CALL_APB1_VERIFY: u64 = 0x4150_4231_0000_0002;
pub const CALL_APB1_INSTALL: u64 = 0x4150_4231_0000_0003;
/// Ordered descriptive scan of protected installed APB1 version directories.
pub const CALL_APB1_NEXT_INSTALLED: u64 = 0x4150_4231_0000_0004;
/// Read an unauthenticated installed APB1 claim for receiver policy lookup.
pub const CALL_APB1_INSPECT_INSTALLED: u64 = 0x4150_4231_0000_0005;
/// Verify the exact installed tree with the selected receiver key.
pub const CALL_APB1_VERIFY_INSTALLED: u64 = 0x4150_4231_0000_0006;
/// Read a chunk from metadata bound to the preceding verification token.
pub const CALL_APB1_READ_VERIFIED_METADATA: u64 = 0x4150_4231_0000_0007;
/// Copy one executable chunk from the verified installed entry into a lent
/// SharedRegion page. The cap remains temporary and is never inherited.
pub const CALL_APB1_READ_VERIFIED_EXECUTABLE: u64 = 0x4150_4231_0000_0008;
/// Copy a chunk from one exact path in the current verified signed payload
/// catalog into a lent SharedRegion. Message bytes carry token, offset, and a
/// canonical relative path of at most 47 bytes.
pub const CALL_APB1_READ_VERIFIED_FILE: u64 = 0x4150_4231_0000_0009;
pub const CALL_APB1_ABI_V1: u64 = 1;
/// filesd's internal APB1 capability record (ADR-0091), index 2/generation 1.
pub const APB1_INSTALL_BADGE: u32 = 2 | (1 << 16);
/// Rights in a key-only verification request and a key+digest install request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Apb1InstallRequest {
    pub trusted_key: [u8; 32],
    pub bundle_digest: [u8; 32],
}
impl Apb1InstallRequest {
    pub fn encode(&self) -> [u8; BYTES] {
        let mut bytes = [0; BYTES];
        bytes[..32].copy_from_slice(&self.trusted_key);
        bytes[32..].copy_from_slice(&self.bundle_digest);
        bytes
    }
    pub fn decode(bytes: &[u8; BYTES]) -> Self {
        let mut trusted_key = [0; 32];
        trusted_key.copy_from_slice(&bytes[..32]);
        let mut bundle_digest = [0; 32];
        bundle_digest.copy_from_slice(&bytes[32..]);
        Self {
            trusted_key,
            bundle_digest,
        }
    }
}

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
        (r.op >= OP_SESSION && r.op <= OP_WATCHED && r.rights & !R_ALL == 0).then_some(r)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apb1_key_and_digest_share_exactly_one_raw_64_byte_record() {
        let request = Apb1InstallRequest {
            trusted_key: core::array::from_fn(|index| index as u8),
            bundle_digest: core::array::from_fn(|index| 255 - index as u8),
        };
        let bytes = request.encode();
        assert_eq!(&bytes[..32], &request.trusted_key);
        assert_eq!(&bytes[32..], &request.bundle_digest);
        assert_eq!(Apb1InstallRequest::decode(&bytes), request);

        // The dedicated w0 operation is not a reinterpretation of legacy
        // AF2Q request bytes or an extension of the generic R_ALL mask.
        assert_eq!(Request::decode(&bytes), None);
        assert_eq!(R_ALL & (1 << 6), 0);
        assert_eq!(APB1_INSTALL_BADGE, 0x0001_0002);
    }
}
