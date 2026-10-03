//! Phase-9 reusable, standalone no_std bitmap toolkit.
//!
//! All graphics operations use caller-owned slices and checked geometry;
//! neither a framebuffer address nor a kernel/device cap exists here. An
//! 800x600 scanout and a 320x200 client surface share the same routines.
//! `Canvas` cannot index outside its borrowed backing even for malicious
//! rectangles, negative coordinates, overflow-sized dimensions or text.
#![no_std]

pub const MAX_WIDTH: usize = 1024;
pub const MAX_HEIGHT: usize = 768;
pub const MAX_TEXT_BYTES: usize = 96;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DrawError {
    InvalidGeometry,
    InvalidText,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

/// Opaque XRGB pixels; `stride` is in 32-bit pixels, not bytes.
/// A canvas borrows the complete span so there can be no alias with a
/// second mutable renderer in the same process.
pub struct Canvas<'a> {
    pixels: &'a mut [u32],
    width: usize,
    height: usize,
    stride: usize,
    /// Half-open pixel clip `[x0, x1) x [y0, y1)`, always inside the canvas.
    /// Every drawing operation is intersected with it; it defaults to the
    /// whole canvas, so callers that never set it see unchanged behaviour.
    clip: (usize, usize, usize, usize),
}

impl<'a> Canvas<'a> {
    pub fn new(
        pixels: &'a mut [u32],
        width: usize,
        height: usize,
        stride: usize,
    ) -> Result<Self, DrawError> {
        if width == 0
            || height == 0
            || width > MAX_WIDTH
            || height > MAX_HEIGHT
            || stride < width
            || stride > MAX_WIDTH
            || stride
                .checked_mul(height)
                .is_none_or(|size| size > pixels.len())
        {
            return Err(DrawError::InvalidGeometry);
        }
        Ok(Self {
            pixels,
            width,
            height,
            stride,
            clip: (0, 0, width, height),
        })
    }

    /// Restrict all later drawing to `rect` intersected with the canvas.
    /// Damage-driven compositors use this to recompose only changed areas.
    pub fn set_clip(&mut self, rect: Rect) {
        let x0 = i64::from(rect.x).clamp(0, self.width as i64) as usize;
        let y0 = i64::from(rect.y).clamp(0, self.height as i64) as usize;
        let x1 = (i64::from(rect.x) + i64::from(rect.width)).clamp(0, self.width as i64) as usize;
        let y1 = (i64::from(rect.y) + i64::from(rect.height)).clamp(0, self.height as i64) as usize;
        self.clip = (x0, y0, x1.max(x0), y1.max(y0));
    }

    pub fn reset_clip(&mut self) {
        self.clip = (0, 0, self.width, self.height);
    }

    pub fn clip(&self) -> Rect {
        let (x0, y0, x1, y1) = self.clip;
        Rect {
            x: x0 as i32,
            y: y0 as i32,
            width: (x1 - x0) as u32,
            height: (y1 - y0) as u32,
        }
    }

    pub fn size(&self) -> (usize, usize) {
        (self.width, self.height)
    }

    /// Fill the current clip (the whole canvas unless a clip is set).
    pub fn clear(&mut self, color: u32) {
        let (x0, y0, x1, y1) = self.clip;
        let color = color & 0x00ff_ffff;
        for y in y0..y1 {
            self.pixels[y * self.stride + x0..y * self.stride + x1].fill(color);
        }
    }

    /// Rectangles are intersected with the canvas, never rejected merely
    /// for being partly offscreen. i64 math cannot overflow on i32+u32.
    /// Return the number of pixels actually touched for structural tests.
    pub fn fill_rect(&mut self, rect: Rect, color: u32) -> usize {
        let (cx0, cy0, cx1, cy1) = self.clip;
        let x0 = i64::from(rect.x).clamp(cx0 as i64, cx1 as i64) as usize;
        let y0 = i64::from(rect.y).clamp(cy0 as i64, cy1 as i64) as usize;
        let x1 = (i64::from(rect.x) + i64::from(rect.width)).clamp(x0 as i64, cx1 as i64) as usize;
        let y1 = (i64::from(rect.y) + i64::from(rect.height)).clamp(y0 as i64, cy1 as i64) as usize;
        let color = color & 0x00ff_ffff;
        for y in y0..y1 {
            self.pixels[y * self.stride + x0..y * self.stride + x1].fill(color);
        }
        (x1 - x0) * (y1 - y0)
    }

