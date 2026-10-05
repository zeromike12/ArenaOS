//! Phase 11.7 widgets: the parts a file explorer is built from.
//!
//! Each widget is plain state plus geometry (pure, host-tested) and a draw
//! function (gfxkit, theme tokens, Arena Sans 13). Nothing here holds
//! authority or does IO: views pass the data they already have.
use crate::components::{Glyph, glyph, hline, outlined, rect, vline};
use crate::{metrics as m, theme::Theme};
use arena_gfxkit::{Canvas, Rect, face13, measure13};

/// Row pitch of lists and the height of Arena Sans 13 text.
pub const ROW: i32 = 20;
pub const HEADER: i32 = 22;
pub const TEXT_H: i32 = face13::HEIGHT as i32;
/// Scrollbar thickness and its minimum thumb length.
pub const BAR: i32 = 10;
pub const THUMB_MIN: i32 = 16;

/// Text in Arena Sans 13, vertically centred in a `h`-pixel band at `y`,
/// clipped to `width` with a trailing ellipsis. Returns the inked width.
#[allow(clippy::too_many_arguments)]
pub fn text_in(
    c: &mut Canvas<'_>,
    x: i32,
    y: i32,
    h: i32,
    width: i32,
    s: &str,
    color: u32,
    bold: bool,
) -> i32 {
    let top = y + (h - TEXT_H) / 2;
    if measure13(s, bold) <= width {
        return c.text13(x, top, s, color, bold).unwrap_or(0);
    }
    let ell = measure13("\u{2026}", bold) + 1;
    let mut end = 0;
    for (i, ch) in s.char_indices() {
        let next = i + ch.len_utf8();
        if measure13(&s[..next], bold) + ell > width {
            break;
        }
        end = next;
    }
    let w = c.text13(x, top, &s[..end], color, bold).unwrap_or(0);
    let e = c
        .text13(x + w + 1, top, "\u{2026}", color, bold)
        .unwrap_or(0);
    w + 1 + e
}

// ---- scrolling -----------------------------------------------------------------

/// A scroll position over `content` pixels seen through `viewport` pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Scroll {
    pub offset: i32,
    pub content: i32,
    pub viewport: i32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScrollHit {
    Thumb,
    PageUp,
    PageDown,
}

impl Scroll {
    pub fn max_offset(&self) -> i32 {
        (self.content - self.viewport).max(0)
    }
    pub fn set(&mut self, offset: i32) {
        self.offset = offset.clamp(0, self.max_offset());
    }
    pub fn by(&mut self, delta: i32) {
        self.set(self.offset.saturating_add(delta));
    }
    /// Scroll the least amount that shows `[top, top + height)` entirely.
    pub fn reveal(&mut self, top: i32, height: i32) {
        if top < self.offset {
            self.set(top);
        } else if top + height > self.offset + self.viewport {
            self.set(top + height - self.viewport);
        }
    }
    /// The thumb inside `track` (None when everything fits).
    pub fn thumb(&self, track: Rect) -> Option<Rect> {
        if self.content <= self.viewport || self.viewport <= 0 {
            return None;
        }
        let len = track.height as i32;
        let size = (len * self.viewport / self.content).clamp(THUMB_MIN.min(len), len);
        let room = len - size;
        let pos = if self.max_offset() == 0 {
            0
        } else {
            room * self.offset / self.max_offset()
        };
        Some(Rect {
            x: track.x,
            y: track.y + pos,
            width: track.width,
            height: size as u32,
        })
    }
    pub fn hit(&self, track: Rect, y: i32) -> Option<ScrollHit> {
        let t = self.thumb(track)?;
        Some(if y < t.y {
            ScrollHit::PageUp
        } else if y >= t.y + t.height as i32 {
            ScrollHit::PageDown
        } else {
            ScrollHit::Thumb
        })
    }
    /// Offset after dragging the thumb by `dy` pixels from `start` offset.
    pub fn drag(&self, track: Rect, start: i32, dy: i32) -> i32 {
        let Some(t) = self.thumb(track) else {
            return 0;
        };
        let room = track.height as i32 - t.height as i32;
        if room <= 0 {
            return start;
        }
        (start + dy * self.max_offset() / room).clamp(0, self.max_offset())
    }
}

