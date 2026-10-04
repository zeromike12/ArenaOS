//! Authenticated producer wire, separate from ordinary client requests.
//! The decoder checks syntax only; the service must compare the landed
//! producer ProofToken to its held boot-root witness before acting.
pub const BYTES: usize = 64;
pub const LAUNCH_FIRST: u16 = 263;
pub const LAUNCH_LAST: u16 = 268;
pub const FOCUS_NEXT: u16 = 269;
pub const CLOSE_FOCUSED: u16 = 270;
/// Not a key: the modifier state changed (Alt release commits the
/// switcher).
pub const MODIFIERS: u16 = 271;
pub const MAX_KEY: u16 = MODIFIERS;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Frame {
    /// A key press or release with the modifier bits held at the time
    /// (shift 1, ctrl 2, alt 4, super 8). `MODIFIERS` reports only a
    /// modifier change.
    Key { code: u16, pressed: bool, mods: u8 },
    /// Absolute tablet position, buttons and wheel notches in the batch.
    Pointer {
        x: u16,
        y: u16,
        buttons: u8,
        wheel: i8,
    },
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Invalid,
}
impl Frame {
    pub fn encode(self) -> Result<[u8; BYTES], Error> {
        let mut b = [0; BYTES];
        b[..4].copy_from_slice(b"AINP");
        b[4] = 1;
        match self {
            Self::Key {
                code,
                pressed,
                mods,
            } => {
                if code > MAX_KEY || mods > 15 {
                    return Err(Error::Invalid);
                }
                b[5] = 1;
                b[8..10].copy_from_slice(&code.to_le_bytes());
                b[12] = u8::from(pressed);
                b[13] = mods;
            }
            Self::Pointer {
                x,
                y,
                buttons,
                wheel,
            } => {
                if x > 32767 || y > 32767 || buttons > 7 {
                    return Err(Error::Invalid);
                }
                b[5] = 2;
                b[8..10].copy_from_slice(&x.to_le_bytes());
                b[10..12].copy_from_slice(&y.to_le_bytes());
                b[12] = buttons;
                b[13] = wheel as u8;
            }
        }
        Ok(b)
    }
    pub fn decode(b: &[u8]) -> Result<Self, Error> {
        if b.len() != BYTES || &b[..4] != b"AINP" || b[4] != 1 {
            return Err(Error::Invalid);
        }
        let x = u16::from_le_bytes(b[8..10].try_into().map_err(|_| Error::Invalid)?);
        let y = u16::from_le_bytes(b[10..12].try_into().map_err(|_| Error::Invalid)?);
        let f = match b[5] {
            1 if b[12] <= 1 => Self::Key {
                code: x,
                pressed: b[12] == 1,
                mods: b[13],
            },
            2 => Self::Pointer {
                x,
                y,
                buttons: b[12],
                wheel: b[13] as i8,
            },
            _ => return Err(Error::Invalid),
        };
        if f.encode()?.as_slice() != b {
            return Err(Error::Invalid);
        }
        Ok(f)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn canonical_control_keys_pointer_and_reserved_fields() {
        for f in [
            Frame::Key {
                code: 8,
                pressed: true,
                mods: 0,
            },
            Frame::Key {
                code: 13,
                pressed: true,
                mods: 3,
            },
            Frame::Key {
                code: 256,
                pressed: false,
                mods: 8,
            },
            Frame::Key {
                code: MODIFIERS,
                pressed: true,
                mods: 0,
            },
            Frame::Pointer {
                x: 32767,
                y: 1,
                buttons: 7,
                wheel: -3,
            },
        ] {
            let b = f.encode().unwrap();
            assert_eq!(Frame::decode(&b), Ok(f));
            for at in [6, 7, 14, 63] {
                let mut bad = b;
                bad[at] = 1;
                assert_eq!(Frame::decode(&bad), Err(Error::Invalid));
            }
        }
        assert!(
            Frame::Pointer {
                x: 32768,
                y: 0,
                buttons: 0,
                wheel: 0
            }
            .encode()
            .is_err()
        );
        assert!(
            Frame::Key {
                code: MAX_KEY + 1,
                pressed: true,
                mods: 0
            }
            .encode()
            .is_err()
        );
    }
}
