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
    /// Present up to four rectangles in one call (Phase 11 latency): the
    /// same scanout authority as `Present`; every rectangle is checked
    /// before any pixel moves. Unused entries are zero.
    PresentRects {
        n: u8,
        rects: [[u16; 4]; PRESENT_RECTS],
    },
    /// Check the calling owner's key queue. A live server requires an
    /// independently verified, root-provisioned region cap with this handle.
    Poll {
        handle: u64,
    },
}
/// Rectangles one `PresentRects` frame carries.
pub const PRESENT_RECTS: usize = 4;

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
            Self::Poll { handle } => (9, handle, 0, 0, 0, 0, 0, 0),
            Self::PresentRects { n, .. } => (10, 0, 0, 0, 0, 0, n, 0),
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
        if let Self::PresentRects { rects, .. } = self {
            for (i, r) in rects.iter().enumerate() {
                for (j, v) in r.iter().enumerate() {
                    let at = 32 + i * 8 + j * 2;
                    b[at..at + 2].copy_from_slice(&v.to_le_bytes());
                }
            }
        }
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
        // PresentRects carries its rectangles in bytes 32..64.
        let tail = if b[5] == 10 { &b[30..32] } else { &b[30..] };
        if b[6..8].iter().chain(tail.iter()).any(|&x| x != 0) {
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
            9 => Self::Poll { handle },
            10 => {
                let mut rects = [[0u16; 4]; PRESENT_RECTS];
                for (i, r) in rects.iter_mut().enumerate() {
                    for (j, v) in r.iter_mut().enumerate() {
                        let at = 32 + i * 8 + j * 2;
                        *v = u16::from_le_bytes([b[at], b[at + 1]]);
                    }
                }
                Self::PresentRects { n: key, rects }
            }
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
        if matches!(op, 2..=5 | 9) && handle == 0 {
            return Err(WireError::Handle);
        }
        if op == 6 && !(b' '..=b'~').contains(&key) {
            return Err(WireError::Key);
        }
        if matches!(op, 1 | 3) && (w == 0 || h == 0 || w > 320 || h > 200) {
            return Err(WireError::Geometry);
        }
        if let Self::PresentRects { n, rects } = self {
            let n = usize::from(n);
            if n == 0 || n > PRESENT_RECTS || rects[n..].iter().any(|r| *r != [0; 4]) {
                return Err(WireError::Geometry);
            }
            for &[_, _, rw, rh] in &rects[..n] {
                if rw == 0
                    || rh == 0
                    || rw > 1024
                    || rh > 768
                    || u64::from(rw) * u64::from(rh) * 4 > 512 * 4096
                {
                    return Err(WireError::Geometry);
                }
            }
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
            Frame::Poll { handle: 7 },
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
            Frame::Poll { handle: 0 }.encode(&mut b),
            Err(WireError::Handle)
        );
        assert_eq!(b, old);
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
    #[test]
    fn present_rects_roundtrip_and_refusals() {
        let mut rects = [[0u16; 4]; PRESENT_RECTS];
        rects[0] = [10, 20, 30, 40];
        rects[1] = [0, 0, 800, 600];
        let f = Frame::PresentRects { n: 2, rects };
        let mut b = [0u8; BYTES];
        f.encode(&mut b).unwrap();
        assert_eq!(b[5], 10);
        assert_eq!(b[28], 2);
        assert_eq!(&b[32..40], &[10, 0, 20, 0, 30, 0, 40, 0]);
        assert_eq!(Frame::decode(&b), Ok(f));
        // A stray byte in an unused entry, a zero count, an empty or an
        // oversized rectangle: refused whole.
        let mut stray = b;
        stray[63] = 1;
        assert!(Frame::decode(&stray).is_err());
        for bad in [
            Frame::PresentRects { n: 0, rects },
            Frame::PresentRects { n: 5, rects },
            Frame::PresentRects { n: 3, rects },
        ] {
            assert!(bad.encode(&mut b).is_err());
        }
        let mut huge = rects;
        huge[1] = [0, 0, 1024, 768];
        assert!(
            Frame::PresentRects { n: 2, rects: huge }
                .encode(&mut b)
                .is_err()
        );
    }
}
