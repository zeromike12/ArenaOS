//! Neutral desktop client wire. A parsed handle never grants authority.
use crate::model::{
    Event, MIN_HEIGHT, MIN_WIDTH, PopupKind, SURFACE_MAX_HEIGHT as MAX_H,
    SURFACE_MAX_WIDTH as MAX_W, TRANSIENT_MAX_HEIGHT, TRANSIENT_MAX_WIDTH, TRANSIENT_MIN,
};
pub const BYTES: usize = 64;
const MAGIC: &[u8; 4] = b"ADSK";
/// ADSK wire version 1: the existing graphical client protocol (opcodes
/// 1..=12). Preserved verbatim.
pub const ADSK_V1: u8 = 1;
/// ADSK wire version 2: identification (opcode 13). The Identify round-trip
/// uses the same 64-byte frame size and canonical reserved-byte rules.
pub const ADSK_V2: u8 = 2;
/// Identify opcode (ADSK version 2 only).
pub const ADSK_OP_IDENTIFY: u8 = 13;
/// The client protocol version this implementation speaks for Identify.
pub const CLIENT_PROTOCOL_VERSION: u16 = 1;
/// Kind value for a SharedRegion surface in Identify replies.
pub const IDENTIFY_SURFACE_KIND: u8 = 7;
/// Minimum rights the Identify reply names for the session's surface.
pub const IDENTIFY_SURFACE_RIGHTS: u64 = crate::abi::RIGHTS_READ | crate::abi::RIGHTS_WRITE;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Invalid,
}

/// A parsed ADSK version-2 Identify request. All bytes not listed are zero.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IdentifyRequest {
    /// Client protocol version (`u16` LE at bytes 8–9).
    pub client_protocol_version: u16,
}

/// A parsed ADSK version-2 Identify reply. All bytes not listed are zero.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IdentifyReply {
    /// Session serial: descriptive, not authority.
    pub session_serial: u64,
    /// Named surface kind (must be 7 = SharedRegion).
    pub surface_kind: u8,
    /// Named surface rights bitmask (must include READ|WRITE).
    pub surface_rights: u64,
    /// Surface object ID (`u64` LE at bytes 18–25).
    pub surface_object_id: u64,
}

/// Validate the reserved-byte and fixed-field rules for an Identify request
/// frame. Returns `Ok(())` when the frame is well-formed.
fn identify_request_valid(b: &[u8]) -> bool {
    b.len() == BYTES
        && &b[..4] == MAGIC
        && b[4] == ADSK_V2
        && b[5] == ADSK_OP_IDENTIFY
        && b[6] == 0
        && b[7] == 0
        && b[10..64].iter().all(|&x| x == 0)
}

/// Validate the reserved-byte and fixed-field rules for an Identify reply
/// frame. Returns `Ok(())` when the frame is well-formed.
fn identify_reply_valid(b: &[u8]) -> bool {
    b.len() == BYTES
        && &b[..4] == MAGIC
        && b[4] == ADSK_V2
        && b[5] == ADSK_OP_IDENTIFY
        && b[6] == 0
        && b[7] == 0
        && b[26..64].iter().all(|&x| x == 0)
}

/// Encode an Identify request into a 64-byte frame.
pub fn identify_request_encode(version: u16) -> [u8; BYTES] {
    let mut b = [0; BYTES];
    b[..4].copy_from_slice(MAGIC);
    b[4] = ADSK_V2;
    b[5] = ADSK_OP_IDENTIFY;
    b[8..10].copy_from_slice(&version.to_le_bytes());
    b
}

/// Encode an Identify reply into a 64-byte frame.
pub fn identify_reply_encode(
    session_serial: u64,
    surface_kind: u8,
    surface_rights: u64,
    surface_object_id: u64,
) -> [u8; BYTES] {
    let mut b = [0; BYTES];
    b[..4].copy_from_slice(MAGIC);
    b[4] = ADSK_V2;
    b[5] = ADSK_OP_IDENTIFY;
    b[8..16].copy_from_slice(&session_serial.to_le_bytes());
    b[16] = surface_kind;
    b[17] = surface_rights as u8;
    b[18..26].copy_from_slice(&surface_object_id.to_le_bytes());
    b
}

/// Decode an Identify reply from a 64-byte frame. Validates magic, version,
/// opcode, and reserved bytes.
pub fn identify_reply_decode(b: &[u8]) -> Result<IdentifyReply, Error> {
    if !identify_reply_valid(b) {
        return Err(Error::Invalid);
    }
    Ok(IdentifyReply {
        session_serial: u64::from_le_bytes(b[8..16].try_into().map_err(|_| Error::Invalid)?),
        surface_kind: b[16],
        surface_rights: b[17] as u64,
        surface_object_id: u64::from_le_bytes(b[18..26].try_into().map_err(|_| Error::Invalid)?),
    })
}

