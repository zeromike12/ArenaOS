//! Bounded AHL1 helper declarations stored as an ordinary signed APB1 resource.
//!
//! IDs and relative executable paths only select entries from the current
//! receiver-verified payload catalog. They are never executable authority.

use crate::{manifest, package_policy};

pub const HEADER_BYTES: usize = 8;
pub const ENTRY_BYTES: usize = 88;
pub const MAX_HELPERS: usize = 4;
pub const ID_BYTES: usize = 32;
pub const PATH_BYTES: usize = 48;
pub const MAX_BYTES: usize = HEADER_BYTES + ENTRY_BYTES * MAX_HELPERS;
pub const RESOURCE_PATH: &[u8] = b"META-INF/arena.helpers";
pub const FLAG_TIMER: u32 = 1;
/// Give the child a separate WRITE-only cap on its AppInstance clock so it
/// can notify its owner without consuming or waiting on the owner's events.
pub const FLAG_OWNER_SIGNAL: u32 = 2;
const KNOWN_FLAGS: u32 = FLAG_TIMER | FLAG_OWNER_SIGNAL;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Header,
    Bounds,
    Identity,
    Path,
    Flags,
    Duplicate,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Descriptor {
    id: [u8; ID_BYTES],
    path: [u8; PATH_BYTES],
    flags: u32,
}

impl Descriptor {
    pub const fn empty() -> Self {
        Self {
            id: [0; ID_BYTES],
            path: [0; PATH_BYTES],
            flags: 0,
        }
    }

    pub const fn id(&self) -> &[u8; ID_BYTES] {
        &self.id
    }

    pub fn path(&self) -> &[u8] {
        fixed(&self.path).unwrap_or(&[])
    }

    pub const fn flags(&self) -> u32 {
        self.flags
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Allowlist {
    entries: [Descriptor; MAX_HELPERS],
    count: usize,
}

impl Allowlist {
    pub fn parse(bytes: &[u8]) -> Result<Self, Error> {
        if bytes.len() < HEADER_BYTES || &bytes[..4] != b"AHL1" || bytes[4] != 1 {
            return Err(Error::Header);
        }
        let count = bytes[5] as usize;
        if !(1..=MAX_HELPERS).contains(&count)
            || bytes[6..8] != [0, 0]
            || bytes.len() != HEADER_BYTES + count * ENTRY_BYTES
        {
            return Err(Error::Bounds);
        }

        let mut entries = [Descriptor::empty(); MAX_HELPERS];
        for (index, record) in bytes[HEADER_BYTES..].chunks_exact(ENTRY_BYTES).enumerate() {
            let mut id = [0u8; ID_BYTES];
            id.copy_from_slice(&record[..ID_BYTES]);
            if !package_policy::canonical_id(&id) {
                return Err(Error::Identity);
            }
            let mut path = [0u8; PATH_BYTES];
            path.copy_from_slice(&record[ID_BYTES..ID_BYTES + PATH_BYTES]);
            let path_bytes = fixed(&path).ok_or(Error::Path)?;
            if path_bytes.len() > PATH_BYTES - 1 || !manifest::valid_relative_path(path_bytes) {
                return Err(Error::Path);
            }
            let flags = u32::from_le_bytes(record[80..84].try_into().map_err(|_| Error::Bounds)?);
            if flags & !KNOWN_FLAGS != 0
                || (flags & FLAG_OWNER_SIGNAL != 0 && flags & FLAG_TIMER == 0)
                || record[84..88] != [0; 4]
            {
                return Err(Error::Flags);
            }
            for prior in &entries[..index] {
                if prior.id == id || prior.path() == path_bytes {
                    return Err(Error::Duplicate);
                }
            }
            entries[index] = Descriptor { id, path, flags };
        }
        Ok(Self { entries, count })
    }

    pub const fn len(&self) -> usize {
        self.count
    }

    pub const fn is_empty(&self) -> bool {
        self.count == 0
    }

    pub fn get(&self, id: &[u8; ID_BYTES]) -> Option<Descriptor> {
        self.entries[..self.count]
            .iter()
            .find(|entry| entry.id == *id)
            .copied()
    }
}

fn fixed(bytes: &[u8; PATH_BYTES]) -> Option<&[u8]> {
    let end = bytes.iter().position(|byte| *byte == 0)?;
    (end != 0 && bytes[end..].iter().all(|byte| *byte == 0)).then_some(&bytes[..end])
}

#[cfg(test)]
mod tests {
    use super::*;
    extern crate std;

    fn fixture() -> std::vec::Vec<u8> {
        let mut bytes = std::vec![0; HEADER_BYTES + ENTRY_BYTES];
        bytes[..4].copy_from_slice(b"AHL1");
        bytes[4] = 1;
        bytes[5] = 1;
        bytes[8..19].copy_from_slice(b"com.tool.gc");
        bytes[40..50].copy_from_slice(b"bin/worker");
        bytes[88..92].copy_from_slice(&FLAG_TIMER.to_le_bytes());
        bytes
    }

    #[test]
    fn resolves_only_a_canonical_signed_descriptor() {
        let bytes = fixture();
        let allowlist = Allowlist::parse(&bytes).unwrap();
        let mut id = [0; ID_BYTES];
        id[..11].copy_from_slice(b"com.tool.gc");
        let id = *allowlist.get(&id).unwrap().id();
        let entry = allowlist.get(&id).unwrap();
        assert_eq!(entry.path(), b"bin/worker");
        assert_eq!(entry.flags(), FLAG_TIMER);
        assert!(allowlist.get(&[0; ID_BYTES]).is_none());
    }

    #[test]
    fn rejects_unknown_flags_noncanonical_paths_and_duplicates() {
        let mut bytes = fixture();
        bytes[88..92].copy_from_slice(&4u32.to_le_bytes());
        assert_eq!(Allowlist::parse(&bytes), Err(Error::Flags));

        let mut bytes = fixture();
        bytes[88..92].copy_from_slice(&FLAG_OWNER_SIGNAL.to_le_bytes());
        assert_eq!(Allowlist::parse(&bytes), Err(Error::Flags));

        let mut bytes = fixture();
        bytes[40..49].copy_from_slice(b"../worker");
        assert_eq!(Allowlist::parse(&bytes), Err(Error::Path));

        let mut bytes = fixture();
        bytes[5] = 2;
        bytes.resize(HEADER_BYTES + ENTRY_BYTES * 2, 0);
        let second = HEADER_BYTES + ENTRY_BYTES;
        let original = bytes[HEADER_BYTES..HEADER_BYTES + ENTRY_BYTES].to_vec();
        bytes[second..second + ENTRY_BYTES].copy_from_slice(&original);
        assert_eq!(Allowlist::parse(&bytes), Err(Error::Duplicate));
    }
}