pub fn scrollbar(c: &mut Canvas<'_>, track: Rect, s: &Scroll, active: bool, t: Theme) {
    let Some(thumb) = s.thumb(track) else {
        return;
    };
    rect(
        c,
        track.x,
        track.y,
        track.width as i32,
        track.height as i32,
        t.track,
    );
    let inner = Rect {
        x: thumb.x + 2,
        y: thumb.y + 1,
        width: thumb.width.saturating_sub(4),
        height: thumb.height.saturating_sub(2),
    };
    outlined(
        c,
        inner,
        if active { t.secondary } else { t.muted },
        if active { t.secondary } else { t.muted },
        2,
    );
}

// ---- selection -------------------------------------------------------------------

/// Selected items among up to `Selection::MAX` (click, Shift range, Ctrl
/// toggle), with the anchor a Shift range extends from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Selection {
    bits: [u64; 16],
    pub anchor: Option<usize>,
    /// The keyboard cursor (where arrows move from).
    pub cursor: Option<usize>,
}

impl Default for Selection {
    fn default() -> Self {
        Self::EMPTY
    }
}

impl Selection {
    pub const MAX: usize = 1024;
    pub const EMPTY: Selection = Selection {
        bits: [0; 16],
        anchor: None,
        cursor: None,
    };
    pub fn contains(&self, i: usize) -> bool {
        i < Self::MAX && self.bits[i / 64] >> (i % 64) & 1 == 1
    }
    fn put(&mut self, i: usize, on: bool) {
        if i < Self::MAX {
            if on {
                self.bits[i / 64] |= 1 << (i % 64);
            } else {
                self.bits[i / 64] &= !(1 << (i % 64));
            }
        }
    }
    pub fn clear(&mut self) {
        *self = Self::EMPTY;
    }
    pub fn count(&self) -> usize {
        self.bits.iter().map(|b| b.count_ones() as usize).sum()
    }
    /// The first selected item.
    pub fn first(&self) -> Option<usize> {
        (0..Self::MAX).find(|i| self.contains(*i))
    }
    /// A pointer press on item `i`: replace, extend (Shift) or toggle (Ctrl).
    pub fn click(&mut self, i: usize, shift: bool, ctrl: bool) {
        if shift {
            let a = self.anchor.unwrap_or(i);
            if !ctrl {
                self.bits = [0; 16];
            }
            for k in a.min(i)..=a.max(i) {
                self.put(k, true);
            }
        } else if ctrl {
            self.put(i, !self.contains(i));
            self.anchor = Some(i);
        } else {
            self.bits = [0; 16];
            self.put(i, true);
            self.anchor = Some(i);
        }
        self.cursor = Some(i);
    }
    /// Arrow keys over `n` items: move the cursor by `delta` (Shift extends).
    pub fn step(&mut self, delta: isize, n: usize, shift: bool) {
        if n == 0 {
            return;
        }
        let from = self.cursor.unwrap_or(0) as isize;
        let to = if self.cursor.is_none() {
            0
        } else {
            (from + delta).clamp(0, n as isize - 1) as usize
        };
        self.click(to, shift, false);
    }
    pub fn select_all(&mut self, n: usize) {
        for i in 0..n.min(Self::MAX) {
            self.put(i, true);
        }
    }
    /// Drop selection beyond `n` items (the listing shrank).
    pub fn truncate(&mut self, n: usize) {
        for i in n..Self::MAX {
            self.put(i, false);
        }
        if self.cursor.is_some_and(|c| c >= n) {
            self.cursor = n.checked_sub(1);
        }
        if self.anchor.is_some_and(|a| a >= n) {
            self.anchor = self.cursor;
        }
    }
}

// ---- list with columns -------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Column {
    pub title: &'static str,
    pub width: i32,
    /// Right-aligned (sizes, dates).
    pub right: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ListHit {
    Header(usize),
    Row(usize),
    Scrollbar(ScrollHit),
    Blank,
}

/// A virtualized list: only visible rows are ever drawn or asked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct List {
    pub rows: usize,
    pub scroll: Scroll,
    /// Sorted column and whether descending.
    pub sort: Option<(usize, bool)>,
    pub hover: Option<usize>,
}