/// Decode an Identify request from a 64-byte frame. Validates magic, version,
/// opcode, and reserved bytes.
pub fn identify_request_decode(b: &[u8]) -> Result<IdentifyRequest, Error> {
    if !identify_request_valid(b) {
        return Err(Error::Invalid);
    }
    Ok(IdentifyRequest {
        client_protocol_version: u16::from_le_bytes(
            b[8..10].try_into().map_err(|_| Error::Invalid)?,
        ),
    })
}

#[cfg(test)]
mod identify_tests {
    use super::*;

    #[test]
    fn identify_request_round_trip() {
        let b = identify_request_encode(CLIENT_PROTOCOL_VERSION);
        assert_eq!(
            identify_request_decode(&b).unwrap().client_protocol_version,
            CLIENT_PROTOCOL_VERSION
        );
    }

    #[test]
    fn identify_reply_round_trip() {
        let b = identify_reply_encode(
            0x1234_5678_9abc_def0,
            IDENTIFY_SURFACE_KIND,
            IDENTIFY_SURFACE_RIGHTS,
            0x0fed_cba9_8765_4321,
        );
        let r = identify_reply_decode(&b).unwrap();
        assert_eq!(r.session_serial, 0x1234_5678_9abc_def0);
        assert_eq!(r.surface_kind, IDENTIFY_SURFACE_KIND);
        assert_eq!(r.surface_rights, IDENTIFY_SURFACE_RIGHTS);
        assert_eq!(r.surface_object_id, 0x0fed_cba9_8765_4321);
    }

    #[test]
    fn identify_reserved_bytes_refused() {
        let mut b = identify_request_encode(CLIENT_PROTOCOL_VERSION);
        for i in [6, 7, 10, 63] {
            let mut mutant = b;
            mutant[i] = 1;
            assert_eq!(identify_request_decode(&mutant), Err(Error::Invalid));
        }
        assert_eq!(identify_request_decode(&b[..63]), Err(Error::Invalid));
    }

    #[test]
    fn identify_reply_reserved_bytes_refused() {
        let mut b = identify_reply_encode(1, IDENTIFY_SURFACE_KIND, IDENTIFY_SURFACE_RIGHTS, 2);
        for i in [6, 7, 26, 63] {
            let mut mutant = b;
            mutant[i] = 1;
            assert_eq!(identify_reply_decode(&mutant), Err(Error::Invalid));
        }
        assert_eq!(identify_reply_decode(&b[..63]), Err(Error::Invalid));
    }

    #[test]
    fn identify_wrong_magic_refused() {
        let mut b = identify_request_encode(CLIENT_PROTOCOL_VERSION);
        b[0] = b'X';
        assert_eq!(identify_request_decode(&b), Err(Error::Invalid));
    }

    #[test]
    fn identify_wrong_version_refused() {
        let mut b = identify_request_encode(CLIENT_PROTOCOL_VERSION);
        b[4] = ADSK_V1;
        assert_eq!(identify_request_decode(&b), Err(Error::Invalid));
    }

