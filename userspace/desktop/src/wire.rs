//! Neutral desktop client wire. A parsed handle never grants authority.
use crate::model::Event;
pub const BYTES: usize = 64;
const MAGIC: &[u8; 4] = b"ADSK";
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Invalid,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Frame {
    Create { width: u16, height: u16 },
    Damage { handle: u64 },
    Poll { handle: u64 },
    Title { handle: u64, text: [u8; 32] },
    Event { handle: u64, event: Event },
}
impl Frame {
    pub fn encode(self) -> Result<[u8; BYTES], Error> {
        let mut b = [0; BYTES];
        b[..4].copy_from_slice(MAGIC);
        b[4] = 1;
        let (op, handle) = match self {
            Self::Create { width, height } => {
                if width < 80 || height < 60 || width > 448 || height > 288 {
                    return Err(Error::Invalid);
                }
                b[24..26].copy_from_slice(&width.to_le_bytes());
                b[26..28].copy_from_slice(&height.to_le_bytes());
                (1, 0)
            }
            Self::Damage { handle } => (2, handle),
            Self::Poll { handle } => (3, handle),
            Self::Title { handle, text } => {
                let end = text.iter().position(|&x| x == 0).unwrap_or(32);
                if end == 0
                    || !text[..end]
                        .iter()
                        .all(|x| x.is_ascii_graphic() || *x == b' ')
                    || text[end..].iter().any(|&x| x != 0)
                {
                    return Err(Error::Invalid);
                }
                b[32..].copy_from_slice(&text);
                (4, handle)
            }
            Self::Event { handle, event } => {
                match event {
                    Event::Key(key) => {
                        if key > 262 {
                            return Err(Error::Invalid);
                        }
                        b[28] = 1;
                        b[24..26].copy_from_slice(&key.to_le_bytes());
                    }
                    Event::Pointer { x, y, buttons } => {
                        if !(-448..=448).contains(&x) || !(-288..=288).contains(&y) || buttons > 7 {
                            return Err(Error::Invalid);
                        }
                        b[28] = 2;
                        b[29] = buttons;
                        b[16..20].copy_from_slice(&x.to_le_bytes());
                        b[20..24].copy_from_slice(&y.to_le_bytes());
                    }
                    Event::Close => {
                        b[28] = 3;
                    }
                    Event::Focus(f) => {
                        b[28] = 4;
                        b[29] = u8::from(f);
                    }
                }
                (5, handle)
            }
        };
        if op != 1 && handle == 0 {
            return Err(Error::Invalid);
        }
        b[5] = op;
        b[8..16].copy_from_slice(&handle.to_le_bytes());
        Ok(b)
    }
    pub fn decode(b: &[u8]) -> Result<Self, Error> {
        if b.len() != BYTES || &b[..4] != MAGIC || b[4] != 1 {
            return Err(Error::Invalid);
        }
        let handle = u64::from_le_bytes(b[8..16].try_into().map_err(|_| Error::Invalid)?);
        let width = u16::from_le_bytes(b[24..26].try_into().map_err(|_| Error::Invalid)?);
        let height = u16::from_le_bytes(b[26..28].try_into().map_err(|_| Error::Invalid)?);
        let x = i32::from_le_bytes(b[16..20].try_into().map_err(|_| Error::Invalid)?);
        let y = i32::from_le_bytes(b[20..24].try_into().map_err(|_| Error::Invalid)?);
        let f = match b[5] {
            1 => Self::Create { width, height },
            2 => Self::Damage { handle },
            3 => Self::Poll { handle },
            4 => Self::Title {
                handle,
                text: b[32..].try_into().map_err(|_| Error::Invalid)?,
            },
            5 => Self::Event {
                handle,
                event: match b[28] {
                    1 => Event::Key(width),
                    2 => Event::Pointer {
                        x,
                        y,
                        buttons: b[29],
                    },
                    3 => Event::Close,
                    4 if b[29] <= 1 => Event::Focus(b[29] == 1),
                    _ => return Err(Error::Invalid),
                },
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
    fn strict_canonical_frames_and_reserved_refusal() {
        let mut title = [0; 32];
        title[..7].copy_from_slice(b"Gallery");
        for f in [
            Frame::Create {
                width: 448,
                height: 288,
            },
            Frame::Damage { handle: 9 },
            Frame::Poll { handle: u64::MAX },
            Frame::Title {
                handle: 9,
                text: title,
            },
            Frame::Event {
                handle: 9,
                event: Event::Pointer {
                    x: -1,
                    y: 288,
                    buttons: 7,
                },
            },
            Frame::Event {
                handle: 9,
                event: Event::Key(262),
            },
            Frame::Event {
                handle: 9,
                event: Event::Close,
            },
            Frame::Event {
                handle: 9,
                event: Event::Focus(false),
            },
        ] {
            let b = f.encode().unwrap();
            assert_eq!(Frame::decode(&b), Ok(f));
            for i in [6, 7, 30, 31] {
                let mut mutant = b;
                mutant[i] = 1;
                assert_eq!(Frame::decode(&mutant), Err(Error::Invalid));
            }
            assert_eq!(Frame::decode(&b[..63]), Err(Error::Invalid));
        }
        assert!(Frame::Poll { handle: 0 }.encode().is_err());
        assert!(
            Frame::Create {
                width: 449,
                height: 288
            }
            .encode()
            .is_err()
        );
        assert!(
            Frame::Event {
                handle: 9,
                event: Event::Key(263)
            }
            .encode()
            .is_err()
        );
    }
}