impl List {
    pub const fn new() -> Self {
        List {
            rows: 0,
            scroll: Scroll {
                offset: 0,
                content: 0,
                viewport: 0,
            },
            sort: None,
            hover: None,
        }
    }
    /// Fit the scroll range to `area` (header included) and `rows`.
    pub fn layout(&mut self, area: Rect, rows: usize) {
        self.rows = rows;
        self.scroll.content = rows as i32 * ROW;
        self.scroll.viewport = area.height as i32 - HEADER;
        let o = self.scroll.offset;
        self.scroll.set(o);
    }
    pub fn body(area: Rect) -> Rect {
        Rect {
            x: area.x,
            y: area.y + HEADER,
            width: area.width,
            height: (area.height as i32 - HEADER).max(0) as u32,
        }
    }
    pub fn track(area: Rect) -> Rect {
        let b = Self::body(area);
        Rect {
            x: b.x + b.width as i32 - BAR,
            y: b.y,
            width: BAR as u32,
            height: b.height,
        }
    }
    /// Rows at least partly visible: `first..end`.
    pub fn visible(&self) -> (usize, usize) {
        let first = (self.scroll.offset / ROW).max(0) as usize;
        let end = ((self.scroll.offset + self.scroll.viewport + ROW - 1) / ROW).max(0) as usize;
        (first.min(self.rows), end.min(self.rows))
    }
    pub fn hit(&self, area: Rect, columns: &[Column], x: i32, y: i32) -> ListHit {
        if x < area.x
            || y < area.y
            || x >= area.x + area.width as i32
            || y >= area.y + area.height as i32
        {
            return ListHit::Blank;
        }
        if y < area.y + HEADER {
            let mut cx = area.x;
            for (i, col) in columns.iter().enumerate() {
                if x < cx + col.width {
                    return ListHit::Header(i);
                }
                cx += col.width;
            }
            return ListHit::Blank;
        }
        let track = Self::track(area);
        if x >= track.x
            && let Some(h) = self.scroll.hit(track, y)
        {
            return ListHit::Scrollbar(h);
        }
        let row = ((y - area.y - HEADER + self.scroll.offset) / ROW) as usize;
        if row < self.rows {
            ListHit::Row(row)
        } else {
            ListHit::Blank
        }
    }
    /// Scroll so row `i` is fully visible.
    pub fn reveal(&mut self, i: usize) {
        self.scroll.reveal(i as i32 * ROW, ROW);
    }

    /// Draw the header, the visible rows (asking `cell(row, column)` only
    /// for those) and the scrollbar. `icon(row)` marks the first column.
    /// Returns how many rows were drawn (virtualization is observable).
    #[allow(clippy::too_many_arguments)]
    pub fn draw<'a>(
        &self,
        c: &mut Canvas<'_>,
        area: Rect,
        columns: &[Column],
        selection: &Selection,
        focused: bool,
        mut cell: impl FnMut(usize, usize) -> &'a str,
        mut icon: impl FnMut(usize) -> Option<Glyph>,
        t: Theme,
    ) -> usize {
        rect(
            c,
            area.x,
            area.y,
            area.width as i32,
            area.height as i32,
            t.field,
        );
        // Header.
        rect(c, area.x, area.y, area.width as i32, HEADER, t.header);
        hline(c, area.x, area.y + HEADER - 1, area.width as i32, t.divider);
        let mut cx = area.x;
        for (i, col) in columns.iter().enumerate() {
            let w = col.width.min(area.x + area.width as i32 - cx);
            if w <= 0 {
                break;
            }
            let label_w = w - 16;
            if col.right {
                let tw = measure13(col.title, true).min(label_w);
                text_in(
                    c,
                    cx + w - 8 - tw,
                    area.y,
                    HEADER,
                    label_w,
                    col.title,
                    t.secondary,
                    true,
                );
            } else {
                text_in(
                    c,
                    cx + 8,
                    area.y,
                    HEADER,
                    label_w,
                    col.title,
                    t.secondary,
                    true,
                );
            }
            if let Some((s, desc)) = self.sort
                && s == i
            {
                let ax = if col.right { cx + 2 } else { cx + w - 12 };
                sort_arrow(c, ax, area.y + HEADER / 2 - 2, desc, t.secondary);
            }
            if i + 1 < columns.len() {
                vline(c, cx + w - 1, area.y + 5, HEADER - 10, t.divider);
            }
            cx += w;
        }
        // Rows: only the visible ones.
        let body = Self::body(area);
        let saved = c.clip();
        let clip = intersect(saved, body);
        c.set_clip(clip);
        let (first, end) = self.visible();
        let mut drawn = 0;
        let inner_w = body.width as i32 - BAR;
        for row in first..end {
            let y = body.y + row as i32 * ROW - self.scroll.offset;
            let on = selection.contains(row);
            if on {
                rect(
                    c,
                    body.x,
                    y,
                    inner_w,
                    ROW,
                    if focused { t.accent } else { t.accent_soft },
                );
            } else if self.hover == Some(row) {
                c.blend_rect(
                    Rect {
                        x: body.x,
                        y,
                        width: inner_w as u32,
                        height: ROW as u32,
                    },
                    t.accent,
                    24,
                );
            }
            if focused && selection.cursor == Some(row) && !on {
                dotted_row(c, body.x, y, inner_w, t.accent);
            }
            let ink = if on && focused { t.on_accent } else { t.text };
            let sub = if on && focused {
                t.on_accent
            } else {
                t.secondary
            };
            let mut cx = body.x;
            for (i, col) in columns.iter().enumerate() {
                let w = col.width.min(body.x + inner_w - cx);
                if w <= 0 {
                    break;
                }
                let mut x = cx + 8;
                if i == 0
                    && let Some(g) = icon(row)
                {
                    glyph(
                        c,
                        x,
                        y + (ROW - 9) / 2,
                        g,
                        if on && focused { t.on_accent } else { t.accent },
                    );
                    x += 16;
                }
                let s = cell(row, i);
                let room = cx + w - 8 - x;
                if col.right {
                    let tw = measure13(s, false).min(room);
                    text_in(c, cx + w - 8 - tw, y, ROW, room, s, sub, false);
                } else {
                    text_in(c, x, y, ROW, room, s, if i == 0 { ink } else { sub }, false);
                }
                cx += w;
            }
            drawn += 1;
        }
        c.set_clip(saved);
        scrollbar(c, Self::track(area), &self.scroll, focused, t);
        drawn
    }
}