    #[test]
    fn identify_wrong_opcode_refused() {
        let mut b = identify_request_encode(CLIENT_PROTOCOL_VERSION);
        b[5] = 1;
        assert_eq!(identify_request_decode(&b), Err(Error::Invalid));
    }
}
/// Published regions of one Damage request (Phase 11.1). `n == 0` means
/// the whole surface — byte-identical to the Phase-10 Damage frame, so
/// existing clients and the signed fixture keep working unchanged.
/// Rectangles are `[x, y, width, height]` in surface pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DamageRects {
    pub n: u8,
    pub r: [[u16; 4]; DamageRects::MAX],
}
impl DamageRects {
    pub const MAX: usize = 5;
    pub const FULL: Self = Self {
        n: 0,
        r: [[0; 4]; Self::MAX],
    };
    pub fn rects(&self) -> &[[u16; 4]] {
        &self.r[..self.n as usize]
    }
    /// Canonical form: unused slots zero; every used rectangle nonempty and
    /// inside the largest surface the wire admits.
    fn valid(&self) -> bool {
        (self.n as usize) <= Self::MAX
            && self.r[self.n as usize..].iter().all(|r| *r == [0; 4])
            && self.rects().iter().all(|&[x, y, w, h]| {
                w > 0
                    && h > 0
                    && u32::from(x) + u32::from(w) <= u32::from(MAX_W)
                    && u32::from(y) + u32::from(h) <= u32::from(MAX_H)
            })
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Frame {
    Create {
        width: u16,
        height: u16,
    },
    /// Create an additional independently backed ordinary window, including
    /// when this process currently has no primary window.
    CreateAdditional {
        width: u16,
        height: u16,
    },
    Damage {
        handle: u64,
        rects: DamageRects,
    },
    Poll {
        handle: u64,
    },
    CancelClose {
        handle: u64,
    },
    Title {
        handle: u64,
        text: [u8; 32],
    },
    Event {
        handle: u64,
        event: Event,
    },
    /// The client publishes its whole surface at this new size (Phase
    /// 11.3): the broker copies width*height pixels, nothing else.
    Resize {
        handle: u64,
        width: u16,
        height: u16,
    },
    /// The client re-lays out to any size of at least this.
    Resizable {
        handle: u64,
        min_width: u16,
        min_height: u16,
    },
    /// Open the window's transient surface at window-local (x, y).
    Popup {
        handle: u64,
        kind: PopupKind,
        x: i32,
        y: i32,
        width: u16,
        height: u16,
    },
    /// Close the client's own transient surface `handle`.
    Dismiss {
        handle: u64,
    },
    /// Close one ordinary window without retiring its application process.
    DestroyWindow {
        handle: u64,
    },
}
fn surface_size(width: u16, height: u16) -> bool {
    (MIN_WIDTH..=MAX_W).contains(&width) && (MIN_HEIGHT..=MAX_H).contains(&height)
}
impl Frame {
    pub fn encode(self) -> Result<[u8; BYTES], Error> {
        let mut b = [0; BYTES];
        b[..4].copy_from_slice(MAGIC);
        b[4] = 1;
        let (op, handle) = match self {
            Self::Create { width, height } => {
                if !surface_size(width, height) {
                    return Err(Error::Invalid);
                }
                b[24..26].copy_from_slice(&width.to_le_bytes());
                b[26..28].copy_from_slice(&height.to_le_bytes());
                (1, 0)
            }
            Self::CreateAdditional { width, height } => {
                if !surface_size(width, height) {
                    return Err(Error::Invalid);
                }
                b[24..26].copy_from_slice(&width.to_le_bytes());
                b[26..28].copy_from_slice(&height.to_le_bytes());
                (12, 0)
            }
            Self::Damage { handle, rects } => {
                if !rects.valid() {
                    return Err(Error::Invalid);
                }
                b[16] = rects.n;
                for (i, r) in rects.r.iter().enumerate() {
                    for (j, v) in r.iter().enumerate() {
                        let at = 24 + i * 8 + j * 2;
                        b[at..at + 2].copy_from_slice(&v.to_le_bytes());
                    }
                }
                (2, handle)
            }
            Self::Poll { handle } => (3, handle),
            Self::CancelClose { handle } => (6, handle),
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
                        if key > crate::input_wire::MAX_KEY {
                            return Err(Error::Invalid);
                        }
                        b[28] = 1;
                        b[24..26].copy_from_slice(&key.to_le_bytes());
                    }
                    Event::Pointer { x, y, buttons } | Event::PopupPointer { x, y, buttons } => {
                        if !(-i32::from(MAX_W)..=i32::from(MAX_W)).contains(&x)
                            || !(-i32::from(MAX_H)..=i32::from(MAX_H)).contains(&y)
                            || buttons > 7
                        {
                            return Err(Error::Invalid);
                        }
                        b[28] = if matches!(event, Event::Pointer { .. }) {
                            2
                        } else {
                            6
                        };
                        b[29] = buttons;
                        b[16..20].copy_from_slice(&x.to_le_bytes());
                        b[20..24].copy_from_slice(&y.to_le_bytes());
                    }
                    Event::Configure { width, height } => {
                        if !surface_size(width, height) {
                            return Err(Error::Invalid);
                        }
                        b[28] = 5;
                        b[24..26].copy_from_slice(&width.to_le_bytes());
                        b[26..28].copy_from_slice(&height.to_le_bytes());
                    }
                    Event::Chord { code, mods } => {
                        if code > crate::input_wire::MAX_KEY || mods == 0 || mods > 15 {
                            return Err(Error::Invalid);
                        }
                        b[28] = 8;
                        b[29] = mods;
                        b[24..26].copy_from_slice(&code.to_le_bytes());
                    }
                    Event::Wheel { x, y, delta } => {
                        if !(-i32::from(MAX_W)..=i32::from(MAX_W)).contains(&x)
                            || !(-i32::from(MAX_H)..=i32::from(MAX_H)).contains(&y)
                            || delta == 0
                        {
                            return Err(Error::Invalid);
                        }
                        b[28] = 9;
                        b[29] = delta as u8;
                        b[16..20].copy_from_slice(&x.to_le_bytes());
                        b[20..24].copy_from_slice(&y.to_le_bytes());
                    }
                    Event::Dismissed(popup) => {
                        if popup == 0 {
                            return Err(Error::Invalid);
                        }
                        b[28] = 7;
                        b[16..24].copy_from_slice(&popup.to_le_bytes());
                    }
                    Event::Close => {
                        b[28] = 3;
                    }
                    Event::Focus(f) => {
                        b[28] = 4;
                        b[29] = u8::from(f);
                    }
                    Event::Chosen => {
                        b[28] = 10;
                    }
                }
                (5, handle)
            }
            Self::Resize {
                handle,
                width,
                height,
            } => {
                if !surface_size(width, height) {
                    return Err(Error::Invalid);
                }
                b[24..26].copy_from_slice(&width.to_le_bytes());
                b[26..28].copy_from_slice(&height.to_le_bytes());
                (7, handle)
            }
            Self::Resizable {
                handle,
                min_width,
                min_height,
            } => {
                if !surface_size(min_width, min_height) {
                    return Err(Error::Invalid);
                }
                b[24..26].copy_from_slice(&min_width.to_le_bytes());
                b[26..28].copy_from_slice(&min_height.to_le_bytes());
                (8, handle)
            }
            Self::Popup {
                handle,
                kind,
                x,
                y,
                width,
                height,
            } => {
                if !(TRANSIENT_MIN..=TRANSIENT_MAX_WIDTH).contains(&width)
                    || !(TRANSIENT_MIN..=TRANSIENT_MAX_HEIGHT).contains(&height)
                    || !(-i32::from(MAX_W)..=i32::from(MAX_W)).contains(&x)
                    || !(-i32::from(MAX_H)..=i32::from(MAX_H)).contains(&y)
                {
                    return Err(Error::Invalid);
                }
                b[16..20].copy_from_slice(&x.to_le_bytes());
                b[20..24].copy_from_slice(&y.to_le_bytes());
                b[24..26].copy_from_slice(&width.to_le_bytes());
                b[26..28].copy_from_slice(&height.to_le_bytes());
                b[28] = kind as u8;
                (9, handle)
            }
            Self::Dismiss { handle } => (10, handle),
            Self::DestroyWindow { handle } => (11, handle),
        };
        if !matches!(op, 1 | 12) && handle == 0 {
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
            12 => Self::CreateAdditional { width, height },
            2 => {
                let mut rects = DamageRects::FULL;
                rects.n = b[16];
                if rects.n as usize > DamageRects::MAX {
                    return Err(Error::Invalid);
                }
                for (i, r) in rects.r.iter_mut().enumerate() {
                    for (j, v) in r.iter_mut().enumerate() {
                        let at = 24 + i * 8 + j * 2;
                        *v = u16::from_le_bytes([b[at], b[at + 1]]);
                    }
                }
                Self::Damage { handle, rects }
            }
            3 => Self::Poll { handle },
            6 => Self::CancelClose { handle },
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
                    5 => Event::Configure { width, height },
                    6 => Event::PopupPointer {
                        x,
                        y,
                        buttons: b[29],
                    },
                    8 => Event::Chord {
                        code: width,
                        mods: b[29],
                    },
                    9 => Event::Wheel {
                        x,
                        y,
                        delta: b[29] as i8,
                    },
                    7 => Event::Dismissed(u64::from_le_bytes(
                        b[16..24].try_into().map_err(|_| Error::Invalid)?,
                    )),
                    10 => Event::Chosen,
                    _ => return Err(Error::Invalid),
                },
            },
            7 => Self::Resize {
                handle,
                width,
                height,
            },
            8 => Self::Resizable {
                handle,
                min_width: width,
                min_height: height,
            },
            9 => Self::Popup {
                handle,
                kind: PopupKind::from_u8(b[28]).ok_or(Error::Invalid)?,
                x,
                y,
                width,
                height,
            },
            10 => Self::Dismiss { handle },
            11 => Self::DestroyWindow { handle },
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
            Frame::CreateAdditional {
                width: 320,
                height: 180,
            },
            Frame::Damage {
                handle: 9,
                rects: DamageRects::FULL,
            },
            Frame::Poll { handle: u64::MAX },
            Frame::CancelClose { handle: 9 },
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
            Frame::Event {
                handle: 9,
                event: Event::Configure {
                    width: 800,
                    height: 518,
                },
            },
            Frame::Event {
                handle: 9,
                event: Event::PopupPointer {
                    x: 3,
                    y: -4,
                    buttons: 1,
                },
            },
            Frame::Event {
                handle: 9,
                event: Event::Dismissed(77),
            },
            Frame::Event {
                handle: 9,
                event: Event::Chord {
                    code: u16::from(b's'),
                    mods: 2,
                },
            },
            Frame::Event {
                handle: 9,
                event: Event::Wheel {
                    x: 10,
                    y: 20,
                    delta: -3,
                },
            },
            Frame::Resize {
                handle: 9,
                width: 1024,
                height: 768,
            },
            Frame::Resizable {
                handle: 9,
                min_width: 80,
                min_height: 60,
            },
            Frame::Popup {
                handle: 9,
                kind: PopupKind::Menu,
                x: -20,
                y: 300,
                width: 160,
                height: 200,
            },
            Frame::Dismiss { handle: 12 },
            Frame::DestroyWindow { handle: 12 },
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
                width: 1025,
                height: 288
            }
            .encode()
            .is_err()
        );
        assert!(
            Frame::Event {
                handle: 9,
                event: Event::Key(crate::input_wire::MAX_KEY + 1)
            }
            .encode()
            .is_err()
        );
    }
    #[test]
    fn damage_rectangles_are_bounded_and_canonical() {
        let mut rects = DamageRects::FULL;
        rects.n = 2;
        rects.r[0] = [0, 240, 448, 24];
        rects.r[1] = [10, 70, 5, 14];
        let f = Frame::Damage { handle: 7, rects };
        let b = f.encode().unwrap();
        assert_eq!(Frame::decode(&b), Ok(f));
        // The Phase-10 byte layout (header + handle only) is full damage.
        let mut old = [0u8; BYTES];
        old[..4].copy_from_slice(MAGIC);
        old[4] = 1;
        old[5] = 2;
        old[8] = 7;
        assert_eq!(
            Frame::decode(&old),
            Ok(Frame::Damage {
                handle: 7,
                rects: DamageRects::FULL
            })
        );
        let refuse = |r: DamageRects| {
            Frame::Damage {
                handle: 7,
                rects: r,
            }
            .encode()
            .is_err()
        };
        let mut over = DamageRects::FULL;
        over.n = 6;
        assert!(refuse(over));
        let mut empty = rects;
        empty.r[1] = [10, 70, 0, 14];
        assert!(refuse(empty));
        let mut outside = rects;
        outside.r[0] = [1, 240, 1024, 24];
        assert!(refuse(outside));
        let mut stray = rects;
        stray.r[3] = [1, 1, 1, 1];
        assert!(refuse(stray));
        // A count above the bound on the wire is refused at decode.
        let mut wire = b;
        wire[16] = 9;
        assert_eq!(Frame::decode(&wire), Err(Error::Invalid));
    }
    #[test]
    fn surface_and_transient_bounds_refuse() {
        for (w, h) in [(79, 60), (80, 59), (1025, 60), (80, 769)] {
            assert!(
                Frame::Resize {
                    handle: 1,
                    width: w,
                    height: h
                }
                .encode()
                .is_err()
            );
            assert!(
                Frame::Event {
                    handle: 1,
                    event: Event::Configure {
                        width: w,
                        height: h
                    }
                }
                .encode()
                .is_err()
            );
        }
        for (w, h) in [(7, 8), (8, 7), (513, 8), (8, 385)] {
            assert!(
                Frame::Popup {
                    handle: 1,
                    kind: PopupKind::Dialog,
                    x: 0,
                    y: 0,
                    width: w,
                    height: h
                }
                .encode()
                .is_err()
            );
        }
        let mut b = Frame::Popup {
            handle: 1,
            kind: PopupKind::Tooltip,
            x: 0,
            y: 0,
            width: 8,
            height: 8,
        }
        .encode()
        .unwrap();
        b[28] = 4;
        assert_eq!(Frame::decode(&b), Err(Error::Invalid));
        assert!(Frame::Dismiss { handle: 0 }.encode().is_err());
        assert!(
            Frame::Event {
                handle: 1,
                event: Event::Dismissed(0)
            }
            .encode()
            .is_err()
        );
    }
}
