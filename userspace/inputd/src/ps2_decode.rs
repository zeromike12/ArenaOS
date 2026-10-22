//! Pure PS/2 Set-1 keyboard and three-byte mouse packet decoders.
//! Device polling and ConsoleInput authority remain in `inputd`.

pub const KEY_PRESS: u32 = 1;
pub const KEY_RELEASE: u32 = 0;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Keyboard {
    extended: bool,
    pause_bytes_remaining: u8,
}

impl Keyboard {
    pub const fn new() -> Self {
        Self {
            extended: false,
            pause_bytes_remaining: 0,
        }
    }

    pub fn byte(&mut self, byte: u8) -> Option<(u16, u32)> {
        if self.pause_bytes_remaining != 0 {
            self.pause_bytes_remaining -= 1;
            return None;
        }
        if byte == 0xE1 {
            // Set-1 Pause is E1 1D 45 E1 9D C5. It has no break code;
            // discard the five continuation bytes as one unsupported key.
            self.extended = false;
            self.pause_bytes_remaining = 5;
            return None;
        }
        if byte == 0xE0 {
            self.extended = true;
            return None;
        }
        let extended = core::mem::replace(&mut self.extended, false);
        let released = byte & 0x80 != 0;
        let scan = byte & 0x7F;
        let code = if extended {
            match scan {
                0x1C => 96,  // keypad enter
                0x1D => 97,  // right control
                0x38 => 100, // right alt
                0x47 => 102, // home
                0x48 => 103, // up
                0x4B => 105, // left
                0x4D => 106, // right
                0x4F => 107, // end
                0x50 => 108, // down
                0x52 => 110, // insert
                0x53 => 111, // delete
                0x5B => 125, // left super
                0x5C => 126, // right super
                _ => return None,
            }
        } else {
            match scan {
                0x01 => 1,
                0x02..=0x0B => u16::from(scan - 0x02 + 2),
                0x0C => 12,
                0x0D => 13,
                0x0E => 14,
                0x0F => 15,
                0x10..=0x19 => u16::from(scan - 0x10 + 16),
                0x1A => 26,
                0x1B => 27,
                0x1C => 28,
                0x1D => 29,
                0x1E..=0x26 => u16::from(scan - 0x1E + 30),
                0x27 => 39,
                0x28 => 40,
                0x29 => 41,
                0x2A => 42,
                0x2B => 43,
                0x2C..=0x32 => u16::from(scan - 0x2C + 44),
                0x33 => 51,
                0x34 => 52,
                0x35 => 53,
                0x36 => 54,
                0x37 => 55,
                0x38 => 56,
                0x39 => 57,
                0x3B..=0x44 => u16::from(scan - 0x3B + 59),
                0x57 => 87,
                0x58 => 88,
                _ => return None,
            }
        };
        Some((code, if released { KEY_RELEASE } else { KEY_PRESS }))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MouseEvent {
    pub dx: i32,
    pub dy: i32,
    pub x: i32,
    pub y: i32,
    pub buttons: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Mouse {
    packet: [u8; 3],
    used: usize,
    x: i32,
    y: i32,
}

impl Mouse {
    pub const fn new() -> Self {
        Self {
            packet: [0; 3],
            used: 0,
            x: 16_384,
            y: 16_384,
        }
    }

    pub fn byte(&mut self, value: u8) -> Option<MouseEvent> {
        if self.used == 0 && value & 0x08 == 0 {
            return None;
        }
        self.packet[self.used] = value;
        self.used += 1;
        if self.used != self.packet.len() {
            return None;
        }
        self.used = 0;
        let header = self.packet[0];
        let dx = if header & 0x40 == 0 {
            self.packet[1] as i8 as i32
        } else {
            0
        };
        let dy = if header & 0x80 == 0 {
            self.packet[2] as i8 as i32
        } else {
            0
        };
        self.x = (self.x + dx * 40).clamp(0, 32_767);
        self.y = (self.y - dy * 40).clamp(0, 32_767);
        Some(MouseEvent {
            dx,
            dy,
            x: self.x,
            y: self.y,
            buttons: header & 0x07,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_ascii_navigation_and_release_scancodes() {
        let mut keyboard = Keyboard::new();
        assert_eq!(keyboard.byte(0x1E), Some((30, KEY_PRESS))); // A
        assert_eq!(keyboard.byte(0x9E), Some((30, KEY_RELEASE)));
        assert_eq!(keyboard.byte(0xE0), None);
        assert_eq!(keyboard.byte(0x48), Some((103, KEY_PRESS))); // Up
        assert_eq!(keyboard.byte(0xE0), None);
        assert_eq!(keyboard.byte(0xC8), Some((103, KEY_RELEASE)));
    }

    #[test]
    fn pause_sequence_is_discarded_without_corrupting_next_key() {
        let mut keyboard = Keyboard::new();
        for byte in [0xE1, 0x1D, 0x45, 0xE1, 0x9D, 0xC5] {
            assert_eq!(keyboard.byte(byte), None);
        }
        assert_eq!(keyboard.byte(0x30), Some((48, KEY_PRESS))); // B
    }

    #[test]
    fn mouse_requires_sync_byte_and_tracks_signed_relative_motion() {
        let mut mouse = Mouse::new();
        assert_eq!(mouse.byte(0x00), None);
        assert_eq!(mouse.byte(0x09), None); // left button, packet start
        assert_eq!(mouse.byte(0xFE), None); // dx = -2
        let event = mouse.byte(0x03).unwrap(); // dy = 3
        assert_eq!((event.dx, event.dy, event.buttons), (-2, 3, 1));
        assert_eq!((event.x, event.y), (16_304, 16_264));
    }
}