impl Default for List {
    fn default() -> Self {
        Self::new()
    }
}

fn sort_arrow(c: &mut Canvas<'_>, x: i32, y: i32, desc: bool, color: u32) {
    let rows: [u16; 4] = if desc {
        [0b1111111, 0b0111110, 0b0011100, 0b0001000]
    } else {
        [0b0001000, 0b0011100, 0b0111110, 0b1111111]
    };
    c.bits_blit(&rows, 7, x, y, color);
}

fn dotted_row(c: &mut Canvas<'_>, x: i32, y: i32, w: i32, color: u32) {
    let mut i = 0;
    while i < w {
        rect(c, x + i, y, 1, 1, color);
        rect(c, x + i, y + ROW - 1, 1, 1, color);
        i += 2;
    }
}

fn intersect(a: Rect, b: Rect) -> Rect {
    let x0 = a.x.max(b.x);
    let y0 = a.y.max(b.y);
    let x1 = (a.x + a.width as i32).min(b.x + b.width as i32);
    let y1 = (a.y + a.height as i32).min(b.y + b.height as i32);
    Rect {
        x: x0,
        y: y0,
        width: (x1 - x0).max(0) as u32,
        height: (y1 - y0).max(0) as u32,
    }
}

// ---- icon grid --------------------------------------------------------------------

pub const CELL_W: i32 = 84;
pub const CELL_H: i32 = 72;

/// Icons in rows: `columns` cells across `area`, scrolled vertically.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Grid {
    pub items: usize,
    pub scroll: Scroll,
}

impl Grid {
    pub fn columns(area: Rect) -> usize {
        ((area.width as i32 - BAR) / CELL_W).max(1) as usize
    }
    pub fn layout(&mut self, area: Rect, items: usize) {
        self.items = items;
        let rows = items.div_ceil(Self::columns(area));
        self.scroll.content = rows as i32 * CELL_H;
        self.scroll.viewport = area.height as i32;
        let o = self.scroll.offset;
        self.scroll.set(o);
    }
    pub fn cell(&self, area: Rect, i: usize) -> Rect {
        let cols = Self::columns(area);
        Rect {
            x: area.x + (i % cols) as i32 * CELL_W,
            y: area.y + (i / cols) as i32 * CELL_H - self.scroll.offset,
            width: CELL_W as u32,
            height: CELL_H as u32,
        }
    }
    pub fn hit(&self, area: Rect, x: i32, y: i32) -> Option<usize> {
        if x < area.x
            || y < area.y
            || x >= area.x + area.width as i32 - BAR
            || y >= area.y + area.height as i32
        {
            return None;
        }
        let cols = Self::columns(area);
        let col = ((x - area.x) / CELL_W) as usize;
        let row = ((y - area.y + self.scroll.offset) / CELL_H) as usize;
        let i = row * cols + col;
        (col < cols && i < self.items).then_some(i)
    }
    pub fn reveal(&mut self, area: Rect, i: usize) {
        let row = (i / Self::columns(area)) as i32;
        self.scroll.reveal(row * CELL_H, CELL_H);
    }
}

