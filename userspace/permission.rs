//! ADR-0048: no_std immutable permission-decision record and namespace recovery core.
//!
//! No filesystem calls or cap checks live here. The permission broker
//! enumerates EVERY fsd directory entry, reads exactly 512 bytes for each
//! nonempty reserved file, then feeds it to Scan. An I/O or short-read
//! error must *not* become an empty generation. The caller also supplies
//! the received capability to its service-boundary authorization check.
//! Only fsd-visible records under the accepted AFS1 crash model are in
//! scope; an invisible newer AFS1 commit is not detectable here.

pub const SECTOR_BYTES: usize = 512;
pub const PAYLOAD_MAX: usize = 32;
pub const MAX_GENERATIONS: u8 = 8;
const MAGIC: &[u8; 8] = b"ARPRM8V1";
const PREFIX: &[u8; 6] = b"perm8-";
const CHECKSUM_OFF: usize = 504;
const FNV_BASIS: u64 = 0xCBF2_9CE4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01B3;

/// A malformed visible record/namespace must never cause an old-value
/// fallback. Exhaustion and invalid caller input are distinct from it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    InvalidInput,
    Corrupt,
}

/// Fixed built-in app request, separate from both durable approval and
/// issued bearer. Bytes contain NO cap slot/object, pid, token or policy
/// decision. Possession of the mediator endpoint remains the acquisition
/// authority; this descriptor only limits what it asks the receiver to do.
pub const REQUEST_BYTES: usize = 8;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AppRequest {
    pub version: u8,
    pub scope: u8,      // 1 = arena.txt
    pub operation: u8,  // 1 = READ
    pub rights: u8,     // 1 = READ, never WRITE
}
pub const ARENA_READ: AppRequest = AppRequest {
    version: 1, scope: 1, operation: 1, rights: 1,
};
// byte 2 is the bounded operation count; bytes 5..8 are reserved.
// A duplicate second operation or a serialized decision cannot fit v1.
pub const READ_REQUEST: [u8; REQUEST_BYTES] = [1, 1, 1, 1, 1, 0, 0, 0];
pub fn encode_request(req: AppRequest) -> Result<[u8; REQUEST_BYTES], Error> {
    if req != ARENA_READ { return Err(Error::InvalidInput); }
    Ok(READ_REQUEST)
}
pub fn decode_request(raw: &[u8]) -> Result<AppRequest, Error> {
    if raw != READ_REQUEST { return Err(Error::InvalidInput); }
    Ok(ARENA_READ)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Record {
    sequence: u8,
    length: u8,
    payload: [u8; PAYLOAD_MAX],
}

impl Record {
    pub fn sequence(&self) -> u8 {
        self.sequence
    }

    pub fn bytes(&self) -> &[u8] {
        &self.payload[..usize::from(self.length)]
    }
}

pub fn name(seq: u8) -> Result<[u8; 8], Error> {
    if !(1..=MAX_GENERATIONS).contains(&seq) {
        return Err(Error::InvalidInput);
    }
    Ok([b'p', b'e', b'r', b'm', b'8', b'-', b'0', b'0' + seq])
}

fn hash(data: &[u8]) -> u64 {
    data.iter().fold(FNV_BASIS, |h, b| {
        (h ^ u64::from(*b)).wrapping_mul(FNV_PRIME)
    })
}

/// Only one fixed scope (arena.txt READ) and DENY=0/ALLOW=1. No
/// cap, bearer, image-id or source-slot number can be serialized.
pub fn valid_decision(bytes: &[u8]) -> bool {
    bytes.len() == 4 && bytes[0] == 1 && bytes[1] == 1
        && bytes[2] <= 1 && bytes[3] == 0
}
pub fn decision(allow: bool) -> [u8; 4] { [1, 1, u8::from(allow), 0] }

/// Build the exact fixed-size record in a caller-owned 512-byte buffer.
/// `fill(0)` also canonicalizes every unused field before checksumming.
pub fn encode(seq: u8, payload: &[u8], out: &mut [u8; SECTOR_BYTES]) -> Result<(), Error> {
    name(seq)?;
    if !valid_decision(payload) {
        return Err(Error::InvalidInput);
    }
    out.fill(0);
    out[..8].copy_from_slice(MAGIC);
    out[8..12].copy_from_slice(&1u32.to_le_bytes());
    out[16..24].copy_from_slice(&u64::from(seq).to_le_bytes());
    out[24..26].copy_from_slice(&(payload.len() as u16).to_le_bytes());
    out[32..32 + payload.len()].copy_from_slice(payload);
    let checksum = hash(&out[..CHECKSUM_OFF]);
    out[CHECKSUM_OFF..].copy_from_slice(&checksum.to_le_bytes());
    Ok(())
}

/// The expected generation comes from the *canonical filename*, never
/// from untrusted bytes inside the record. Verify every reserved bit.
pub fn decode(seq: u8, raw: &[u8; SECTOR_BYTES]) -> Result<Record, Error> {
    name(seq).map_err(|_| Error::Corrupt)?;
    if &raw[..8] != MAGIC
        || raw[8..12] != 1u32.to_le_bytes()
        || raw[12..16].iter().any(|&b| b != 0)
        || raw[16..24] != u64::from(seq).to_le_bytes()
        || raw[26..32].iter().any(|&b| b != 0)
    {
        return Err(Error::Corrupt);
    }
    let length = u16::from_le_bytes([raw[24], raw[25]]) as usize;
    if length != 4 || !valid_decision(&raw[32..36])
        || raw[32 + length..CHECKSUM_OFF].iter().any(|&b| b != 0) {
        return Err(Error::Corrupt);
    }
    let mut checksum = [0u8; 8];
    checksum.copy_from_slice(&raw[CHECKSUM_OFF..]);
    if hash(&raw[..CHECKSUM_OFF]) != u64::from_le_bytes(checksum) {
        return Err(Error::Corrupt);
    }
    let mut payload = [0u8; PAYLOAD_MAX];
    payload[..length].copy_from_slice(&raw[32..32 + length]);
    Ok(Record {
        sequence: seq,
        length: length as u8,
        payload,
    })
}

/// A *complete* scan of fsd's namespace (not just the highest file).
/// No allocator, fixed 8-bit seen set, order-independent LS ingestion.
pub struct Scan {
    seen: u8,
    max: u8,
    pending: Option<u8>,
    current: Option<Record>,
    poisoned: bool, // even a caller that ignores ingest's Err cannot return an old value
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Recovery {
    pub current: Option<Record>,
    pub pending: Option<u8>,
}

impl Recovery {
    /// None means the eight immutable slots are exhausted. If the
    /// last slot is empty after CREATE, it can still be filled once.
    pub fn next(&self) -> Option<u8> {
        self.pending.or_else(|| {
            let next = self.current.map_or(1, |r| r.sequence + 1);
            (next <= MAX_GENERATIONS).then_some(next)
        })
    }
}

impl Scan {
    pub const fn new() -> Self {
        Self {
            seen: 0,
            max: 0,
            pending: None,
            current: None,
            poisoned: false,
        }
    }

    fn corrupt(&mut self) -> Result<bool, Error> {
        self.poisoned = true;
        Err(Error::Corrupt)
    }

    /// `contents=None` ONLY for an actually empty file. Caller must
    /// check both FS status and returned read length before passing a
    /// nonempty file; a short/failed read is corruption, not UNSET.
    /// Returns true when `name` was in the reserved namespace.
    pub fn ingest(
        &mut self,
        filename: &[u8],
        size: u64,
        contents: Option<&[u8; SECTOR_BYTES]>,
    ) -> Result<bool, Error> {
        if !filename.starts_with(PREFIX) {
            return Ok(false);
        }
        if filename.len() != 8 || filename[6] != b'0' || !(b'1'..=b'8').contains(&filename[7]) {
            return self.corrupt();
        }
        let seq = filename[7] - b'0';
        let bit = 1u8 << (seq - 1);
        if self.seen & bit != 0 {
            return self.corrupt();
        }
        self.seen |= bit;
        self.max = self.max.max(seq);
        match (size, contents) {
            (0, None) => {
                if self.pending.replace(seq).is_some() {
                    return self.corrupt();
                }
            }
            (512, Some(raw)) => {
                let record = match decode(seq, raw) {
                    Ok(value) => value,
                    Err(_) => return self.corrupt(),
                };
                if self.current.is_none_or(|r| seq > r.sequence) {
                    self.current = Some(record);
                }
            }
            _ => return self.corrupt(),
        }
        Ok(true)
    }

    pub fn finish(self) -> Result<Recovery, Error> {
        // Seen bits must be contiguous 1..max. A pending empty file is
        // permitted ONLY at max; it may be generation 1 with no value.
        if self.poisoned
            || self.seen != ((1u16 << self.max) - 1) as u8
            || self.pending.is_some_and(|p| p != self.max)
            || (self.pending.is_some() && self.current.is_some_and(|r| r.sequence + 1 != self.max))
        {
            return Err(Error::Corrupt);
        }
        Ok(Recovery {
            current: self.current,
            pending: self.pending,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn sector(seq: u8, allow: bool) -> [u8; 512] {
        let mut buf = [0xa5; 512];
        encode(seq, &decision(allow), &mut buf).unwrap();
        buf
    }
    fn reseal(raw: &mut [u8; 512]) {
        let sum = hash(&raw[..504]);
        raw[504..].copy_from_slice(&sum.to_le_bytes());
    }
    #[test]
    fn request_is_bounded_and_cannot_encode_authority_or_approval() {
        assert_eq!(encode_request(ARENA_READ), Ok(READ_REQUEST));
        assert_eq!(decode_request(&READ_REQUEST), Ok(ARENA_READ));
        assert_eq!(decode_request(&READ_REQUEST[..7]), Err(Error::InvalidInput));
        let mut extra = READ_REQUEST.to_vec();
        extra.push(0);
        assert_eq!(decode_request(&extra), Err(Error::InvalidInput));
        for (index, bad) in [(0, 2), (1, 2), (2, 0), (2, 2),
                             (3, 3), (4, 2), (5, 1), (6, 1), (7, 1)] {
            let mut bytes = READ_REQUEST;
            bytes[index] = bad;
            assert_eq!(decode_request(&bytes), Err(Error::InvalidInput),
                       "bad request field {index}");
        }
        for malformed in [AppRequest { version: 2, ..ARENA_READ },
                          AppRequest { scope: 2, ..ARENA_READ },
                          AppRequest { operation: 2, ..ARENA_READ },
                          AppRequest { rights: 3, ..ARENA_READ }] {
            assert_eq!(encode_request(malformed), Err(Error::InvalidInput));
        }
        let mut duplicate = READ_REQUEST;
        duplicate[2] = 2;
        duplicate[5] = 1;
        assert_eq!(decode_request(&duplicate), Err(Error::InvalidInput));
    }
    #[test]
    fn fixed_scope_and_no_authority_in_record() {
        let x = sector(1, true);
        assert_eq!(decode(1, &x).unwrap().bytes(), decision(true));
        // Independently packed Python reference (tools/permission_record.py).
        assert_eq!(u64::from_le_bytes(x[504..].try_into().unwrap()), 0x0bbf_8fb9_1fb5_b0cf);
        assert_eq!(u64::from_le_bytes(sector(2, false)[504..].try_into().unwrap()), 0xc49a_8488_15f4_b4ad);
        assert!(x[36..504].iter().all(|b| *b == 0));
        assert_eq!(name(1).unwrap(), *b"perm8-01");
        for payload in [&[1, 2, 1, 0][..], &[2, 1, 1, 0], &[1, 1, 2, 0],
                        &[1, 1, 1, 1], &[1, 1, 1], &[1, 1, 1, 0, 0]] {
            assert_eq!(encode(1, payload, &mut [0; 512]), Err(Error::InvalidInput));
        }
        for offset in [0, 8, 12, 16, 24, 26, 32, 33, 34, 35, 36, 503] {
            let mut bad = x;
            bad[offset] ^= 0x80;
            reseal(&mut bad);
            assert_eq!(decode(1, &bad), Err(Error::Corrupt), "offset {offset}");
        }
        assert_eq!(decode(2, &x), Err(Error::Corrupt));
    }
    #[test]
    fn poisoned_scan_never_falls_back_to_allow() {
        let old = sector(1, true);
        let deny = sector(2, false);
        for (filename, raw) in [(&b"perm8-03"[..], &deny),
                                (&b"perm8-02"[..], &old)] {
            let mut s = Scan::new();
            s.ingest(b"perm8-01", 512, Some(&old)).unwrap();
            assert!(s.ingest(filename, 512, Some(raw)).is_err());
            assert_eq!(s.finish(), Err(Error::Corrupt));
        }
        let mut s = Scan::new();
        s.ingest(b"perm8-01", 512, Some(&old)).unwrap();
        s.ingest(b"perm8-02", 0, None).unwrap();
        let recovered = s.finish().unwrap();
        assert_eq!(recovered.current.unwrap().bytes(), decision(true));
        assert_eq!(recovered.next(), Some(2));
        for n in 1..=8 {
            let mut s = Scan::new();
            for i in 1..=n { s.ingest(&name(i).unwrap(), 512, Some(&sector(i, i % 2 == 0))).unwrap(); }
            let r = s.finish().unwrap();
            assert_eq!(r.next(), (n < 8).then_some(n + 1));
        }
    }
}
