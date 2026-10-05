//! Function-service and startup metadata. No visual tokens or authorization
//! identity cross this wire; the held capability is carried separately.
pub const BYTES: usize = 64;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Frame {
    Bootstrap,
    Started {
        kind: u8,
        theme: u8,
        motion: bool,
        path: [u8; 32],
    },
    List {
        cursor: u32,
    },
    Entry {
        cursor: u32,
        size: u64,
        name: [u8; 32],
    },
    Read {
        name: [u8; 32],
    },
    Put {
        name: [u8; 32],
        length: u16,
    },
    Delete {
        name: [u8; 32],
    },
    Configure {
        theme: u8,
        motion: bool,
    },
    Launch {
        kind: u8,
        path: [u8; 32],
    },
    LaunchImage,
    Display,
    Create {
        name: [u8; 32],
    },
    /// Ask the broker's trusted chooser for a document to open (or a place
    /// to save, `name` suggested). Nothing is granted by the request.
    Choose {
        save: bool,
        /// Open for reading only (never with `save`).
        read_only: bool,
        name: [u8; 32],
    },
    /// The lent capability (a filesd file capability) is offered for the
    /// next application this session launches.
    Offer,
    /// Collect the chooser's outcome: the reply carries the granted
    /// capability, or none when the user cancelled.
    TakeGrant,
    /// Reply to TakeGrant: the title of what was chosen (display only).
    Granted {
        save: bool,
        read_only: bool,
        name: [u8; 32],
    },
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Invalid,
}
fn name_valid(name: &[u8; 32], empty: bool) -> bool {
    let n = name.iter().position(|b| *b == 0).unwrap_or(32);
    (empty || n > 0)
        && n < 32
        && name[..n].iter().all(|b| b.is_ascii_graphic() && *b != b'/')
        && name[n..].iter().all(|b| *b == 0)
}
/// A display title: printable ASCII then zero padding (never authority).
fn printable(name: &[u8; 32]) -> bool {
    let n = name.iter().position(|b| *b == 0).unwrap_or(32);
    n < 32
        && name[..n].iter().all(|b| (0x20..0x7f).contains(b))
        && name[n..].iter().all(|b| *b == 0)
}
impl Frame {
    pub fn encode(self) -> Result<[u8; BYTES], Error> {
        let mut b = [0u8; BYTES];
        b[..4].copy_from_slice(b"ASVC");
        b[4] = 1;
        b[5] = match self {
            Self::Bootstrap => 1,
            Self::Started {
                kind,
                theme,
                motion,
                path,
            } => {
                if kind > 5 || theme > 1 || !name_valid(&path, true) {
                    return Err(Error::Invalid);
                }
                b[6] = kind;
                b[7] = theme;
                b[8] = u8::from(motion);
                b[32..].copy_from_slice(&path);
                2
            }
            Self::List { cursor } => {
                b[12..16].copy_from_slice(&cursor.to_le_bytes());
                3
            }
            Self::Entry { cursor, size, name } => {
                if !name_valid(&name, true) {
                    return Err(Error::Invalid);
                }
                b[12..16].copy_from_slice(&cursor.to_le_bytes());
                b[16..24].copy_from_slice(&size.to_le_bytes());
                b[32..].copy_from_slice(&name);
                4
            }
            Self::Read { name } | Self::Delete { name } | Self::Create { name } => {
                if !name_valid(&name, false) {
                    return Err(Error::Invalid);
                }
                b[32..].copy_from_slice(&name);
                if matches!(self, Self::Read { .. }) {
                    5
                } else if matches!(self, Self::Delete { .. }) {
                    7
                } else {
                    12
                }
            }
            Self::Put { name, length } => {
                if !name_valid(&name, false) || length > 4096 {
                    return Err(Error::Invalid);
                }
                b[10..12].copy_from_slice(&length.to_le_bytes());
                b[32..].copy_from_slice(&name);
                6
            }
            Self::Configure { theme, motion } => {
                if theme > 1 {
                    return Err(Error::Invalid);
                }
                b[7] = theme;
                b[8] = u8::from(motion);
                8
            }
            Self::Launch { kind, path } => {
                if kind > 5 || !name_valid(&path, true) {
                    return Err(Error::Invalid);
                }
                b[6] = kind;
                b[32..].copy_from_slice(&path);
                9
            }
            Self::LaunchImage => 10,
            Self::Display => 11,
            Self::Choose {
                save,
                read_only,
                name,
            }
            | Self::Granted {
                save,
                read_only,
                name,
            } => {
                if !printable(&name) || (save && read_only) {
                    return Err(Error::Invalid);
                }
                b[8] = u8::from(save);
                b[9] = u8::from(read_only);
                b[32..].copy_from_slice(&name);
                if matches!(self, Self::Choose { .. }) {
                    13
                } else {
                    16
                }
            }
            Self::Offer => 14,
            Self::TakeGrant => 15,
        };
        Ok(b)
    }
    pub fn decode(b: &[u8]) -> Result<Self, Error> {
        if b.len() != BYTES || &b[..4] != b"ASVC" || b[4] != 1 {
            return Err(Error::Invalid);
        }
        let cursor = u32::from_le_bytes(b[12..16].try_into().map_err(|_| Error::Invalid)?);
        let name = b[32..].try_into().map_err(|_| Error::Invalid)?;
        let f = match b[5] {
            1 => Self::Bootstrap,
            2 if b[8] <= 1 => Self::Started {
                kind: b[6],
                theme: b[7],
                motion: b[8] == 1,
                path: name,
            },
            3 => Self::List { cursor },
            4 => Self::Entry {
                cursor,
                size: u64::from_le_bytes(b[16..24].try_into().map_err(|_| Error::Invalid)?),
                name,
            },
            5 => Self::Read { name },
            6 => Self::Put {
                name,
                length: u16::from_le_bytes(b[10..12].try_into().map_err(|_| Error::Invalid)?),
            },
            7 => Self::Delete { name },
            8 if b[8] <= 1 => Self::Configure {
                theme: b[7],
                motion: b[8] == 1,
            },
            9 => Self::Launch {
                kind: b[6],
                path: name,
            },
            10 => Self::LaunchImage,
            11 => Self::Display,
            12 => Self::Create { name },
            13 if b[8] <= 1 && b[9] <= 1 => Self::Choose {
                save: b[8] == 1,
                read_only: b[9] == 1,
                name,
            },
            14 => Self::Offer,
            15 => Self::TakeGrant,
            16 if b[8] <= 1 && b[9] <= 1 => Self::Granted {
                save: b[8] == 1,
                read_only: b[9] == 1,
                name,
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
    fn canonical_metadata_and_operations_do_not_accept_reserved_or_names_as_authority() {
        let mut name = [0; 32];
        name[..9].copy_from_slice(b"user-note");
        for f in [
            Frame::Bootstrap,
            Frame::Started {
                kind: 0,
                theme: 1,
                motion: false,
                path: name,
            },
            Frame::List { cursor: u32::MAX },
            Frame::Entry {
                cursor: 4,
                size: 9000,
                name,
            },
            Frame::Read { name },
            Frame::Put { name, length: 4096 },
            Frame::Delete { name },
            Frame::Create { name },
            Frame::Configure {
                theme: 0,
                motion: true,
            },
            Frame::Launch {
                kind: 5,
                path: [0; 32],
            },
            Frame::LaunchImage,
            Frame::Display,
            Frame::Choose {
                save: true,
                read_only: false,
                name,
            },
            Frame::Offer,
            Frame::TakeGrant,
            Frame::Granted {
                save: false,
                read_only: true,
                name,
            },
        ] {
            let b = f.encode().unwrap();
            assert_eq!(Frame::decode(&b), Ok(f));
            let flags = matches!(f, Frame::Choose { .. } | Frame::Granted { .. });
            for i in [9, 24, 25, 26, 27, 28, 29, 30, 31] {
                if flags && i == 9 {
                    continue;
                }
                let mut bad = b;
                bad[i] = 1;
                assert_eq!(Frame::decode(&bad), Err(Error::Invalid));
            }
            assert!(Frame::decode(&b[..63]).is_err());
        }
        assert!(Frame::Read { name: [0; 32] }.encode().is_err());
        // A save is never read-only.
        assert!(
            Frame::Choose {
                save: true,
                read_only: true,
                name
            }
            .encode()
            .is_err()
        );
        assert!(Frame::Put { name, length: 4097 }.encode().is_err());
        let mut bad = name;
        bad[1] = b'/';
        assert!(Frame::Read { name: bad }.encode().is_err());
    }
}