// ---- single-line text input ------------------------------------------------------------

pub const INPUT_MAX: usize = 255;

/// An editable name: printable ASCII, a caret and an optional selection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Input {
    pub buf: [u8; INPUT_MAX],
    pub len: usize,
    pub cursor: usize,
    /// The other end of the selection, when there is one.
    pub anchor: Option<usize>,
}

/// Key codes shared with the desktop input wire.
pub mod keys {
    pub const BACKSPACE: u16 = 8;
    pub const LEFT: u16 = 256;
    pub const RIGHT: u16 = 257;
    pub const HOME: u16 = 260;
    pub const END: u16 = 261;
    pub const DELETE: u16 = 262;
}

impl Input {
    pub const EMPTY: Input = Input {
        buf: [0; INPUT_MAX],
        len: 0,
        cursor: 0,
        anchor: None,
    };
    pub fn text(&self) -> &str {
        core::str::from_utf8(&self.buf[..self.len]).unwrap_or("")
    }
    pub fn set(&mut self, s: &[u8]) {
        let n = s.len().min(INPUT_MAX);
        self.buf[..n].copy_from_slice(&s[..n]);
        self.len = n;
        self.cursor = n;
        self.anchor = None;
    }
    /// Select everything (rename starts with the name selected).
    pub fn select_all(&mut self) {
        self.anchor = Some(0);
        self.cursor = self.len;
    }
    /// Select `[from, to)` (e.g. a name without its extension).
    pub fn select(&mut self, from: usize, to: usize) {
        self.anchor = Some(from.min(self.len));
        self.cursor = to.min(self.len);
    }
    pub fn selection(&self) -> Option<(usize, usize)> {
        let a = self.anchor?;
        (a != self.cursor).then(|| (a.min(self.cursor), a.max(self.cursor)))
    }
    fn delete_selection(&mut self) -> bool {
        let Some((a, b)) = self.selection() else {
            return false;
        };
        self.buf.copy_within(b..self.len, a);
        self.len -= b - a;
        self.cursor = a;
        self.anchor = None;
        true
    }
    /// One key; returns whether the text or caret changed.
    pub fn key(&mut self, code: u16, shift: bool) -> bool {
        use keys::*;
        match code {
            32..=126 => {
                self.delete_selection();
                if self.len == INPUT_MAX {
                    return false;
                }
                self.buf.copy_within(self.cursor..self.len, self.cursor + 1);
                self.buf[self.cursor] = code as u8;
                self.len += 1;
                self.cursor += 1;
            }
            BACKSPACE => {
                if !self.delete_selection() && self.cursor > 0 {
                    self.buf.copy_within(self.cursor..self.len, self.cursor - 1);
                    self.len -= 1;
                    self.cursor -= 1;
                }
            }
            DELETE => {
                if !self.delete_selection() && self.cursor < self.len {
                    self.buf.copy_within(self.cursor + 1..self.len, self.cursor);
                    self.len -= 1;
                }
            }
            LEFT | RIGHT | HOME | END => {
                let to = match code {
                    LEFT if !shift && self.selection().is_some() => {
                        self.selection().map_or(0, |s| s.0)
                    }
                    RIGHT if !shift && self.selection().is_some() => {
                        self.selection().map_or(0, |s| s.1)
                    }
                    LEFT => self.cursor.saturating_sub(1),
                    RIGHT => (self.cursor + 1).min(self.len),
                    HOME => 0,
                    _ => self.len,
                };
                if shift {
                    self.anchor.get_or_insert(self.cursor);
                } else {
                    self.anchor = None;
                }
                self.cursor = to;
            }
            _ => return false,
        }
        true
    }
    /// The caret index nearest x-offset `dx` from the text start.
    pub fn index_at(&self, dx: i32) -> usize {
        let s = self.text();
        let mut best = 0;
        for i in 0..=self.len {
            let w = if i == 0 {
                0
            } else {
                measure13(&s[..i], false) + 1
            };
            if w - 1 <= dx {
                best = i;
            }
        }
        best
    }
    pub fn draw(&self, c: &mut Canvas<'_>, r: Rect, focused: bool, t: Theme) {
        outlined(
            c,
            r,
            t.field,
            if focused { t.frame_focus } else { t.field_edge },
            m::RADIUS,
        );
        let x = r.x + 6;
        let top = r.y + (r.height as i32 - TEXT_H) / 2;
        let s = self.text();
        let at = |i: usize| {
            if i == 0 {
                0
            } else {
                measure13(&s[..i], false) + 1
            }
        };
        let saved = c.clip();
        c.set_clip(intersect(
            saved,
            Rect {
                x: r.x + 2,
                y: r.y + 2,
                width: r.width.saturating_sub(4),
                height: r.height.saturating_sub(4),
            },
        ));
        if let Some((a, b)) = self.selection() {
            let x0 = x + at(a) - 1;
            let w = at(b) - at(a) + 1;
            rect(
                c,
                x0,
                top,
                w,
                TEXT_H,
                if focused { t.accent_soft } else { t.header },
            );
        }
        let _ = c.text13(x, top, s, t.text, false);
        if focused {
            rect(c, x + at(self.cursor) - 1, top + 1, 1, TEXT_H - 2, t.text);
        }
        c.set_clip(saved);
    }
}

