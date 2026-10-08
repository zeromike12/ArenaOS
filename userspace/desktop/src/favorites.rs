//! Persisted descriptive application pins. A favorite is only a registry
//! preference; launch still resolves through the verified live catalog.
use crate::apps::APPLICATION_IDS;

pub const MAX_FAVORITES: usize = 8;
const HEADER: usize = 16;
const ID_BYTES: usize = 32;
const CHECKSUM: usize = 8;
pub const BYTES: usize = HEADER + MAX_FAVORITES * ID_BYTES + CHECKSUM;
const MAGIC: &[u8; 4] = b"AFAV";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Full,
    InvalidApplicationId,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Favorites {
    entries: [[u8; ID_BYTES]; MAX_FAVORITES],
    len: usize,
}

impl Favorites {
    pub const fn new() -> Self {
        Self {
            entries: [[0; ID_BYTES]; MAX_FAVORITES],
            len: 0,
        }
    }

    /// Initial compatibility preference: preserve the six existing dock
    /// entries on a new AFS2 profile. Users can unpin any of them.
    pub fn builtins() -> Self {
        let mut favorites = Self::new();
        for application_id in APPLICATION_IDS {
            let _ = favorites.add(&application_id);
        }
        favorites
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn contains(&self, application_id: &[u8; ID_BYTES]) -> bool {
        self.entries[..self.len].contains(application_id)
    }

    pub fn get(&self, index: usize) -> Option<&[u8; ID_BYTES]> {
        self.entries.get(index).filter(|_| index < self.len)
    }

    /// Add one descriptive application ID. The caller must resolve it
    /// against the current catalog before using the result to launch.
    pub fn add(&mut self, application_id: &[u8; ID_BYTES]) -> Result<(), Error> {
        if !valid_application_id(application_id) {
            return Err(Error::InvalidApplicationId);
        }
        if self.contains(application_id) {
            return Ok(());
        }
        if self.len == MAX_FAVORITES {
            return Err(Error::Full);
        }
        self.entries[self.len] = *application_id;
        self.len += 1;
        Ok(())
    }

    /// Toggle a pinned entry and return its new state (`true` means pinned).
    pub fn toggle(&mut self, application_id: &[u8; ID_BYTES]) -> Result<bool, Error> {
        if !valid_application_id(application_id) {
            return Err(Error::InvalidApplicationId);
        }
        if let Some(index) = self.entries[..self.len]
            .iter()
            .position(|entry| entry == application_id)
        {
            self.entries.copy_within(index + 1..self.len, index);
            self.len -= 1;
            self.entries[self.len] = [0; ID_BYTES];
            return Ok(false);
        }
        self.add(application_id)?;
        Ok(true)
    }

    /// Remove IDs that no longer resolve in the verified application catalog.
    pub fn retain(&mut self, mut keep: impl FnMut(&[u8; ID_BYTES]) -> bool) -> usize {
        let before = self.len;
        let mut write = 0;
        for read in 0..before {
            let entry = self.entries[read];
            if keep(&entry) {
                self.entries[write] = entry;
                write += 1;
            }
        }
        self.entries[write..].fill([0; ID_BYTES]);
        self.len = write;
        before - write
    }

    pub fn encode(&self) -> [u8; BYTES] {
        let mut bytes = [0u8; BYTES];
        bytes[..4].copy_from_slice(MAGIC);
        bytes[4] = 1;
        bytes[6..8].copy_from_slice(&(self.len as u16).to_le_bytes());
        for (index, application_id) in self.entries[..self.len].iter().enumerate() {
            let start = HEADER + index * ID_BYTES;
            bytes[start..start + ID_BYTES].copy_from_slice(application_id);
        }
        let sum = checksum(&bytes[..BYTES - CHECKSUM]);
        bytes[BYTES - CHECKSUM..].copy_from_slice(&sum.to_le_bytes());
        bytes
    }

    pub fn decode(bytes: &[u8]) -> Option<Self> {
        if bytes.len() != BYTES
            || &bytes[..4] != MAGIC
            || bytes[4] != 1
            || bytes[5] != 0
            || bytes[8..HEADER].iter().any(|byte| *byte != 0)
            || checksum(&bytes[..BYTES - CHECKSUM])
                != u64::from_le_bytes(bytes[BYTES - CHECKSUM..].try_into().ok()?)
        {
            return None;
        }
        let len = u16::from_le_bytes(bytes[6..8].try_into().ok()?) as usize;
        if len > MAX_FAVORITES {
            return None;
        }
        let mut favorites = Self::new();
        for index in 0..MAX_FAVORITES {
            let start = HEADER + index * ID_BYTES;
            let mut application_id = [0; ID_BYTES];
            application_id.copy_from_slice(&bytes[start..start + ID_BYTES]);
            if index < len {
                if !valid_application_id(&application_id) || favorites.contains(&application_id) {
                    return None;
                }
                favorites.entries[index] = application_id;
            } else if application_id.iter().any(|byte| *byte != 0) {
                return None;
            }
        }
        favorites.len = len;
        Some(favorites)
    }
}

impl Default for Favorites {
    fn default() -> Self {
        Self::builtins()
    }
}

fn valid_application_id(value: &[u8; ID_BYTES]) -> bool {
    let Some(end) = value.iter().position(|byte| *byte == 0) else {
        return false;
    };
    end > 0
        && value[end..].iter().all(|byte| *byte == 0)
        && value[..end].iter().enumerate().all(|(index, byte)| {
            byte.is_ascii_lowercase()
                || byte.is_ascii_digit()
                || (index != 0 && matches!(*byte, b'.' | b'-' | b'_'))
        })
}

fn checksum(bytes: &[u8]) -> u64 {
    let mut sum = 0xcbf2_9ce4_8422_2325u64;
    for byte in bytes {
        sum ^= u64::from(*byte);
        sum = sum.wrapping_mul(0x0000_0100_0000_01b3);
    }
    sum
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app_id(value: &[u8]) -> [u8; ID_BYTES] {
        let mut id = [0; ID_BYTES];
        id[..value.len()].copy_from_slice(value);
        id
    }

    #[test]
    fn built_in_defaults_round_trip_and_user_toggle_is_persistent() {
        let mut favorites = Favorites::builtins();
        let app = app_id(b"org.arenaos.phase13app");
        assert_eq!(favorites.len(), 6);
        assert_eq!(favorites.toggle(&app), Ok(true));
        assert_eq!(favorites.toggle(&app), Ok(false));
        assert_eq!(favorites.toggle(&app), Ok(true));
        let bytes = favorites.encode();
        let decoded = Favorites::decode(&bytes).unwrap();
        assert_eq!(decoded, favorites);
        assert!(decoded.contains(&app));
    }

    #[test]
    fn stale_ids_are_pruned_without_reordering_survivors() {
        let mut favorites = Favorites::new();
        let a = app_id(b"org.arena.a");
        let b = app_id(b"org.arena.b");
        let c = app_id(b"org.arena.c");
        favorites.add(&a).unwrap();
        favorites.add(&b).unwrap();
        favorites.add(&c).unwrap();
        assert_eq!(favorites.retain(|id| id != &b), 1);
        assert_eq!(favorites.get(0), Some(&a));
        assert_eq!(favorites.get(1), Some(&c));
    }

    #[test]
    fn capacity_invalid_ids_duplicates_and_corruption_are_bounded() {
        let mut favorites = Favorites::new();
        for index in 0..MAX_FAVORITES {
            let mut name = [0; 32];
            name[..11].copy_from_slice(b"org.arena.x");
            name[11] = b'0' + index as u8;
            favorites.add(&name).unwrap();
        }
        let full = app_id(b"org.arena.full");
        assert_eq!(favorites.add(&full), Err(Error::Full));
        let duplicate = *favorites.get(0).unwrap();
        assert_eq!(favorites.add(&duplicate), Ok(()));
        assert_eq!(favorites.add(&[b'X'; 32]), Err(Error::InvalidApplicationId));
        let mut bytes = favorites.encode();
        bytes[HEADER + 2] ^= 1;
        assert!(Favorites::decode(&bytes).is_none());
    }
}
