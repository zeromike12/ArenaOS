//! Persisted user handler defaults. These records contain only application
//! IDs and content types; they never contain File or Image capabilities.
pub const CONTENT_TYPE_BYTES: usize = 32;
pub const APPLICATION_ID_BYTES: usize = 32;
pub const MAX_DEFAULTS: usize = 32;
const HEADER: usize = 16;
const ENTRY: usize = CONTENT_TYPE_BYTES + APPLICATION_ID_BYTES;
const CHECKSUM: usize = 8;
pub const BYTES: usize = HEADER + MAX_DEFAULTS * ENTRY + CHECKSUM;
const MAGIC: &[u8; 4] = b"ASOC";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DefaultEntry {
    pub content_type: [u8; CONTENT_TYPE_BYTES],
    pub application_id: [u8; APPLICATION_ID_BYTES],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Full,
    InvalidContentType,
    InvalidApplicationId,
}

pub struct Defaults {
    entries: [Option<DefaultEntry>; MAX_DEFAULTS],
    len: usize,
}

impl Defaults {
    pub const fn new() -> Self {
        Self {
            entries: [None; MAX_DEFAULTS],
            len: 0,
        }
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn get(&self, content_type: &[u8]) -> Option<&[u8; APPLICATION_ID_BYTES]> {
        self.entries[..self.len]
            .iter()
            .flatten()
            .find(|entry| field(&entry.content_type) == Some(content_type))
            .map(|entry| &entry.application_id)
    }

    /// Store an explicit user choice. The caller must first verify that the
    /// current installed registry lists this application as a handler.
    pub fn set(&mut self, content_type: &[u8], application_id: &[u8; 32]) -> Result<(), Error> {
        if !valid_content_type(content_type) || content_type.len() >= CONTENT_TYPE_BYTES {
            return Err(Error::InvalidContentType);
        }
        if !valid_application_id(application_id) {
            return Err(Error::InvalidApplicationId);
        }
        let entry = DefaultEntry {
            content_type: fixed(content_type),
            application_id: *application_id,
        };
        let at = self.entries[..self.len]
            .iter()
            .position(|item| item.is_some_and(|item| item.content_type == entry.content_type));
        if let Some(at) = at {
            self.entries[at] = Some(entry);
            return Ok(());
        }
        if self.len == MAX_DEFAULTS {
            return Err(Error::Full);
        }
        let at = self.entries[..self.len]
            .iter()
            .position(|item| item.is_some_and(|item| item.content_type > entry.content_type))
            .unwrap_or(self.len);
        for index in (at..self.len).rev() {
            self.entries[index + 1] = self.entries[index];
        }
        self.entries[at] = Some(entry);
        self.len += 1;
        Ok(())
    }

    /// Remove defaults whose app is no longer installed or no longer
    /// advertises the content type. Returns the number removed.
    pub fn retain(&mut self, mut keep: impl FnMut(&[u8], &[u8; 32]) -> bool) -> usize {
        let before = self.len;
        let mut write = 0;
        for read in 0..self.len {
            let entry = self.entries[read].unwrap();
            let Some(content_type) = field(&entry.content_type) else {
                continue;
            };
            if keep(content_type, &entry.application_id) {
                self.entries[write] = Some(entry);
                write += 1;
            }
        }
        self.entries[write..].fill(None);
        self.len = write;
        before - write
    }

    pub fn encode(&self) -> [u8; BYTES] {
        let mut bytes = [0u8; BYTES];
        bytes[..4].copy_from_slice(MAGIC);
        bytes[4] = 1;
        bytes[6..8].copy_from_slice(&(self.len as u16).to_le_bytes());
        for (index, entry) in self.entries[..self.len].iter().flatten().enumerate() {
            let start = HEADER + index * ENTRY;
            bytes[start..start + CONTENT_TYPE_BYTES].copy_from_slice(&entry.content_type);
            bytes[start + CONTENT_TYPE_BYTES..start + ENTRY].copy_from_slice(&entry.application_id);
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
        if len > MAX_DEFAULTS {
            return None;
        }
        let mut result = Self::new();
        for index in 0..MAX_DEFAULTS {
            let start = HEADER + index * ENTRY;
            let mut content_type = [0; CONTENT_TYPE_BYTES];
            content_type.copy_from_slice(&bytes[start..start + CONTENT_TYPE_BYTES]);
            let mut application_id = [0; APPLICATION_ID_BYTES];
            application_id.copy_from_slice(&bytes[start + CONTENT_TYPE_BYTES..start + ENTRY]);
            if index < len {
                let value = field(&content_type)?;
                if !valid_content_type(value)
                    || value.len() >= CONTENT_TYPE_BYTES
                    || !valid_application_id(&application_id)
                    || (index > 0 && field(&result.entries[index - 1]?.content_type)? >= value)
                {
                    return None;
                }
                result.entries[index] = Some(DefaultEntry {
                    content_type,
                    application_id,
                });
            } else if content_type.iter().any(|byte| *byte != 0)
                || application_id.iter().any(|byte| *byte != 0)
            {
                return None;
            }
        }
        result.len = len;
        Some(result)
    }
}

impl Default for Defaults {
    fn default() -> Self {
        Self::new()
    }
}

fn fixed(value: &[u8]) -> [u8; CONTENT_TYPE_BYTES] {
    let mut out = [0; CONTENT_TYPE_BYTES];
    out[..value.len()].copy_from_slice(value);
    out
}

fn field(value: &[u8; CONTENT_TYPE_BYTES]) -> Option<&[u8]> {
    let end = value.iter().position(|byte| *byte == 0)?;
    value[end..]
        .iter()
        .all(|byte| *byte == 0)
        .then_some(&value[..end])
}

fn valid_content_type(value: &[u8]) -> bool {
    let Some(slash) = value.iter().position(|byte| *byte == b'/') else {
        return false;
    };
    slash > 0
        && slash + 1 < value.len()
        && value.len() < CONTENT_TYPE_BYTES
        && value.iter().all(|byte| {
            byte.is_ascii_lowercase()
                || byte.is_ascii_digit()
                || matches!(*byte, b'/' | b'.' | b'+' | b'-')
        })
}

fn valid_application_id(value: &[u8; 32]) -> bool {
    let Some(end) = value.iter().position(|byte| *byte == 0) else {
        return false;
    };
    end > 0
        && value[end..].iter().all(|byte| *byte == 0)
        && value[..end].iter().all(|byte| (0x21..=0x7e).contains(byte))
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

    fn app_id(value: &[u8]) -> [u8; 32] {
        let mut id = [0; 32];
        id[..value.len()].copy_from_slice(value);
        id
    }

    #[test]
    fn canonical_defaults_round_trip_and_corruption_refuses() {
        let mut defaults = Defaults::new();
        defaults
            .set(b"text/plain", &app_id(b"org.arena.editor"))
            .unwrap();
        defaults
            .set(b"image/png", &app_id(b"org.arena.viewer"))
            .unwrap();
        let bytes = defaults.encode();
        let decoded = Defaults::decode(&bytes).unwrap();
        assert_eq!(
            decoded.get(b"text/plain"),
            Some(&app_id(b"org.arena.editor"))
        );
        assert_eq!(
            decoded.get(b"image/png"),
            Some(&app_id(b"org.arena.viewer"))
        );
        for at in [0, 4, 6, HEADER + 2, BYTES - 1] {
            let mut bad = bytes;
            bad[at] ^= 1;
            assert!(Defaults::decode(&bad).is_none());
        }
        assert!(Defaults::decode(&bytes[..BYTES - 1]).is_none());
    }

    #[test]
    fn replacing_and_pruning_preserve_sorted_canonical_state() {
        let mut defaults = Defaults::new();
        defaults
            .set(b"text/plain", &app_id(b"org.arena.editor"))
            .unwrap();
        defaults
            .set(b"application/pdf", &app_id(b"org.arena.viewer"))
            .unwrap();
        defaults
            .set(b"text/plain", &app_id(b"org.arena.viewer"))
            .unwrap();
        assert_eq!(defaults.len(), 2);
        assert_eq!(
            defaults.retain(
                |content, app| content != b"text/plain" || app == &app_id(b"org.arena.editor")
            ),
            1
        );
        assert_eq!(defaults.get(b"text/plain"), None);
        assert!(Defaults::decode(&defaults.encode()).is_some());
    }

    #[test]
    fn malformed_preference_fields_are_rejected() {
        let mut defaults = Defaults::new();
        assert_eq!(
            defaults.set(b"Text/Plain", &app_id(b"org.arena.editor")),
            Err(Error::InvalidContentType)
        );
        assert_eq!(
            defaults.set(b"text/plain", &[b'x'; 32]),
            Err(Error::InvalidApplicationId)
        );
        assert_eq!(
            defaults.set(b"/plain", &app_id(b"org.arena.editor")),
            Err(Error::InvalidContentType)
        );
    }
}