// ---- breadcrumb ---------------------------------------------------------------------

const CHEVRON: [u16; 7] = [0b001, 0b010, 0b100, 0b100, 0b100, 0b010, 0b001];

/// Segment `i` laid out at its x and width; leading segments that do not
/// fit collapse into a leading ellipsis (segment 0 is always shown).
pub fn breadcrumb_layout(segments: &[&str], width: i32, out: &mut [(i32, i32); 16]) -> usize {
    let n = segments.len().min(16);
    let sep = 14;
    let w = |i: usize| measure13(segments[i], i + 1 == n) + 8;
    let total: i32 = (0..n).map(w).sum::<i32>() + sep * (n as i32 - 1).max(0);
    // Drop segments after the first until the rest fits.
    let mut skip = 0;
    let mut used = total;
    while used > width && skip + 2 < n {
        skip += 1;
        used -= w(skip) + sep;
    }
    let mut x = 0;
    for (i, slot) in out.iter_mut().enumerate().take(n) {
        if i >= 1 && i <= skip {
            *slot = (-1, 0);
            continue;
        }
        if i == 1 + skip && skip > 0 {
            x += measure13("\u{2026}", false) + sep;
        }
        *slot = (x, w(i));
        x += w(i) + sep;
    }
    n
}

pub fn breadcrumb(c: &mut Canvas<'_>, r: Rect, segments: &[&str], hover: Option<usize>, t: Theme) {
    let mut lay = [(0, 0); 16];
    let n = breadcrumb_layout(segments, r.width as i32, &mut lay);
    let mut prev_end = None;
    for (i, &(x, w)) in lay.iter().enumerate().take(n) {
        if x < 0 {
            continue;
        }
        let last = i + 1 == n;
        if let Some(e) = prev_end {
            if x - e > 16 {
                text_in(
                    c,
                    r.x + e + 4,
                    r.y,
                    r.height as i32,
                    12,
                    "\u{2026}",
                    t.muted,
                    false,
                );
            }
            c.bits_blit(
                &CHEVRON,
                3,
                r.x + x - 9,
                r.y + (r.height as i32 - 7) / 2,
                t.muted,
            );
        }
        if hover == Some(i) && !last {
            outlined(
                c,
                Rect {
                    x: r.x + x,
                    y: r.y + 2,
                    width: w as u32,
                    height: r.height.saturating_sub(4),
                },
                t.header,
                t.header,
                m::RADIUS,
            );
        }
        text_in(
            c,
            r.x + x + 4,
            r.y,
            r.height as i32,
            w,
            segments[i],
            if last { t.text } else { t.secondary },
            last,
        );
        prev_end = Some(x + w);
    }
}

pub fn breadcrumb_hit(segments: &[&str], width: i32, dx: i32) -> Option<usize> {
    let mut lay = [(0, 0); 16];
    let n = breadcrumb_layout(segments, width, &mut lay);
    (0..n).find(|&i| lay[i].0 >= 0 && dx >= lay[i].0 && dx < lay[i].0 + lay[i].1)
}

// ---- splitter and focus order ---------------------------------------------------------------

/// A vertical divider at `pos` between `min` and `max` (sidebar width).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Splitter {
    pub pos: i32,
    pub min: i32,
    pub max: i32,
}

