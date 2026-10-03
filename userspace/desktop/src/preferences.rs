//! Ordinary application data in ui10-prefs; AFS1 and configd formats remain
//! unchanged. Decoder rejects unknown versions, padding and corruption.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Preferences {
    pub dark: bool,
    pub motion: bool,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            dark: false,
            motion: true,
        }
    }
}
fn hash(data: &[u8]) -> u64 {
    let mut h = 0xcbf29ce484222325u64;
    for b in data {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}
impl Preferences {
    pub fn encode(self) -> [u8; 16] {
        let mut b = [0u8; 16];
        b[..4].copy_from_slice(b"UI10");
        b[4] = 1;
        b[5] = u8::from(self.dark);
        b[6] = u8::from(self.motion);
        let sum = hash(&b[..8]);
        b[8..].copy_from_slice(&sum.to_le_bytes());
        b
    }
    pub fn decode(bytes: &[u8]) -> Option<Self> {
        if bytes.len() != 16
            || &bytes[..4] != b"UI10"
            || bytes[4] != 1
            || bytes[5] > 1
            || bytes[6] > 1
            || bytes[7] != 0
            || hash(&bytes[..8]) != u64::from_le_bytes(bytes[8..].try_into().ok()?)
        {
            return None;
        }
        Some(Self {
            dark: bytes[5] == 1,
            motion: bytes[6] == 1,
        })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn canonical_durable_choices() {
        for dark in [false, true] {
            for motion in [false, true] {
                let p = Preferences { dark, motion };
                let b = p.encode();
                assert_eq!(Preferences::decode(&b), Some(p));
                for i in 0..16 {
                    let mut bad = b;
                    bad[i] ^= 128;
                    assert_eq!(Preferences::decode(&bad), None);
                }
                assert_eq!(Preferences::decode(&b[..15]), None);
            }
        }
    }
}
