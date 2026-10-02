//! GRAPHICS v1 bounded inline IPC frame, **not** an authority token.
//!
//! This pure parser proves only syntax. A future service must check the
//! transferred cap's full object generation and rights for *every* request,
//! and independently verify the input-producer token and client lifecycle.
//!
//! Bytes: [0..4]=AGFX, [4]=version(1), [5]=op, [6..8]=zero;
//! [8..16]=handle LE; [16..20]=signed x LE, [20..24]=signed y LE;
//! [24..26]=w LE, [26..28]=h LE, [28]=key, [29]=pressed;
//! [30..64]=reserved zero. No PID, name, framebuffer address, or
//! capability number appears on this wire. IPC transfers its cap separately.

pub const BYTES: usize = 64;
const MAGIC: &[u8; 4] = b"AGFX";
const VERSION: u8 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WireError {
    Length,
    Magic,
    Version,
    UnknownOp,
    Reserved,
    Geometry,
    Handle,
    Key,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Frame {
    Create {
        x: i32,
        y: i32,
        w: u16,
        h: u16,
    },
    Move {
        handle: u64,
        x: i32,
        y: i32,
    },
    Damage {
        handle: u64,
        x: i32,
        y: i32,
        w: u16,
        h: u16,
    },
    Focus {
        handle: u64,
    },
    Destroy {
        handle: u64,
    },
    Key {
        ascii: u8,
        pressed: bool,
    },
    Present {
        x: i32,
        y: i32,
        w: u16,
        h: u16,
    },
    Mode,
}
impl Frame {
    fn fields(self) -> (u8, u64, i32, i32, u16, u16, u8, u8) {
        match self {
            Self::Create { x, y, w, h } => (1, 0, x, y, w, h, 0, 0),
            Self::Move { handle, x, y } => (2, handle, x, y, 0, 0, 0, 0),
            Self::Damage { handle, x, y, w, h } => (3, handle, x, y, w, h, 0, 0),
            Self::Focus { handle } => (4, handle, 0, 0, 0, 0, 0, 0),
            Self::Destroy { handle } => (5, handle, 0, 0, 0, 0, 0, 0),
            Self::Key { ascii, pressed } => (6, 0, 0, 0, 0, 0, ascii, u8::from(pressed)),
            Self::Present { x, y, w, h } => (7, 0, x, y, w, h, 0, 0),
            Self::Mode => (8, 0, 0, 0, 0, 0, 0, 0),
        }
    }
    /// Refuse invalid content *before* touching a caller-owned buffer.
    pub fn encode(self, out: &mut [u8; BYTES]) -> Result<(), WireError> {
        self.check()?;
        let (op, handle, x, y, w, h, key, pressed) = self.fields();
        let mut b = [0u8; BYTES];
        b[0..4].copy_from_slice(MAGIC);
        b[4] = VERSION;
        b[5] = op;
        b[8..16].copy_from_slice(&handle.to_le_bytes());
        b[16..20].copy_from_slice(&x.to_le_bytes());
        b[20..24].copy_from_slice(&y.to_le_bytes());
        b[24..26].copy_from_slice(&w.to_le_bytes());
        b[26..28].copy_from_slice(&h.to_le_bytes());
        b[28] = key;
        b[29] = pressed;
        *out = b;
        Ok(())
    }
    /// Parse exactly one 64-byte IPC payload with zero reserved bytes.
    /// Refusal does not construct a partially authorized surface.
    pub fn decode(b: &[u8]) -> Result<Self, WireError> {
        if b.len() != BYTES {
            return Err(WireError::Length);
        }
        if &b[0..4] != MAGIC {
            return Err(WireError::Magic);
        }
        if b[4] != VERSION {
            return Err(WireError::Version);
        }
        if b[6..8].iter().chain(b[30..].iter()).any(|&x| x != 0) {
            return Err(WireError::Reserved);
        }
        let handle = u64::from_le_bytes(b[8..16].try_into().unwrap());
        let x = i32::from_le_bytes(b[16..20].try_into().unwrap());
        let y = i32::from_le_bytes(b[20..24].try_into().unwrap());
        let w = u16::from_le_bytes(b[24..26].try_into().unwrap());
        let h = u16::from_le_bytes(b[26..28].try_into().unwrap());
        let (key, pressed) = (b[28], b[29]);
        let frame = match b[5] {
            1 => Self::Create { x, y, w, h },
            2 => Self::Move { handle, x, y },
            3 => Self::Damage { handle, x, y, w, h },
            4 => Self::Focus { handle },
            5 => Self::Destroy { handle },
            6 if pressed <= 1 => Self::Key {
                ascii: key,
                pressed: pressed == 1,
            },
            6 => return Err(WireError::Key),
            7 => Self::Present { x, y, w, h },
            8 => Self::Mode,
            _ => return Err(WireError::UnknownOp),
        };
        frame.check()?;
        if frame.fields() != (b[5], handle, x, y, w, h, key, pressed) {
            return Err(WireError::Reserved);
        }
        Ok(frame)
    }
    fn check(self) -> Result<(), WireError> {
        let (op, handle, x, y, w, h, key, _) = self.fields();
        if matches!(op, 2..=5) && handle == 0 {
            return Err(WireError::Handle);
        }
        if op == 6 && !(b' '..=b'~').contains(&key) {
            return Err(WireError::Key);
        }
        if matches!(op, 1 | 3) && (w == 0 || h == 0 || w > 320 || h > 200) {
            return Err(WireError::Geometry);
        }
        if op == 7
            && (w == 0
                || h == 0
                || w > 1024
                || h > 768
                || x < 0
                || y < 0
                || u64::from(w) * u64::from(h) * 4 > 512 * 4096)
        {
            return Err(WireError::Geometry);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_versioned_golden_bytes_and_roundtrips() {
        let mut b = [0xff; BYTES];
        let create = Frame::Create {
            x: -1,
            y: 8,
            w: 320,
            h: 200,
        };
        create.encode(&mut b).unwrap();
        assert_eq!(&b[..8], b"AGFX\x01\x01\0\0");
        assert_eq!(&b[8..16], &[0; 8]);
        assert_eq!(&b[16..28], &[255, 255, 255, 255, 8, 0, 0, 0, 64, 1, 200, 0]);
        assert_eq!(&b[28..], &[0; 36]);
        assert_eq!(Frame::decode(&b), Ok(create));
        for frame in [
            Frame::Mode,
            Frame::Focus { handle: 7 },
            Frame::Destroy { handle: 7 },
            Frame::Move {
                handle: 4,
                x: i32::MAX,
                y: i32::MIN,
            },
            Frame::Damage {
                handle: 4,
                x: -1,
                y: 0,
                w: 4,
                h: 5,
            },
            Frame::Key {
                ascii: b'K',
                pressed: false,
            },
            Frame::Present {
                x: 1,
                y: 2,
                w: 800,
                h: 600,
            },
        ] {
            frame.encode(&mut b).unwrap();
            assert_eq!(Frame::decode(&b), Ok(frame));
        }
    }
    #[test]
    fn malformed_opcode_reserved_and_bearerless_handle_refuse() {
        let mut b = [0u8; BYTES];
        Frame::Mode.encode(&mut b).unwrap();
        assert_eq!(Frame::decode(&b[..63]), Err(WireError::Length));
        b[0] = 0;
        assert_eq!(Frame::decode(&b), Err(WireError::Magic));
        b[0] = b'A';
        b[4] = 2;
        assert_eq!(Frame::decode(&b), Err(WireError::Version));
        b[4] = 1;
        b[5] = 255;
        assert_eq!(Frame::decode(&b), Err(WireError::UnknownOp));
        b[5] = 8;
        b[63] = 1;
        assert_eq!(Frame::decode(&b), Err(WireError::Reserved));
        b[63] = 0;
        b[8] = 1;
        assert_eq!(Frame::decode(&b), Err(WireError::Reserved));
        b[8] = 0;
        b[5] = 4;
        assert_eq!(Frame::decode(&b), Err(WireError::Handle));
        let old = b;
        assert_eq!(
            Frame::Focus { handle: 0 }.encode(&mut b),
            Err(WireError::Handle)
        );
        assert_eq!(b, old);
        b[5] = 6;
        b[28] = 1;
        assert_eq!(Frame::decode(&b), Err(WireError::Key));
    }
    #[test]
    fn hostile_inline_frame_fuzz_never_reads_short_buffers() {
        let mut bytes = [0u8; BYTES];
        let mut state = 0xd1b5_4a32_8eef_134bu64;
        for iteration in 0..=2048 {
            for b in &mut bytes {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                *b = state as u8;
            }
            let length = iteration % (BYTES + 1);
            let result = Frame::decode(&bytes[..length]);
            if length < BYTES {
                assert_eq!(result, Err(WireError::Length));
            }
        }
    }

    #[test]
    fn exact_reserved_bytes_and_geometry_not_cap_authority() {
        let mut b = [0u8; BYTES];
        Frame::Create {
            x: 0,
            y: 0,
            w: 1,
            h: 1,
        }
        .encode(&mut b)
        .unwrap();
        b[27] = 1;
        assert_eq!(Frame::decode(&b), Err(WireError::Geometry));
        b[27] = 0;
        b[29] = 1;
        assert_eq!(Frame::decode(&b), Err(WireError::Reserved));
        b[29] = 0;
        assert_eq!(
            Frame::decode(&b),
            Ok(Frame::Create {
                x: 0,
                y: 0,
                w: 1,
                h: 1
            })
        );
        let old = b;
        assert_eq!(
            Frame::Present {
                x: 0,
                y: 0,
                w: 1024,
                h: 768
            }
            .encode(&mut b),
            Err(WireError::Geometry)
        );
        assert_eq!(b, old);
    }
}
