//! Application models contain no colors, pixel geometry, IPC or privilege.
pub const TEXT_MAX: usize = 4096;
pub const LINE_MAX: usize = 64;
pub const SCROLL_LINES: usize = 32;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Full,
    Invalid,
}
pub struct Editor {
    pub data: [u8; TEXT_MAX],
    pub len: usize,
    pub cursor: usize,
    pub dirty: bool,
    pub path: [u8; 32],
}
impl Default for Editor {
    fn default() -> Self {
        Self::new()
    }
}
impl Editor {
    pub const fn new() -> Self {
        Self {
            data: [0; TEXT_MAX],
            len: 0,
            cursor: 0,
            dirty: false,
            path: [0; 32],
        }
    }
    pub fn load(&mut self, data: &[u8], path: &str) -> Result<(), Error> {
        if data.len() > TEXT_MAX
            || !data
                .iter()
                .all(|b| b.is_ascii_graphic() || matches!(b, b' ' | b'\n' | b'\t'))
            || path.len() > 31
            || !path.is_ascii()
        {
            return Err(Error::Invalid);
        }
        self.data.fill(0);
        self.data[..data.len()].copy_from_slice(data);
        self.len = data.len();
        self.cursor = 0;
        self.dirty = false;
        self.path.fill(0);
        self.path[..path.len()].copy_from_slice(path.as_bytes());
        Ok(())
    }
    pub fn insert(&mut self, b: u8) -> Result<(), Error> {
        if !b.is_ascii_graphic() && !matches!(b, b' ' | b'\n' | b'\t') {
            return Err(Error::Invalid);
        }
        if self.len == TEXT_MAX {
            return Err(Error::Full);
        }
        self.data
            .copy_within(self.cursor..self.len, self.cursor + 1);
        self.data[self.cursor] = b;
        self.len += 1;
        self.cursor += 1;
        self.dirty = true;
        Ok(())
    }
    pub fn backspace(&mut self) {
        if self.cursor != 0 {
            self.cursor -= 1;
            self.delete();
        }
    }
    pub fn delete(&mut self) {
        if self.cursor < self.len {
            self.data
                .copy_within(self.cursor + 1..self.len, self.cursor);
            self.len -= 1;
            self.data[self.len] = 0;
            self.dirty = true;
        }
    }
    pub fn left(&mut self) {
        self.cursor = self.cursor.saturating_sub(1)
    }
    pub fn right(&mut self) {
        self.cursor = (self.cursor + 1).min(self.len)
    }
    pub fn home(&mut self) {
        self.cursor = self.data[..self.cursor]
            .iter()
            .rposition(|b| *b == b'\n')
            .map(|i| i + 1)
            .unwrap_or(0)
    }
    pub fn end(&mut self) {
        self.cursor = self.data[self.cursor..self.len]
            .iter()
            .position(|b| *b == b'\n')
            .map(|i| self.cursor + i)
            .unwrap_or(self.len)
    }
    pub fn vertical(&mut self, down: bool) {
        let at = self.cursor;
        self.home();
        let start = self.cursor;
        let column = at - start;
        if down {
            self.cursor = at;
            self.end();
            if self.cursor == self.len {
                return;
            }
            self.cursor += 1;
        } else {
            if start == 0 {
                self.cursor = at;
                return;
            }
            self.cursor = start - 1;
            self.home();
        }
        let target = self.cursor;
        self.end();
        self.cursor = (target + column).min(self.cursor);
    }
}
pub struct Terminal {
    pub lines: [[u8; LINE_MAX]; SCROLL_LINES],
    pub sizes: [usize; SCROLL_LINES],
    pub count: usize,
    pub input: [u8; LINE_MAX],
    pub len: usize,
    pub cursor: usize,
    pub dropped: usize,
}
impl Default for Terminal {
    fn default() -> Self {
        Self::new()
    }
}
impl Terminal {
    pub const fn new() -> Self {
        Self {
            lines: [[0; LINE_MAX]; SCROLL_LINES],
            sizes: [0; SCROLL_LINES],
            count: 0,
            input: [0; LINE_MAX],
            len: 0,
            cursor: 0,
            dropped: 0,
        }
    }
    pub fn write(&mut self, bytes: &[u8]) {
        for line in bytes.split(|b| *b == b'\n') {
            if line.is_empty() {
                self.line(&[]);
            } else {
                for chunk in line.chunks(LINE_MAX) {
                    self.line(chunk);
                }
            }
        }
    }
    fn line(&mut self, data: &[u8]) {
        if self.count == SCROLL_LINES {
            self.lines.copy_within(1..SCROLL_LINES, 0);
            self.sizes.copy_within(1..SCROLL_LINES, 0);
            self.count -= 1;
            self.dropped = self.dropped.saturating_add(1);
        }
        self.lines[self.count].fill(0);
        self.lines[self.count][..data.len()].copy_from_slice(data);
        self.sizes[self.count] = data.len();
        self.count += 1;
    }
    pub fn key(&mut self, key: u16) -> bool {
        match key {
            8 if self.cursor > 0 => {
                self.cursor -= 1;
                self.input
                    .copy_within(self.cursor + 1..self.len, self.cursor);
                self.len -= 1;
                self.input[self.len] = 0;
            }
            256 => self.cursor = self.cursor.saturating_sub(1),
            257 => self.cursor = (self.cursor + 1).min(self.len),
            260 => self.cursor = 0,
            261 => self.cursor = self.len,
            262 if self.cursor < self.len => {
                self.input
                    .copy_within(self.cursor + 1..self.len, self.cursor);
                self.len -= 1;
                self.input[self.len] = 0;
            }
            13 => return true,
            32..=126 if self.len < LINE_MAX => {
                self.input
                    .copy_within(self.cursor..self.len, self.cursor + 1);
                self.input[self.cursor] = key as u8;
                self.len += 1;
                self.cursor += 1;
            }
            _ => {}
        }
        false
    }
    pub fn consume(&mut self) -> ([u8; LINE_MAX], usize) {
        let out = self.input;
        let len = self.len;
        self.input.fill(0);
        self.len = 0;
        self.cursor = 0;
        (out, len)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn editor_insert_delete_navigation_and_atomic_load_refusal() {
        let mut e = Editor::new();
        e.load(b"abc\nx\n12345", "user-note").unwrap();
        e.cursor = 2;
        e.vertical(true);
        assert_eq!(e.cursor, 5);
        e.vertical(true);
        assert_eq!(e.cursor, 7);
        e.home();
        e.insert(b'!').unwrap();
        assert_eq!(&e.data[..e.len], b"abc\nx\n!12345");
        e.backspace();
        assert_eq!(&e.data[..e.len], b"abc\nx\n12345");
        e.cursor = 1;
        e.delete();
        assert_eq!(&e.data[..e.len], b"ac\nx\n12345");
        let before = e.data;
        assert_eq!(e.load(&[0xff], "bad"), Err(Error::Invalid));
        assert_eq!(e.data, before);
    }
    #[test]
    fn editor_full_refuses_without_cursor_or_data_mutation() {
        let mut e = Editor::new();
        e.load(&[b'x'; TEXT_MAX], "user-full").unwrap();
        e.cursor = 300;
        let before = e.data;
        assert_eq!(e.insert(b'y'), Err(Error::Full));
        assert_eq!(e.cursor, 300);
        assert_eq!(e.data, before);
        assert!(!e.dirty);
    }
    #[test]
    fn terminal_line_edit_and_bounded_scrollback() {
        let mut t = Terminal::new();
        for b in b"echo ac" {
            assert!(!t.key(u16::from(*b)));
        }
        t.key(256);
        t.key(b'b' as u16);
        assert!(t.key(13));
        let (s, n) = t.consume();
        assert_eq!(&s[..n], b"echo abc");
        for _ in 0..40 {
            t.write(b"line");
        }
        assert_eq!(t.count, 32);
        assert_eq!(t.dropped, 8);
        assert_eq!(&t.lines[31][..t.sizes[31]], b"line");
    }
}

/// Reusable bounded single-line input used by file/save dialogs.
pub struct Line {
    pub bytes: [u8; 32],
    pub len: usize,
    pub cursor: usize,
}
impl Default for Line {
    fn default() -> Self {
        Self::new()
    }
}
impl Line {
    pub const fn new() -> Self {
        Self {
            bytes: [0; 32],
            len: 0,
            cursor: 0,
        }
    }
    pub fn set(&mut self, value: &[u8]) {
        self.bytes.fill(0);
        self.len = value.len().min(31);
        self.bytes[..self.len].copy_from_slice(&value[..self.len]);
        self.cursor = self.len;
    }
    pub fn key(&mut self, key: u16) -> bool {
        match key {
            13 => return true,
            8 if self.cursor > 0 => {
                self.cursor -= 1;
                self.bytes
                    .copy_within(self.cursor + 1..self.len, self.cursor);
                self.len -= 1;
                self.bytes[self.len] = 0;
            }
            256 => self.cursor = self.cursor.saturating_sub(1),
            257 => self.cursor = (self.cursor + 1).min(self.len),
            260 => self.cursor = 0,
            261 => self.cursor = self.len,
            262 if self.cursor < self.len => {
                self.bytes
                    .copy_within(self.cursor + 1..self.len, self.cursor);
                self.len -= 1;
                self.bytes[self.len] = 0;
            }
            32..=126 if self.len < 31 => {
                self.bytes
                    .copy_within(self.cursor..self.len, self.cursor + 1);
                self.bytes[self.cursor] = key as u8;
                self.len += 1;
                self.cursor += 1;
            }
            _ => {}
        }
        false
    }
}