    /// This bitmap font is intentionally small and wholly owned here: each
    /// glyph is seven 5-bit rows, width 5, height 7, fixed advance 6. An
    /// unsupported printable glyph renders '?'; non-ASCII or oversized
    /// input is a typed refusal *before* any pixel is changed.
    pub fn text(&mut self, x: i32, y: i32, text: &str, color: u32) -> Result<usize, DrawError> {
        self.text_scaled(x, y, text, color, 1)
    }
    /// Owned bitmap glyphs at bounded integer scale, preserving case.
    /// Invalid scale or text refuses before changing any destination pixel.
    pub fn text_scaled(
        &mut self,
        x: i32,
        y: i32,
        text: &str,
        color: u32,
        scale: u8,
    ) -> Result<usize, DrawError> {
        if !(1..=3).contains(&scale)
            || text.len() > MAX_TEXT_BYTES
            || !text.bytes().all(|c| c.is_ascii_graphic() || c == b' ')
        {
            return Err(DrawError::InvalidText);
        }
        let mut touched = 0usize;
        let color = color & 0x00ff_ffff;
        let (cx0, cy0, cx1, cy1) = (
            self.clip.0 as i64,
            self.clip.1 as i64,
            self.clip.2 as i64,
            self.clip.3 as i64,
        );
        for (i, c) in text.bytes().enumerate() {
            let gx = i64::from(x) + (i as i64) * 6 * i64::from(scale);
            // Whole glyph cells outside the clip cost nothing.
            if gx >= cx1 || gx + 5 * i64::from(scale) <= cx0 {
                continue;
            }
            let bitmap = font::glyph(c);
            for (row, bits) in bitmap.iter().enumerate() {
                let py = i64::from(y) + row as i64 * i64::from(scale);
                if py + i64::from(scale) <= cy0 || py >= cy1 {
                    continue;
                }
                for column in 0..5 {
                    let px = gx + column * i64::from(scale);
                    if px + i64::from(scale) <= cx0
                        || px >= cx1
                        || (bits & (1 << (4 - column))) == 0
                    {
                        continue;
                    }
                    for dy in 0..i64::from(scale) {
                        for dx in 0..i64::from(scale) {
                            let (xx, yy) = (px + dx, py + dy);
                            if xx >= cx0 && yy >= cy0 && xx < cx1 && yy < cy1 {
                                self.pixels[yy as usize * self.stride + xx as usize] = color;
                                touched += 1;
                            }
                        }
                    }
                }
            }
        }
        Ok(touched)
    }

    /// Opaque, clipped z-order composition. The source is a *different*
    /// immutable slice and validated width/stride; the compositor decides
    /// who owns it, not this deliberately authority-free helper.
    pub fn blit(
        &mut self,
        source: &[u32],
        src_width: usize,
        src_height: usize,
        src_stride: usize,
        x: i32,
        y: i32,
    ) -> Result<usize, DrawError> {
        if src_width == 0
            || src_height == 0
            || src_width > MAX_WIDTH
            || src_height > MAX_HEIGHT
            || src_stride < src_width
            || src_stride > MAX_WIDTH
            || src_stride
                .checked_mul(src_height)
                .is_none_or(|span| span > source.len())
        {
            return Err(DrawError::InvalidGeometry);
        }
        let (cx0, cy0, cx1, cy1) = self.clip;
        let x0 = i64::from(x).clamp(cx0 as i64, cx1 as i64) as usize;
        let y0 = i64::from(y).clamp(cy0 as i64, cy1 as i64) as usize;
        let x1 = (i64::from(x) + src_width as i64).clamp(x0 as i64, cx1 as i64) as usize;
        let y1 = (i64::from(y) + src_height as i64).clamp(y0 as i64, cy1 as i64) as usize;
        let sx0 = (x0 as i64 - i64::from(x)) as usize;
        for dy in y0..y1 {
            let sy = (dy as i64 - i64::from(y)) as usize;
            let src = &source[sy * src_stride + sx0..sy * src_stride + sx0 + (x1 - x0)];
            let dst = &mut self.pixels[dy * self.stride + x0..dy * self.stride + x1];
            for (d, s) in dst.iter_mut().zip(src) {
                *d = *s & 0x00ff_ffff;
            }
        }
        Ok((x1 - x0) * (y1 - y0))
    }
}

