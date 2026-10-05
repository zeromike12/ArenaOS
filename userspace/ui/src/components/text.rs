//! Typography on the two Arena-owned bitmap faces.
//!
//! Body and Display use the 5x7 face on its fixed grid (the terminal and
//! editor depend on the grid). Strong and Caption use Arena Sans 13, the
//! proportional face (Phase 11.7): its capitals sit in the 5x7 box (their
//! art starts one row above it) so every layout keeps its baselines.
use crate::metrics as m;
use arena_gfxkit::{Canvas, measure13};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Style {
    /// 5x7, advance 6. Running text, values, console and editor text.
    Body,
    /// Arena Sans 13 bold. Window titles, names, emphasis.
    Strong,
    /// Arena Sans 13, upper-case. Section labels, status lines, eyebrows.
    Caption,
    /// Scale 2, advance 12. Page titles and primary figures.
    Display,
}

pub const fn advance(style: Style) -> i32 {
    match style {
        Style::Body => m::FONT_ADVANCE,
        Style::Strong | Style::Caption => m::TRACKED_ADVANCE,
        Style::Display => m::TITLE_FONT_ADVANCE,
    }
}

pub const fn height(style: Style) -> i32 {
    match style {
        Style::Display => m::TITLE_FONT_HEIGHT,
        _ => m::FONT_HEIGHT,
    }
}

/// Inked width of `s` (no trailing inter-glyph gap).
pub fn measure(s: &str, style: Style) -> i32 {
    match style {
        Style::Strong => measure13(s, true),
        Style::Caption => {
            let mut w = 0;
            let mut buf = [0u8; 4];
            for (i, ch) in s.chars().enumerate() {
                let up = ch.to_ascii_uppercase();
                w += measure13(up.encode_utf8(&mut buf), false) + i32::from(i != 0);
            }
            w
        }
        _ => {
            let n = s.len() as i32;
            if n == 0 {
                return 0;
            }
            let gap = if style == Style::Display { 2 } else { 1 };
            n * advance(style) - gap
        }
    }
}

/// Arena Sans capitals are 9 rows from art row 0; the 5x7 box starts 2
/// rows lower (cell row 1 plus one row).
const SANS_RAISE: i32 = 2;

/// Draws `s` and returns the x just after its last advance.
pub fn text(c: &mut Canvas<'_>, x: i32, y: i32, s: &str, style: Style, color: u32) -> i32 {
    match style {
        Style::Body => {
            // Split long strings without weakening gfxkit's bounded input contract.
            for (i, chunk) in s.as_bytes().chunks(96).enumerate() {
                if let Ok(part) = core::str::from_utf8(chunk) {
                    let _ = c.text_scaled(x + (i * 96) as i32 * m::FONT_ADVANCE, y, part, color, 1);
                }
            }
        }
        Style::Display => {
            for (i, chunk) in s.as_bytes().chunks(96).enumerate() {
                if let Ok(part) = core::str::from_utf8(chunk) {
                    let _ = c.text_scaled(
                        x + (i * 96) as i32 * m::TITLE_FONT_ADVANCE,
                        y,
                        part,
                        color,
                        m::TITLE_SCALE,
                    );
                }
            }
        }
        Style::Strong => {
            let _ = c.text13(x, y - SANS_RAISE, s, color, true);
        }
        Style::Caption => {
            // Upper-cased in bounded chunks (no allocation).
            let mut pen = x;
            let mut buf = [0u8; 64];
            let mut n = 0;
            let flush = |c: &mut Canvas<'_>, buf: &[u8], pen: &mut i32| {
                if let Ok(part) = core::str::from_utf8(buf) {
                    let _ = c.text13(*pen, y - SANS_RAISE, part, color, false);
                    *pen += measure13(part, false) + 1;
                }
            };
            for ch in s.chars() {
                let up = ch.to_ascii_uppercase();
                if n + up.len_utf8() > buf.len() {
                    flush(c, &buf[..n], &mut pen);
                    n = 0;
                }
                up.encode_utf8(&mut buf[n..]);
                n += up.len_utf8();
            }
            flush(c, &buf[..n], &mut pen);
        }
    }
    match style {
        Style::Strong | Style::Caption => x + measure(s, style) + 1,
        _ => x + s.len() as i32 * advance(style),
    }
}

/// Longest prefix of `s` that fits `width`, and whether it was shortened.
pub fn fit(s: &str, style: Style, width: i32) -> (&str, bool) {
    if matches!(style, Style::Strong | Style::Caption) {
        if measure(s, style) <= width {
            return (s, false);
        }
        let room = width - measure("..", style) - 1;
        let mut end = 0;
        for (i, ch) in s.char_indices() {
            let next = i + ch.len_utf8();
            if measure(&s[..next], style) > room {
                break;
            }
            end = next;
        }
        return (&s[..end], true);
    }
    let max = (width.max(0) / advance(style)) as usize;
    if s.len() <= max {
        return (s, false);
    }
    let keep = max.saturating_sub(2).min(s.len());
    (s.get(..keep).unwrap_or(""), true)
}

/// Draws `s` clipped to `width`; a shortened value ends in "..".
pub fn text_fit(
    c: &mut Canvas<'_>,
    x: i32,
    y: i32,
    s: &str,
    style: Style,
    color: u32,
    width: i32,
) -> i32 {
    let (shown, cut) = fit(s, style, width);
    let end = text(c, x, y, shown, style, color);
    if cut {
        text(c, end, y, "..", style, color)
    } else {
        end
    }
}

/// Text centred horizontally in `[x, x + width)`.
pub fn text_centered(
    c: &mut Canvas<'_>,
    x: i32,
    width: i32,
    y: i32,
    s: &str,
    style: Style,
    color: u32,
) {
    let (shown, _) = fit(s, style, width);
    text(
        c,
        x + (width - measure(shown, style)) / 2,
        y,
        shown,
        style,
        color,
    );
}

/// Right-aligned text ending at `right`.
pub fn text_right(c: &mut Canvas<'_>, right: i32, y: i32, s: &str, style: Style, color: u32) {
    text(c, right - measure(s, style), y, s, style, color);
}

pub fn label(c: &mut Canvas<'_>, x: i32, y: i32, s: &str, color: u32) {
    text(c, x, y, s, Style::Body, color);
}

pub fn strong(c: &mut Canvas<'_>, x: i32, y: i32, s: &str, color: u32) {
    text(c, x, y, s, Style::Strong, color);
}

pub fn caption(c: &mut Canvas<'_>, x: i32, y: i32, s: &str, color: u32) {
    text(c, x, y, s, Style::Caption, color);
}

pub fn heading(c: &mut Canvas<'_>, x: i32, y: i32, s: &str, color: u32) {
    text(c, x, y, s, Style::Display, color);
}

/// Formats an unsigned decimal into `out`, returning the used prefix.
/// `group` inserts thin-space-free comma grouping (1,231) for figures.
pub fn decimal(out: &mut [u8; 32], mut value: u64, group: bool) -> &str {
    let mut digits = [0u8; 20];
    let mut n = 0;
    loop {
        digits[n] = b'0' + (value % 10) as u8;
        n += 1;
        value /= 10;
        if value == 0 {
            break;
        }
    }
    let mut len = 0;
    for i in (0..n).rev() {
        out[len] = digits[i];
        len += 1;
        if group && i != 0 && i % 3 == 0 {
            out[len] = b',';
            len += 1;
        }
    }
    core::str::from_utf8(&out[..len]).unwrap_or("?")
}