impl Splitter {
    pub fn hit(&self, x: i32) -> bool {
        (x - self.pos).abs() <= 3
    }
    pub fn drag_to(&mut self, x: i32) {
        self.pos = x.clamp(self.min, self.max);
    }
    pub fn draw(&self, c: &mut Canvas<'_>, top: i32, height: i32, active: bool, t: Theme) {
        vline(
            c,
            self.pos,
            top,
            height,
            if active { t.accent } else { t.divider },
        );
    }
}

/// Tab order over `n` focusable parts: the next (or previous) one.
pub fn focus_next(current: usize, n: usize, back: bool) -> usize {
    if n == 0 {
        return 0;
    }
    if back {
        (current + n - 1) % n
    } else {
        (current + 1) % n
    }
}

/// A file-kind glyph for a list or grid row.
pub fn kind_glyph(folder: bool) -> Glyph {
    if folder {
        Glyph::Folder
    } else {
        Glyph::Document
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::palette;

    #[test]
    fn scroll_clamps_reveals_and_maps_the_thumb() {
        let mut s = Scroll {
            offset: 0,
            content: 1000,
            viewport: 200,
        };
        s.by(-50);
        assert_eq!(s.offset, 0);
        s.by(5000);
        assert_eq!(s.offset, 800);
        s.reveal(100, 20);
        assert_eq!(s.offset, 100);
        s.reveal(700, 20);
        assert_eq!(s.offset, 520);
        let track = Rect {
            x: 0,
            y: 0,
            width: 10,
            height: 200,
        };
        let t = s.thumb(track).unwrap();
        assert_eq!(t.height, 40);
        assert_eq!(s.hit(track, t.y - 1), Some(ScrollHit::PageUp));
        assert_eq!(s.hit(track, t.y + 1), Some(ScrollHit::Thumb));
        assert_eq!(s.drag(track, 0, 160), 800);
        assert_eq!(s.drag(track, 800, -1000), 0);
        let fits = Scroll {
            offset: 0,
            content: 100,
            viewport: 200,
        };
        assert_eq!(fits.thumb(track), None);
        assert_eq!(fits.hit(track, 5), None);
    }

    #[test]
    fn selection_click_shift_ctrl_and_steps() {
        let mut s = Selection::EMPTY;
        s.click(3, false, false);
        s.click(6, true, false);
        assert_eq!((s.count(), s.first()), (4, Some(3)));
        s.click(4, false, true);
        assert!(!s.contains(4) && s.count() == 3);
        s.click(9, false, false);
        assert_eq!(s.count(), 1);
        s.step(-1, 10, true);
        s.step(-1, 10, true);
        assert_eq!((s.count(), s.first()), (3, Some(7)));
        s.step(5, 10, false);
        assert_eq!((s.count(), s.cursor), (1, Some(9)));
        s.select_all(5);
        assert_eq!(s.count(), 6);
        s.truncate(5);
        assert_eq!((s.count(), s.cursor), (5, Some(4)));
        assert!(!s.contains(Selection::MAX));
    }

    #[test]
    fn list_is_virtualized_hit_tested_and_revealing() {
        let area = Rect {
            x: 10,
            y: 20,
            width: 300,
            height: HEADER as u32 + 5 * ROW as u32,
        };
        let cols = [
            Column {
                title: "Name",
                width: 180,
                right: false,
            },
            Column {
                title: "Size",
                width: 120,
                right: true,
            },
        ];
        let mut l = List::new();
        l.layout(area, 200);
        assert_eq!(l.visible(), (0, 5));
        l.reveal(50);
        assert_eq!(l.visible(), (46, 51));
        assert_eq!(l.hit(area, cols.as_slice(), 20, 25), ListHit::Header(0));
        assert_eq!(l.hit(area, &cols, 250, 25), ListHit::Header(1));
        assert_eq!(l.hit(area, &cols, 20, 20 + HEADER + 1), ListHit::Row(46));
        assert!(matches!(
            l.hit(area, &cols, 305, 20 + HEADER + 50),
            ListHit::Scrollbar(_)
        ));
        let mut px = [0u32; 320 * 160];
        let mut c = Canvas::new(&mut px, 320, 160, 320).unwrap();
        let mut asked = 0;
        let names = ["alpha", "beta"];
        let drawn = l.draw(
            &mut c,
            area,
            &cols,
            &Selection::EMPTY,
            true,
            |r, _| {
                asked += 1;
                names[r % 2]
            },
            |_| None,
            palette(false),
        );
        assert_eq!(drawn, 5);
        assert_eq!(asked, 10);
        let mut short = List::new();
        short.layout(area, 2);
        assert_eq!(
            short.hit(area, &cols, 20, 20 + HEADER + 3 * ROW),
            ListHit::Blank
        );
    }

    #[test]
    fn grid_columns_cells_and_hits() {
        let area = Rect {
            x: 0,
            y: 0,
            width: 4 * CELL_W as u32 + BAR as u32,
            height: 2 * CELL_H as u32,
        };
        let mut g = Grid::default();
        g.layout(area, 10);
        assert_eq!(Grid::columns(area), 4);
        assert_eq!(g.cell(area, 5).x, CELL_W);
        assert_eq!(g.hit(area, CELL_W + 1, CELL_H + 1), Some(5));
        assert_eq!(g.hit(area, 3 * CELL_W + 1, 2 * CELL_H - 1), Some(7));
        g.reveal(area, 9);
        assert_eq!(g.scroll.offset, CELL_H);
        assert_eq!(g.hit(area, 2 * CELL_W - 1, CELL_H + 1), Some(9));
        assert_eq!(g.hit(area, 3 * CELL_W + 1, CELL_H + 1), None);
    }

    #[test]
    fn input_edits_with_selection_and_bounds() {
        let mut i = Input::EMPTY;
        i.set(b"report.txt");
        i.select(0, 6);
        assert!(i.key(b'n' as u16, false));
        assert_eq!(i.text(), "n.txt");
        i.key(keys::HOME, false);
        i.key(keys::RIGHT, true);
        i.key(keys::RIGHT, true);
        assert_eq!(i.selection(), Some((0, 2)));
        i.key(keys::BACKSPACE, false);
        assert_eq!(i.text(), "txt");
        i.key(keys::END, false);
        i.key(keys::DELETE, false);
        assert_eq!(i.text(), "txt");
        i.select_all();
        i.key(keys::LEFT, false);
        assert_eq!((i.cursor, i.selection()), (0, None));
        let mut full = Input::EMPTY;
        full.set(&[b'a'; INPUT_MAX]);
        assert!(!full.key(b'b' as u16, false));
        assert_eq!(full.len, INPUT_MAX);
        let mut j = Input::EMPTY;
        j.set(b"mmmm");
        let w = measure13("mm", false);
        assert_eq!(j.index_at(w + 1), 2);
        assert_eq!(j.index_at(-5), 0);
    }

    #[test]
    fn breadcrumb_collapses_and_hits() {
        let segs = ["Home", "Documents", "Projects", "2026", "Reports"];
        let mut lay = [(0, 0); 16];
        let n = breadcrumb_layout(&segs, 1000, &mut lay);
        assert_eq!(n, 5);
        assert!(lay.iter().take(5).all(|(x, _)| *x >= 0));
        assert_eq!(breadcrumb_hit(&segs, 1000, lay[2].0 + 2), Some(2));
        let n = breadcrumb_layout(&segs, 150, &mut lay);
        assert_eq!(n, 5);
        assert!(lay[0].0 == 0 && lay[4].0 > 0 && lay[1].0 < 0);
        assert!(lay[4].0 + lay[4].1 <= 150 || lay[3].0 >= 0);
        assert_eq!(breadcrumb_hit(&segs, 150, lay[4].0 + 1), Some(4));
    }

    #[test]
    fn splitter_and_focus_order() {
        let mut s = Splitter {
            pos: 150,
            min: 100,
            max: 300,
        };
        assert!(s.hit(152) && !s.hit(160));
        s.drag_to(20);
        assert_eq!(s.pos, 100);
        assert_eq!(focus_next(2, 3, false), 0);
        assert_eq!(focus_next(0, 3, true), 2);
        assert_eq!(focus_next(0, 0, false), 0);
    }

    #[test]
    fn text_in_ellipsizes_inside_its_width() {
        let mut px = [0u32; 100 * 20];
        let mut c = Canvas::new(&mut px, 100, 20, 100).unwrap();
        let w = text_in(&mut c, 0, 0, 20, 60, "A very long file name.txt", 1, false);
        assert!(w <= 60, "{w}");
        assert!(px.iter().enumerate().all(|(i, p)| *p == 0 || i % 100 < 60));
    }
}