/// Owned 5x7 ASCII subset. No firmware font, external TTF, allocations,
/// graphics server policy or host font library is linked into ring 3.
mod font {
    pub fn glyph(c: u8) -> [u8; 7] {
        let lower = match c {
            b'a' => Some([0, 0, 14, 1, 15, 17, 15]),
            b'b' => Some([16, 16, 30, 17, 17, 17, 30]),
            b'c' => Some([0, 0, 14, 17, 16, 17, 14]),
            b'd' => Some([1, 1, 15, 17, 17, 17, 15]),
            b'e' => Some([0, 0, 14, 17, 31, 16, 14]),
            b'f' => Some([6, 8, 8, 28, 8, 8, 8]),
            b'g' => Some([0, 0, 15, 17, 15, 1, 14]),
            b'h' => Some([16, 16, 30, 17, 17, 17, 17]),
            b'i' => Some([4, 0, 12, 4, 4, 4, 14]),
            b'j' => Some([2, 0, 6, 2, 2, 18, 12]),
            b'k' => Some([16, 16, 18, 20, 24, 20, 18]),
            b'l' => Some([12, 4, 4, 4, 4, 4, 14]),
            b'm' => Some([0, 0, 26, 21, 21, 21, 21]),
            b'n' => Some([0, 0, 30, 17, 17, 17, 17]),
            b'o' => Some([0, 0, 14, 17, 17, 17, 14]),
            b'p' => Some([0, 0, 30, 17, 30, 16, 16]),
            b'q' => Some([0, 0, 15, 17, 15, 1, 1]),
            b'r' => Some([0, 0, 22, 25, 16, 16, 16]),
            b's' => Some([0, 0, 15, 16, 14, 1, 30]),
            b't' => Some([8, 8, 28, 8, 8, 9, 6]),
            b'u' => Some([0, 0, 17, 17, 17, 19, 13]),
            b'v' => Some([0, 0, 17, 17, 17, 10, 4]),
            b'w' => Some([0, 0, 17, 17, 21, 21, 10]),
            b'x' => Some([0, 0, 17, 10, 4, 10, 17]),
            b'y' => Some([0, 0, 17, 17, 15, 1, 14]),
            b'z' => Some([0, 0, 31, 2, 4, 8, 31]),
            _ => None,
        };
        if let Some(rows) = lower {
            return rows;
        }
        match c.to_ascii_uppercase() {
            b'A' => [14, 17, 17, 31, 17, 17, 17],
            b'B' => [30, 17, 17, 30, 17, 17, 30],
            b'C' => [14, 17, 16, 16, 16, 17, 14],
            b'D' => [30, 17, 17, 17, 17, 17, 30],
            b'E' => [31, 16, 16, 30, 16, 16, 31],
            b'F' => [31, 16, 16, 30, 16, 16, 16],
            b'G' => [14, 17, 16, 23, 17, 17, 15],
            b'H' => [17, 17, 17, 31, 17, 17, 17],
            b'I' => [31, 4, 4, 4, 4, 4, 31],
            b'J' => [7, 2, 2, 2, 18, 18, 12],
            b'K' => [17, 18, 20, 24, 20, 18, 17],
            b'L' => [16, 16, 16, 16, 16, 16, 31],
            b'M' => [17, 27, 21, 21, 17, 17, 17],
            b'N' => [17, 25, 21, 19, 17, 17, 17],
            b'O' => [14, 17, 17, 17, 17, 17, 14],
            b'P' => [30, 17, 17, 30, 16, 16, 16],
            b'Q' => [14, 17, 17, 17, 21, 18, 13],
            b'R' => [30, 17, 17, 30, 20, 18, 17],
            b'S' => [15, 16, 16, 14, 1, 1, 30],
            b'T' => [31, 4, 4, 4, 4, 4, 4],
            b'U' => [17, 17, 17, 17, 17, 17, 14],
            b'V' => [17, 17, 17, 17, 17, 10, 4],
            b'W' => [17, 17, 17, 21, 21, 21, 10],
            b'X' => [17, 17, 10, 4, 10, 17, 17],
            b'Y' => [17, 17, 10, 4, 4, 4, 4],
            b'Z' => [31, 1, 2, 4, 8, 16, 31],
            b'0' => [14, 17, 19, 21, 25, 17, 14],
            b'1' => [4, 12, 4, 4, 4, 4, 14],
            b'2' => [14, 17, 1, 2, 4, 8, 31],
            b'3' => [30, 1, 1, 14, 1, 1, 30],
            b'4' => [2, 6, 10, 18, 31, 2, 2],
            b'5' => [31, 16, 16, 30, 1, 1, 30],
            b'6' => [14, 16, 16, 30, 17, 17, 14],
            b'7' => [31, 1, 2, 4, 8, 8, 8],
            b'8' => [14, 17, 17, 14, 17, 17, 14],
            b'9' => [14, 17, 17, 15, 1, 1, 14],
            b' ' => [0; 7],
            b'.' => [0, 0, 0, 0, 0, 12, 12],
            b':' => [0, 12, 12, 0, 12, 12, 0],
            b',' => [0, 0, 0, 0, 12, 12, 8],
            b'-' => [0, 0, 0, 31, 0, 0, 0],
            b'+' => [0, 4, 4, 31, 4, 4, 0],
            b'/' => [1, 1, 2, 4, 8, 16, 16],
            b'!' => [4, 4, 4, 4, 4, 0, 4],
            b'?' => [14, 17, 1, 2, 4, 0, 4],
            b'(' => [2, 4, 8, 8, 8, 4, 2],
            b')' => [8, 4, 2, 2, 2, 4, 8],
            b'[' => [14, 8, 8, 8, 8, 8, 14],
            b']' => [14, 2, 2, 2, 2, 2, 14],
            b'=' => [0, 0, 31, 0, 31, 0, 0],
            b'_' => [0, 0, 0, 0, 0, 0, 31],
            b'>' => [16, 8, 4, 2, 4, 8, 16],
            b'<' => [1, 2, 4, 8, 4, 2, 1],
            b'|' => [4, 4, 4, 4, 4, 4, 4],
            b'%' => [25, 25, 2, 4, 8, 19, 19],
            b'*' => [0, 21, 14, 31, 14, 21, 0],
            b'#' => [10, 31, 10, 10, 31, 10, 0],
            b'&' => [12, 18, 20, 8, 21, 18, 13],
            b'@' => [14, 17, 23, 21, 23, 16, 14],
            b'$' => [4, 15, 20, 14, 5, 30, 4],
            b'^' => [4, 10, 17, 0, 0, 0, 0],
            b'~' => [0, 0, 9, 22, 0, 0, 0],
            b'`' => [8, 4, 0, 0, 0, 0, 0],
            b'\'' => [4, 4, 0, 0, 0, 0, 0],
            b'"' => [10, 10, 0, 0, 0, 0, 0],
            b'\\' => [16, 16, 8, 4, 2, 1, 1],
            b'{' => [2, 4, 4, 8, 4, 4, 2],
            b'}' => [8, 4, 4, 2, 4, 4, 8],
            b';' => [0, 12, 12, 0, 12, 12, 8],
            _ => [14, 17, 1, 2, 4, 0, 4], // conspicuous replacement
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checked_geometry_and_disjoint_padding() {
        let mut pixels = [0xdead_beefu32; 4 * 3];
        assert!(Canvas::new(&mut pixels, 5, 3, 4).is_err());
        let mut c = Canvas::new(&mut pixels, 3, 3, 4).unwrap();
        c.clear(0xff34_5678);
        assert_eq!(
            c.fill_rect(
                Rect {
                    x: -1,
                    y: -1,
                    width: 2,
                    height: 2
                },
                0x00abcdef
            ),
            1
        );
        assert_eq!(
            c.fill_rect(
                Rect {
                    x: i32::MAX,
                    y: i32::MAX,
                    width: u32::MAX,
                    height: u32::MAX
                },
                0
            ),
            0
        );
        assert_eq!(pixels[0], 0x00ab_cdef);
        assert_eq!([pixels[3], pixels[7], pixels[11]], [0xdead_beef; 3]);
    }

    #[test]
    fn owned_font_and_bounded_rejection() {
        let mut pixels = [0u32; 24 * 16];
        let mut c = Canvas::new(&mut pixels, 24, 16, 24).unwrap();
        assert!(c.text(1, 1, "A9!?", 0x00ffffff).unwrap() > 20);
        let mut before = [0u32; 24 * 16];
        before.copy_from_slice(c.pixels);
        assert_eq!(c.text(0, 0, "☃", 1), Err(DrawError::InvalidText));
        let long = [b'X'; MAX_TEXT_BYTES + 1];
        assert_eq!(
            c.text(0, 0, core::str::from_utf8(&long).unwrap(), 1),
            Err(DrawError::InvalidText)
        );
        assert_eq!(c.pixels, before);
        assert_eq!(c.text_scaled(0, 0, "A", 1, 0), Err(DrawError::InvalidText));
        assert_eq!(c.text_scaled(0, 0, "A", 1, 4), Err(DrawError::InvalidText));
        assert_eq!(c.pixels, before);
        assert!(c.text_scaled(-3, -2, "a", 1, 2).unwrap() > 20);
        assert_ne!(font::glyph(b'a'), font::glyph(b'A'));
        for c in b'a'..=b'z' {
            assert_ne!(font::glyph(c), font::glyph(c.to_ascii_uppercase()));
        }
        assert_ne!(font::glyph(b'A'), font::glyph(b'B'));
    }

    #[test]
    fn clipped_opaque_blit_and_refusals() {
        let mut dst = [0x123u32; 5 * 5];
        let src = [0xAABB_CCDDu32; 3 * 3];
        let mut c = Canvas::new(&mut dst, 5, 5, 5).unwrap();
        assert_eq!(c.blit(&src, 3, 3, 3, -1, -1), Ok(4));
        assert_eq!(c.pixels[0], 0x00bb_ccdd);
        assert_eq!(
            c.blit(&src, usize::MAX, 3, 3, 0, 0),
            Err(DrawError::InvalidGeometry)
        );
        assert_eq!(
            c.blit(&src, 3, 3, usize::MAX, 0, 0),
            Err(DrawError::InvalidGeometry)
        );
        assert_eq!(c.blit(&src, 3, 3, 3, i32::MAX, i32::MAX), Ok(0));
        assert_eq!(c.pixels[24], 0x123);
    }

    #[test]
    fn clip_restricts_every_operation_exactly() {
        // Drawing the same scene clipped must equal the unclipped scene
        // inside the clip and leave every pixel outside it untouched.
        fn scene(c: &mut Canvas<'_>) {
            c.clear(0x0011_2233);
            c.fill_rect(
                Rect {
                    x: 3,
                    y: 2,
                    width: 20,
                    height: 9,
                },
                0x00ff_0000,
            );
            let src = [0x00aa_bbccu32; 6 * 4];
            c.blit(&src, 6, 4, 6, 10, 5).unwrap();
            c.text_scaled(1, 8, "Ab9", 0x0000_ff00, 2).unwrap();
        }
        let (w, h) = (32usize, 24usize);
        let mut full = [0u32; 32 * 24];
        scene(&mut Canvas::new(&mut full, w, h, w).unwrap());
        let clip = Rect {
            x: 7,
            y: 4,
            width: 11,
            height: 13,
        };
        let mut part = [0xdead_beefu32; 32 * 24];
        let mut c = Canvas::new(&mut part, w, h, w).unwrap();
        c.set_clip(clip);
        assert_eq!(c.clip(), clip);
        scene(&mut c);
        for y in 0..h {
            for x in 0..w {
                let inside = (7..18).contains(&x) && (4..17).contains(&y);
                let expect = if inside { full[y * w + x] } else { 0xdead_beef };
                assert_eq!(part[y * w + x], expect, "({x},{y})");
            }
        }
        // Clip is intersected with the canvas; reset restores everything.
        let mut c = Canvas::new(&mut part, w, h, w).unwrap();
        c.set_clip(Rect {
            x: -5,
            y: 20,
            width: 100,
            height: 100,
        });
        assert_eq!(
            c.clip(),
            Rect {
                x: 0,
                y: 20,
                width: 32,
                height: 4
            }
        );
        c.set_clip(Rect {
            x: 40,
            y: 40,
            width: 4,
            height: 4,
        });
        assert_eq!(
            c.fill_rect(
                Rect {
                    x: 0,
                    y: 0,
                    width: 32,
                    height: 24
                },
                1
            ),
            0
        );
        c.reset_clip();
        assert_eq!(
            c.fill_rect(
                Rect {
                    x: 0,
                    y: 0,
                    width: 32,
                    height: 24
                },
                1
            ),
            32 * 24
        );
    }
}
