//! ADR-0046: no_std on-disk config record and namespace recovery core.
//!
//! No filesystem calls or cap checks live here. The eventual configd
//! enumerates EVERY fsd directory entry, reads exactly 512 bytes for each
//! nonempty reserved file, then feeds it to Scan. An I/O or short-read
//! error must *not* become an empty generation. The caller also supplies
//! the received capability to its service-boundary authorization check.
//! Only fsd-visible records under the accepted AFS1 crash model are in
//! scope; an invisible newer AFS1 commit is not detectable here.

pub const SECTOR_BYTES: usize = 512;
pub const PAYLOAD_MAX: usize = 32;
pub const MAX_GENERATIONS: u8 = 8;
const MAGIC: &[u8; 8] = b"ARCFG8V1";
const PREFIX: &[u8; 5] = b"cfg8-";
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

pub fn name(seq: u8) -> Result<[u8; 7], Error> {
    if !(1..=MAX_GENERATIONS).contains(&seq) {
        return Err(Error::InvalidInput);
    }
    Ok([b'c', b'f', b'g', b'8', b'-', b'0', b'0' + seq])
}

fn hash(data: &[u8]) -> u64 {
    data.iter().fold(FNV_BASIS, |h, b| {
        (h ^ u64::from(*b)).wrapping_mul(FNV_PRIME)
    })
}

/// Build the exact fixed-size record in a caller-owned 512-byte buffer.
/// `fill(0)` also canonicalizes every unused field before checksumming.
pub fn encode(seq: u8, payload: &[u8], out: &mut [u8; SECTOR_BYTES]) -> Result<(), Error> {
    name(seq)?;
    if payload.len() > PAYLOAD_MAX {
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
    if length > PAYLOAD_MAX || raw[32 + length..CHECKSUM_OFF].iter().any(|&b| b != 0) {
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
        if filename.len() != 7 || filename[5] != b'0' || !(b'1'..=b'8').contains(&filename[6]) {
            return self.corrupt();
        }
        let seq = filename[6] - b'0';
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

    fn record(seq: u8, payload: &[u8]) -> [u8; 512] {
        let mut out = [0xE5; 512]; // encode must clear stale bytes
        encode(seq, payload, &mut out).unwrap();
        out
    }

    fn reseal(out: &mut [u8; 512]) {
        let checksum = hash(&out[..504]);
        out[504..].copy_from_slice(&checksum.to_le_bytes());
    }

    #[test]
    fn python_reference_vectors_and_roundtrip() {
        let empty = record(1, b"");
        assert_eq!(
            u64::from_le_bytes(empty[504..].try_into().unwrap()),
            0x9036_0ed9_a523_9411
        );
        assert_eq!(decode(1, &empty).unwrap().bytes(), b"");
        let hello = record(2, b"hello");
        assert_eq!(
            u64::from_le_bytes(hello[504..].try_into().unwrap()),
            0x1b2c_356e_aa80_2b8f
        );
        assert_eq!(decode(2, &hello).unwrap().bytes(), b"hello");
        let all: [u8; 32] = core::array::from_fn(|i| i as u8);
        let max = record(8, &all);
        assert_eq!(
            u64::from_le_bytes(max[504..].try_into().unwrap()),
            0xe839_b023_f540_c718
        );
        assert_eq!(decode(8, &max).unwrap().bytes(), &all);
        assert_eq!(encode(0, b"", &mut [0; 512]), Err(Error::InvalidInput));
        assert_eq!(encode(9, b"", &mut [0; 512]), Err(Error::InvalidInput));
        assert_eq!(encode(1, &[1; 33], &mut [0; 512]), Err(Error::InvalidInput));
    }

    #[test]
    fn visible_corruption_never_falls_back_to_old() {
        let old = record(1, b"old");
        let good = record(2, b"new");
        for (offset, byte) in [
            (0, 0),
            (8, 2),
            (12, 1),
            (16, 3),
            (24, 33),
            (26, 1),
            (35, 1),
            (503, 1),
        ] {
            let mut bad = good;
            bad[offset] = byte;
            reseal(&mut bad); // even with a repaired checksum: reject format
            let mut scan = Scan::new();
            scan.ingest(b"cfg8-01", 512, Some(&old)).unwrap();
            assert_eq!(
                scan.ingest(b"cfg8-02", 512, Some(&bad)),
                Err(Error::Corrupt),
                "{offset}"
            );
        }
        let mut bad = good;
        bad[33] ^= 1;
        assert_eq!(decode(2, &bad), Err(Error::Corrupt));
        assert_eq!(decode(3, &good), Err(Error::Corrupt));
    }

    #[test]
    fn pending_contiguous_and_exhausted() {
        let s = Scan::new().finish().unwrap();
        assert_eq!(s.next(), Some(1));
        let mut s = Scan::new();
        assert!(!s.ingest(b"arena.txt", 2, None).unwrap());
        s.ingest(b"cfg8-01", 0, None).unwrap();
        assert_eq!(
            s.finish().unwrap(),
            Recovery {
                current: None,
                pending: Some(1)
            }
        );

        let mut s = Scan::new();
        s.ingest(b"cfg8-02", 512, Some(&record(2, b"new"))).unwrap();
        s.ingest(b"cfg8-03", 0, None).unwrap();
        s.ingest(b"cfg8-01", 512, Some(&record(1, b"old"))).unwrap();
        let found = s.finish().unwrap();
        assert_eq!(found.current.unwrap().bytes(), b"new");
        assert_eq!(found.next(), Some(3));
        let mut s = Scan::new();
        for seq in 1..=MAX_GENERATIONS {
            s.ingest(&name(seq).unwrap(), 512, Some(&record(seq, b"x")))
                .unwrap();
        }
        assert_eq!(s.finish().unwrap().next(), None);
    }

    #[test]
    fn malformed_namespace_and_read_failures() {
        for filename in [
            b"cfg8-00".as_slice(),
            b"cfg8-09",
            b"cfg8-1",
            b"cfg8-01.bak",
            b"cfg8-secret",
        ] {
            assert_eq!(Scan::new().ingest(filename, 0, None), Err(Error::Corrupt));
        }
        let mut s = Scan::new();
        s.ingest(b"cfg8-02", 512, Some(&record(2, b"new"))).unwrap();
        assert_eq!(s.finish(), Err(Error::Corrupt)); // gap
        let mut s = Scan::new();
        s.ingest(b"cfg8-01", 512, Some(&record(1, b"old"))).unwrap();
        assert_eq!(s.ingest(b"cfg8-01", 0, None), Err(Error::Corrupt));
        assert_eq!(s.finish(), Err(Error::Corrupt)); // cannot ignore an ingest error
        let mut s = Scan::new();
        s.ingest(b"cfg8-01", 0, None).unwrap();
        s.ingest(b"cfg8-02", 512, Some(&record(2, b"new"))).unwrap();
        assert_eq!(s.finish(), Err(Error::Corrupt)); // pending before newest
        let mut s = Scan::new();
        s.ingest(b"cfg8-01", 0, None).unwrap();
        assert_eq!(s.ingest(b"cfg8-02", 0, None), Err(Error::Corrupt));
        for (size, has_data) in [(512, false), (511, true), (513, true), (0, true)] {
            assert_eq!(
                Scan::new().ingest(b"cfg8-01", size, has_data.then_some(&record(1, b"x"))),
                Err(Error::Corrupt)
            );
        }
    }
}
