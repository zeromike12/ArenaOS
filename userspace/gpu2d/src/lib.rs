//! Strict, allocation-free virtio-gpu 2D command/response codec.
//!
//! OASIS virtio v1.3 §5.7, plain little-endian wire bytes (never native
//! struct casts). This library has no MMIO, DMA, queue, cap or driver-ready
//! authority. It is not a GPU service or evidence of actual guest pixels.
#![no_std]

pub mod device;
pub mod queue;

pub const HEADER: usize = 24;
pub const MAX_REQUEST: usize = 56;
pub const DISPLAY_MODES: usize = 16;
pub const DISPLAY_REPLY: usize = HEADER + DISPLAY_MODES * 24;
pub const MAX_SCANOUT_BYTES: u32 = 512 * 4096;
pub const FORMAT_B8G8R8X8_UNORM: u32 = 2;
pub const GET_DISPLAY_INFO: u32 = 0x0100;
pub const CREATE_2D: u32 = 0x0101;
pub const UNREF: u32 = 0x0102;
pub const SET_SCANOUT: u32 = 0x0103;
pub const FLUSH: u32 = 0x0104;
pub const TRANSFER: u32 = 0x0105;
pub const ATTACH: u32 = 0x0106;
pub const DETACH: u32 = 0x0107;
pub const OK_NODATA: u32 = 0x1100;
pub const OK_DISPLAY_INFO: u32 = 0x1101;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WireError {
    InvalidGeometry,
    InvalidBacking,
    InvalidResource,
    WrongResponse,
    MalformedResponse,
    NoScanout,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl Rect {
    pub fn within(self, width: u32, height: u32) -> bool {
        self.width != 0
            && self.height != 0
            && self.x.checked_add(self.width).is_some_and(|x| x <= width)
            && self.y.checked_add(self.height).is_some_and(|y| y <= height)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Mode {
    pub scanout: u32,
    pub width: u32,
    pub height: u32,
}
impl Mode {
    pub fn new(scanout: u32, width: u32, height: u32) -> Result<Self, WireError> {
        let pixels = width.checked_mul(height).and_then(|n| n.checked_mul(4));
        if scanout >= DISPLAY_MODES as u32
            || width == 0
            || height == 0
            || width > 1024
            || height > 768
            || pixels.is_none_or(|n| n > MAX_SCANOUT_BYTES)
        {
            return Err(WireError::InvalidGeometry);
        }
        Ok(Self {
            scanout,
            width,
            height,
        })
    }
    pub const fn full_rect(self) -> Rect {
        Rect {
            x: 0,
            y: 0,
            width: self.width,
            height: self.height,
        }
    }
    pub fn span(self) -> u32 {
        self.width * self.height * 4
    }
}

pub enum Command {
    DisplayInfo,
    Create {
        mode: Mode,
        resource: u32,
    },
    Attach {
        mode: Mode,
        resource: u32,
        physical: u64,
        bytes: u32,
    },
    SetScanout {
        mode: Mode,
        resource: u32,
    },
    Transfer {
        mode: Mode,
        rect: Rect,
        offset: u64,
        resource: u32,
    },
    Flush {
        mode: Mode,
        rect: Rect,
        resource: u32,
    },
    Detach {
        resource: u32,
    },
    Unref {
        resource: u32,
    },
    ClearScanout {
        mode: Mode,
    },
}

fn put32(out: &mut [u8], at: usize, value: u32) {
    out[at..at + 4].copy_from_slice(&value.to_le_bytes());
}
fn put64(out: &mut [u8], at: usize, value: u64) {
    out[at..at + 8].copy_from_slice(&value.to_le_bytes());
}
fn get32(input: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(input[at..at + 4].try_into().unwrap())
}
#[cfg(test)]
fn get64(input: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(input[at..at + 8].try_into().unwrap())
}
fn put_rect(out: &mut [u8; MAX_REQUEST], at: usize, rect: Rect) {
    for (i, word) in [rect.x, rect.y, rect.width, rect.height].iter().enumerate() {
        put32(out, at + 4 * i, *word);
    }
}

impl Command {
    /// Refuse malformed layout *before* publishing any bytes. Caller must
    /// validate the held DMA cap/backing object outside this pure codec.
    pub fn encode(&self, out: &mut [u8; MAX_REQUEST]) -> Result<usize, WireError> {
        let mut b = [0u8; MAX_REQUEST];
        let (kind, len) = match *self {
            Command::DisplayInfo => (GET_DISPLAY_INFO, HEADER),
            Command::Create { mode, resource } => {
                validate_mode(mode)?;
                validate_resource(resource)?;
                put32(&mut b, 24, resource);
                put32(&mut b, 28, FORMAT_B8G8R8X8_UNORM);
                put32(&mut b, 32, mode.width);
                put32(&mut b, 36, mode.height);
                (CREATE_2D, 40)
            }
            Command::Attach {
                mode,
                resource,
                physical,
                bytes,
            } => {
                validate_mode(mode)?;
                validate_resource(resource)?;
                if physical == 0
                    || physical % 4096 != 0
                    || bytes < mode.span()
                    || bytes > MAX_SCANOUT_BYTES
                    || bytes % 4096 != 0
                    || physical.checked_add(u64::from(bytes)).is_none()
                {
                    return Err(WireError::InvalidBacking);
                }
                put32(&mut b, 24, resource);
                put32(&mut b, 28, 1); // one bounded memory entry
                put64(&mut b, 32, physical);
                put32(&mut b, 40, bytes);
                (ATTACH, 48)
            }
            Command::SetScanout { mode, resource } => {
                validate_mode(mode)?;
                validate_resource(resource)?;
                put_rect(&mut b, 24, mode.full_rect());
                put32(&mut b, 40, mode.scanout);
                put32(&mut b, 44, resource);
                (SET_SCANOUT, 48)
            }
            Command::ClearScanout { mode } => {
                validate_mode(mode)?;
                put32(&mut b, 40, mode.scanout);
                (SET_SCANOUT, 48)
            }
            Command::Transfer {
                mode,
                rect,
                offset,
                resource,
            } => {
                validate_mode(mode)?;
                validate_resource(resource)?;
                let expected =
                    u64::from(rect.y) * u64::from(mode.width) * 4 + u64::from(rect.x) * 4;
                if !rect.within(mode.width, mode.height) || offset != expected {
                    return Err(WireError::InvalidGeometry);
                }
                put_rect(&mut b, 24, rect);
                put64(&mut b, 40, offset);
                put32(&mut b, 48, resource);
                (TRANSFER, 56)
            }
            Command::Flush {
                mode,
                rect,
                resource,
            } => {
                validate_mode(mode)?;
                validate_resource(resource)?;
                if !rect.within(mode.width, mode.height) {
                    return Err(WireError::InvalidGeometry);
                }
                put_rect(&mut b, 24, rect);
                put32(&mut b, 40, resource);
                (FLUSH, 48)
            }
            Command::Detach { resource } | Command::Unref { resource } => {
                validate_resource(resource)?;
                put32(&mut b, 24, resource);
                (
                    if matches!(self, Command::Detach { .. }) {
                        DETACH
                    } else {
                        UNREF
                    },
                    32,
                )
            }
        };
        put32(&mut b, 0, kind);
        *out = b;
        Ok(len)
    }
}
fn validate_resource(id: u32) -> Result<(), WireError> {
    if id == 0 {
        Err(WireError::InvalidResource)
    } else {
        Ok(())
    }
}
fn validate_mode(mode: Mode) -> Result<(), WireError> {
    if Mode::new(mode.scanout, mode.width, mode.height).is_err() {
        Err(WireError::InvalidGeometry)
    } else {
        Ok(())
    }
}

/// Device writes `used_len` bytes into a DMA response buffer; the driver
/// must validate length and opcode before accepting/acknowledging the entry.
/// No speculative read of a short response is permitted.
pub fn response_no_data(input: &[u8], used_len: usize) -> Result<(), WireError> {
    response_header(input, used_len, HEADER, OK_NODATA)
}
fn response_header(
    input: &[u8],
    used_len: usize,
    expected_len: usize,
    expected_kind: u32,
) -> Result<(), WireError> {
    if used_len != expected_len || input.len() < used_len || used_len < HEADER {
        return Err(WireError::MalformedResponse);
    }
    if get32(input, 0) != expected_kind {
        return Err(WireError::WrongResponse);
    }
    // No fence or context was negotiated; unknown nonzero fields fail closed.
    if input[4..HEADER].iter().any(|&x| x != 0) {
        return Err(WireError::MalformedResponse);
    }
    Ok(())
}

pub fn response_display_info(input: &[u8], used_len: usize) -> Result<Mode, WireError> {
    response_header(input, used_len, DISPLAY_REPLY, OK_DISPLAY_INFO)?;
    let mut mode = None;
    for scanout in 0..DISPLAY_MODES {
        let start = HEADER + scanout * 24;
        let enabled = get32(input, start + 16);
        let flags = get32(input, start + 20);
        if enabled > 1 || flags != 0 {
            return Err(WireError::MalformedResponse);
        }
        if enabled == 1 {
            if mode.is_some() || get32(input, start) != 0 || get32(input, start + 4) != 0 {
                return Err(WireError::MalformedResponse);
            }
            mode = Some(Mode::new(
                scanout as u32,
                get32(input, start + 8),
                get32(input, start + 12),
            )?);
        }
    }
    mode.ok_or(WireError::NoScanout)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn command_layouts_are_exact_and_refuse_before_touching_output() {
        let m = Mode::new(0, 800, 600).unwrap();
        let mut b = [0xa5; MAX_REQUEST];
        assert_eq!(
            Command::Attach {
                mode: m,
                resource: 1,
                physical: 0x120000,
                bytes: 469 * 4096
            }
            .encode(&mut b),
            Ok(48)
        );
        assert_eq!(get32(&b, 0), ATTACH);
        assert_eq!(get32(&b, 24), 1);
        assert_eq!(get32(&b, 28), 1);
        assert_eq!(get64(&b, 32), 0x120000);
        assert_eq!(get32(&b, 40), 469 * 4096);
        assert_eq!(&b[44..], &[0; 12]);
        let old = b;
        assert_eq!(
            Command::Attach {
                mode: m,
                resource: 1,
                physical: 1,
                bytes: 469 * 4096
            }
            .encode(&mut b),
            Err(WireError::InvalidBacking)
        );
        assert_eq!(b, old);
        assert_eq!(
            Command::Create {
                mode: m,
                resource: 1
            }
            .encode(&mut b),
            Ok(40)
        );
        assert_eq!(get32(&b, 28), 2);
        assert_eq!(get32(&b, 32), 800);
        assert_eq!(get32(&b, 36), 600);
        assert_eq!(
            Command::Transfer {
                mode: m,
                rect: m.full_rect(),
                offset: 0,
                resource: 1
            }
            .encode(&mut b),
            Ok(56)
        );
        assert_eq!(get32(&b, 48), 1);
        assert_eq!(
            Command::Flush {
                mode: m,
                rect: m.full_rect(),
                resource: 1
            }
            .encode(&mut b),
            Ok(48)
        );
        assert_eq!(get32(&b, 40), 1);
        assert_eq!(Command::Detach { resource: 1 }.encode(&mut b), Ok(32));
        assert_eq!(Command::Unref { resource: 1 }.encode(&mut b), Ok(32));
    }
    #[test]
    fn hostile_geometry_and_stale_response_lengths_refuse() {
        assert_eq!(Mode::new(0, 1024, 768), Err(WireError::InvalidGeometry));
        assert_eq!(Mode::new(16, 800, 600), Err(WireError::InvalidGeometry));
        let m = Mode::new(0, 800, 600).unwrap();
        let mut b = [0u8; MAX_REQUEST];
        let off = Rect {
            x: u32::MAX,
            y: 0,
            width: 1,
            height: 1,
        };
        assert_eq!(
            Command::Flush {
                mode: m,
                rect: off,
                resource: 1
            }
            .encode(&mut b),
            Err(WireError::InvalidGeometry)
        );
        assert_eq!(
            Command::Create {
                mode: m,
                resource: 0
            }
            .encode(&mut b),
            Err(WireError::InvalidResource)
        );
        let mut ok = [0u8; HEADER];
        put32(&mut ok, 0, OK_NODATA);
        assert_eq!(response_no_data(&ok, HEADER), Ok(()));
        assert_eq!(
            response_no_data(&ok, HEADER - 1),
            Err(WireError::MalformedResponse)
        );
        ok[4] = 1;
        assert_eq!(
            response_no_data(&ok, HEADER),
            Err(WireError::MalformedResponse)
        );
        ok[4] = 0;
        put32(&mut ok, 0, 0x1200);
        assert_eq!(response_no_data(&ok, HEADER), Err(WireError::WrongResponse));
    }
    #[test]
    fn short_hostile_response_fuzz_never_reads_past_used_length() {
        let mut bytes = [0u8; DISPLAY_REPLY];
        let mut state = 0x9e37_79b9_7f4a_7c15u64;
        for length in 0..=DISPLAY_REPLY {
            for b in &mut bytes {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                *b = state as u8;
            }
            let _ = response_display_info(&bytes[..length], length);
            let _ = response_no_data(&bytes[..length], length);
            if length < DISPLAY_REPLY {
                assert_eq!(
                    response_display_info(&bytes, length),
                    Err(WireError::MalformedResponse)
                );
            }
            if length != HEADER {
                assert_eq!(
                    response_no_data(&bytes, length),
                    Err(WireError::MalformedResponse)
                );
            }
        }
    }

    #[test]
    fn display_response_only_one_checked_mode() {
        let mut b = [0u8; DISPLAY_REPLY];
        put32(&mut b, 0, OK_DISPLAY_INFO);
        put32(&mut b, HEADER + 8, 800);
        put32(&mut b, HEADER + 12, 600);
        put32(&mut b, HEADER + 16, 1);
        assert_eq!(
            response_display_info(&b, DISPLAY_REPLY),
            Mode::new(0, 800, 600)
        );
        assert_eq!(
            response_display_info(&b, DISPLAY_REPLY - 1),
            Err(WireError::MalformedResponse)
        );
        put32(&mut b, HEADER + 24 + 16, 1);
        assert_eq!(
            response_display_info(&b, DISPLAY_REPLY),
            Err(WireError::MalformedResponse)
        );
        put32(&mut b, HEADER + 24 + 16, 0);
        put32(&mut b, HEADER + 8, u32::MAX);
        assert_eq!(
            response_display_info(&b, DISPLAY_REPLY),
            Err(WireError::InvalidGeometry)
        );
    }
}
