//! Retained application view with keyed repaint bands (Phase 11.1).
//!
//! `View` is everything an application window paints. `View::paint` draws
//! it completely; under a gfxkit clip it draws exactly the clipped part of
//! that same picture. `View::bands` splits the window into full-width
//! horizontal bands and gives each a key: a hash of exactly the state whose
//! pixels can land inside that band. Comparing the previous frame's bands
//! with the next frame's yields the dirty rectangles; painting the next view
//! under each dirty rectangle over the previous raster reproduces a full
//! redraw pixel for pixel (proved by the randomized tests below). A key that
//! misses a dependency (the old caret row, a newly exposed scrolled line)
//! shows up as a pixel difference, never as a silent stale raster.
use super::{
    layout::Layout,
    model::{Editor, Line, Terminal},
    view,
};
use arena_gfxkit::{Canvas, Rect};
use arena_ui::{components as c, metrics as m, theme};

/// Upper bound on bands in any layout (terminal: header, rows, filler,
/// input, status; editor: header, top, rows, tail, status) at the tallest
/// surface (768 px: at most 48 text rows).
pub const MAX_BANDS: usize = 56;
/// Editor rows at the tallest surface.
const MAX_EDIT_ROWS: usize = 50;
/// Rectangles a client publishes per Damage request (wire bound).
pub const MAX_DIRTY: usize = 5;

/// Everything an application window paints, borrowed from its model.
#[derive(Clone, Copy)]
pub struct View<'a> {
    pub kind: u8,
    /// Bit 0: dark palette; bit 1: motion (Settings shows it).
    pub appearance: u8,
    pub status: &'a str,
    pub terminal: &'a Terminal,
    pub editor: &'a Editor,
    pub line: &'a Line,
    pub top: usize,
    pub dialog: u8,
    pub names: &'a [[u8; 32]],
    pub selected: usize,
    pub preview: &'a [u8],
    pub display: (u16, u16),
    pub counts: &'a [u64; 9],
    pub processes: &'a [(u64, u64)],
    pub gallery_theme: u8,
    /// Surface size the view is laid out for (Phase 11.3).
    pub size: (u16, u16),
}

impl View<'_> {
    /// Draw the whole window (only its clipped part under a canvas clip).
    pub fn paint(&self, canvas: &mut Canvas<'_>) {
        let dark = self.appearance & 1 != 0;
        let t = theme::palette(dark);
        view::frame(canvas, super::TITLES[self.kind as usize], self.status, t);
        match self.kind {
            super::TERMINAL => view::terminal(canvas, self.terminal, self.top, t),
            super::EDITOR => view::editor(canvas, self.editor, self.top, self.line, self.dialog, t),
            super::FILES => view::files(
                canvas,
                view::FilesView {
                    names: self.names,
                    top: self.top,
                    selected: self.selected,
                    bytes: self.preview,
                    line: self.line,
                    dialogue: self.dialog != 0,
                },
                t,
            ),
            super::SETTINGS => {
                view::settings(canvas, dark, self.appearance & 2 != 0, self.display, t)
            }
            super::MONITOR => view::monitor(canvas, self.counts, self.processes, self.top, t),
            _ => c::gallery(
                canvas,
                if self.gallery_theme < 2 {
                    theme::palette(self.gallery_theme == 1)
                } else {
                    t
                },
            ),
        }
    }

    /// Keyed bands covering the window exactly once, top to bottom.
    pub fn bands(&self, out: &mut Bands) {
        let l = Layout::new(self.size.0, self.size.1);
        out.n = 0;
        out.w = l.W;
        out.h = l.H;
        let base = {
            let mut h = Fnv::new();
            h.u64(u64::from(self.kind)).u64(u64::from(self.appearance));
            h
        };
        let header = l.HEADER_BOTTOM + 1;
        let mut head = base;
        let mut status = base;
        status.bytes(self.status.as_bytes());
        match self.kind {
            super::TERMINAL => {
                let t = self.terminal;
                head.u64(t.count as u64).u64(self.top as u64);
                out.push(0, header, head.0);
                let start = t
                    .count
                    .saturating_sub(l.TERMINAL_ROWS)
                    .saturating_sub(self.top);
                let mut y = header;
                for row in 0..l.TERMINAL_ROWS {
                    let end = (l.CONTENT_Y + 3 + (row as i32 + 1) * m::LINE_HEIGHT).min(l.INPUT_Y);
                    let mut k = base;
                    k.u64(row as u64);
                    let index = start + row;
                    if index < t.count {
                        k.u64(1).bytes(&t.lines[index][..t.sizes[index]]);
                    }
                    out.push(y, end, k.0);
                    y = end;
                }
                if y < l.INPUT_Y {
                    out.push(y, l.INPUT_Y, base.0 ^ 0x51);
                }
                let mut input = base;
                input
                    .bytes(&t.input[..t.len])
                    .u64(t.len as u64)
                    .u64(t.cursor as u64);
                out.push(l.INPUT_Y, l.STATUS_Y, input.0);
            }
            super::EDITOR => {
                let e = self.editor;
                head.u64(u64::from(self.dialog))
                    .bytes(&e.path)
                    .u64(u64::from(e.dirty));
                // The header shows line:column of the caret.
                head.bytes(&e.data[..e.cursor]);
                if self.dialog != 0 {
                    head.bytes(&self.line.bytes[..self.line.len])
                        .u64(self.line.cursor as u64);
                }
                out.push(0, header, head.0);
                let first = l.EDIT_TEXT.y + 3;
                out.push(header, first, base.0 ^ 0xE0);
                let mut all_rows = [base; MAX_EDIT_ROWS];
                let rows = &mut all_rows[..l.EDIT_ROWS.min(MAX_EDIT_ROWS)];
                for (r, k) in rows.iter_mut().enumerate() {
                    k.u64(r as u64);
                }
                let caret_row = l.visual_row(e);
                if caret_row >= self.top && caret_row < self.top + l.EDIT_ROWS {
                    rows[caret_row - self.top].u64(0xCA);
                }
                // Walk the buffer exactly as the painter does.
                let (mut row, mut column) = (0usize, 0usize);
                for i in 0..=e.len {
                    if i == e.cursor && row >= self.top && row < self.top + l.EDIT_ROWS {
                        rows[row - self.top].u64(0xC0).u64(column as u64);
                    }
                    if i == e.len {
                        break;
                    }
                    let b = e.data[i];
                    if b == b'\n' {
                        row += 1;
                        column = 0;
                        continue;
                    }
                    if row >= self.top && row < self.top + l.EDIT_ROWS && b != b'\t' {
                        rows[row - self.top].u64(column as u64).bytes(&[b]);
                    }
                    column += if b == b'\t' { 4 - column % 4 } else { 1 };
                    if column >= l.EDIT_COLUMNS {
                        row += 1;
                        column = 0;
                    }
                }
                if e.len == 0 {
                    rows[0].u64(0xE7);
                }
                for (r, k) in rows.iter().enumerate() {
                    let y = first + r as i32 * m::LINE_HEIGHT;
                    out.push(y, y + m::LINE_HEIGHT, k.0);
                }
                let tail = first + l.EDIT_ROWS as i32 * m::LINE_HEIGHT;
                if tail < l.STATUS_Y {
                    out.push(tail, l.STATUS_Y, base.0 ^ 0xE1);
                }
            }
            _ => {
                // Files, Settings, Monitor, Gallery: one content band whose
                // key covers all of their (small) content state.
                let mut all = head;
                all.u64(self.top as u64)
                    .u64(self.selected as u64)
                    .u64(u64::from(self.dialog))
                    .u64(u64::from(self.gallery_theme))
                    .u64(u64::from(self.display.0))
                    .u64(u64::from(self.display.1))
                    .bytes(self.preview)
                    .bytes(&self.line.bytes[..self.line.len])
                    .u64(self.line.cursor as u64)
                    .u64(self.names.len() as u64);
                for n in self.names {
                    all.bytes(n);
                }
                for v in self.counts {
                    all.u64(*v);
                }
                for (a, b) in self.processes {
                    all.u64(*a).u64(*b);
                }
                out.push(0, header, all.0);
                out.push(header, l.STATUS_Y, all.0 ^ 0xC7);
            }
        }
        out.push(l.STATUS_Y, l.H, status.0);
    }
}

/// FNV-1a, 64 bit: band keys (a collision only costs a missed repaint of
/// one band; 2^-64 per comparison).
#[derive(Clone, Copy)]
pub struct Fnv(pub u64);
impl Fnv {
    pub const fn new() -> Self {
        Self(0xcbf2_9ce4_8422_2325)
    }
    pub fn bytes(&mut self, b: &[u8]) -> &mut Self {
        for x in b {
            self.0 = (self.0 ^ u64::from(*x)).wrapping_mul(0x0000_0100_0000_01b3);
        }
        self
    }
    pub fn u64(&mut self, v: u64) -> &mut Self {
        self.bytes(&v.to_le_bytes())
    }
}
impl Default for Fnv {
    fn default() -> Self {
        Self::new()
    }
}

/// Bands of one frame: `[y0, y1)` full-width rows with their keys.
#[derive(Clone, Copy)]
pub struct Bands {
    pub y: [(i32, i32); MAX_BANDS],
    pub keys: [u64; MAX_BANDS],
    pub n: usize,
    /// Surface size the bands cover.
    pub w: i32,
    pub h: i32,
}
impl Bands {
    pub const EMPTY: Self = Self {
        y: [(0, 0); MAX_BANDS],
        keys: [0; MAX_BANDS],
        n: 0,
        w: 0,
        h: 0,
    };
    fn push(&mut self, y0: i32, y1: i32, key: u64) {
        if y1 > y0 && self.n < MAX_BANDS {
            self.y[self.n] = (y0, y1);
            self.keys[self.n] = key;
            self.n += 1;
        }
    }
}

/// Dirty rectangles for one repaint, at most `MAX_DIRTY`, never overlapping.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Dirty {
    pub rects: [Rect; MAX_DIRTY],
    pub n: usize,
    full: bool,
}
impl Dirty {
    pub const NONE: Self = Self {
        rects: [Rect {
            x: 0,
            y: 0,
            width: 0,
            height: 0,
        }; MAX_DIRTY],
        n: 0,
        full: false,
    };
    /// The whole `w` x `h` surface.
    pub const fn full(w: i32, h: i32) -> Self {
        let mut d = Self::NONE;
        d.rects[0] = Rect {
            x: 0,
            y: 0,
            width: w as u32,
            height: h as u32,
        };
        d.n = 1;
        d.full = true;
        d
    }
    pub fn rects(&self) -> &[Rect] {
        &self.rects[..self.n]
    }
    /// One rectangle covering the whole surface.
    pub fn is_full(&self) -> bool {
        self.full
    }
}

/// Bands whose key changed, as full-width rectangles: adjacent dirty bands
/// are merged into one rectangle; when more than `MAX_DIRTY` runs remain,
/// the two runs separated by the smallest clean gap are joined (the gap is
/// repainted too, which is always correct), until the list fits. A layout
/// change (different band boundaries) repaints everything.
pub fn dirty(prev: &Bands, next: &Bands) -> Dirty {
    if (prev.w, prev.h) != (next.w, next.h)
        || prev.n != next.n
        || prev.y[..prev.n] != next.y[..next.n]
    {
        return Dirty::full(next.w, next.h);
    }
    let mut runs = [(0i32, 0i32); MAX_BANDS];
    let mut count = 0;
    for i in 0..next.n {
        if prev.keys[i] == next.keys[i] {
            continue;
        }
        let (y0, y1) = next.y[i];
        if count > 0 && runs[count - 1].1 == y0 {
            runs[count - 1].1 = y1;
        } else {
            runs[count] = (y0, y1);
            count += 1;
        }
    }
    while count > MAX_DIRTY {
        let mut best = 1;
        for i in 1..count {
            if runs[i].0 - runs[i - 1].1 < runs[best].0 - runs[best - 1].1 {
                best = i;
            }
        }
        runs[best - 1].1 = runs[best].1;
        for i in best..count - 1 {
            runs[i] = runs[i + 1];
        }
        count -= 1;
    }
    let mut d = Dirty::NONE;
    for (i, (y0, y1)) in runs[..count].iter().enumerate() {
        d.rects[i] = Rect {
            x: 0,
            y: *y0,
            width: next.w as u32,
            height: (y1 - y0) as u32,
        };
    }
    d.n = count;
    // Every band dirty is one rectangle covering the surface: publish it
    // as a whole-surface Damage.
    d.full = count == 1 && d.rects[0].y == 0 && d.rects[0].height as i32 == next.h;
    d
}

/// Repaint `next` into `canvas` (holding the previous frame) under each
/// dirty rectangle; the result equals a full paint of `next`.
pub fn repaint(canvas: &mut Canvas<'_>, next: &View<'_>, dirty: &Dirty) {
    for r in dirty.rects() {
        canvas.set_clip(*r);
        next.paint(canvas);
    }
    canvas.reset_clip();
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use crate::apps::{EDITOR, TERMINAL};
    use std::vec;
    use std::vec::Vec;

    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
        fn below(&mut self, n: u64) -> u64 {
            self.next() % n
        }
    }

    struct Fixture {
        terminal: Terminal,
        editor: Editor,
        line: Line,
        names: Vec<[u8; 32]>,
        counts: [u64; 9],
        processes: Vec<(u64, u64)>,
        status: &'static str,
        top: usize,
        appearance: u8,
        kind: u8,
        size: (u16, u16),
    }
    const DEFAULT: (u16, u16) = (448, 288);
    impl Fixture {
        fn new(kind: u8) -> Self {
            Self {
                terminal: Terminal::new(),
                editor: Editor::new(),
                line: Line::new(),
                names: vec![],
                counts: [0; 9],
                processes: vec![],
                status: "READY",
                top: 0,
                appearance: 0,
                kind,
                size: DEFAULT,
            }
        }
        fn view(&self) -> View<'_> {
            View {
                kind: self.kind,
                appearance: self.appearance,
                status: self.status,
                terminal: &self.terminal,
                editor: &self.editor,
                line: &self.line,
                top: self.top,
                dialog: 0,
                names: &self.names,
                selected: 0,
                preview: &[],
                display: (800, 600),
                counts: &self.counts,
                processes: &self.processes,
                gallery_theme: 2,
                size: self.size,
            }
        }
    }

    fn full(v: &View<'_>) -> Vec<u32> {
        let (w, h) = (usize::from(v.size.0), usize::from(v.size.1));
        let mut px = vec![0u32; w * h];
        let mut c = Canvas::new(&mut px, w, h, w).unwrap();
        v.paint(&mut c);
        px
    }

    /// Drive `steps` random model edits; after each, the incremental raster
    /// (previous frame + repaint under the dirty rectangles) must equal a
    /// full paint, and must have repainted strictly less than the window
    /// for ordinary edits.
    fn check(
        kind: u8,
        seed: u64,
        steps: usize,
        edit: fn(&mut Fixture, &mut Rng),
    ) -> (usize, usize) {
        check_at(kind, seed, steps, edit, DEFAULT)
    }

    fn check_at(
        kind: u8,
        seed: u64,
        steps: usize,
        edit: fn(&mut Fixture, &mut Rng),
        size: (u16, u16),
    ) -> (usize, usize) {
        let (w, h) = (usize::from(size.0), usize::from(size.1));
        let mut rng = Rng(seed);
        let mut fx = Fixture::new(kind);
        fx.size = size;
        let mut shown = full(&fx.view());
        let mut bands = Bands::EMPTY;
        fx.view().bands(&mut bands);
        let (mut partial, mut total) = (0, 0);
        for step in 0..steps {
            edit(&mut fx, &mut rng);
            let mut next = Bands::EMPTY;
            fx.view().bands(&mut next);
            let d = dirty(&bands, &next);
            assert!(d.n <= MAX_DIRTY);
            {
                let mut c = Canvas::new(&mut shown, w, h, w).unwrap();
                repaint(&mut c, &fx.view(), &d);
            }
            assert!(
                shown == full(&fx.view()),
                "kind {kind} step {step}: partial != full"
            );
            total += 1;
            if !d.is_full() {
                partial += 1;
            }
            bands = next;
        }
        (partial, total)
    }

    fn terminal_edit(fx: &mut Fixture, rng: &mut Rng) {
        match rng.below(10) {
            0..=4 => {
                let key = [b'a', b'z', b' ', b'7', b'-'][rng.below(5) as usize];
                fx.terminal.key(u16::from(key));
            }
            5 => {
                fx.terminal.key(8); // backspace
            }
            6 => {
                // Enter: the line moves into the transcript.
                let (line, n) = fx.terminal.consume();
                let mut out = Vec::from(&line[..n]);
                out.push(b'\n');
                fx.terminal.write(&out);
            }
            7 => fx.top = rng.below(4) as usize, // scrollback offset
            8 => fx.status = ["READY", "REFUSED: FILE NOT FOUND", "SAVED"][rng.below(3) as usize],
            _ => fx.appearance = rng.below(4) as u8,
        }
    }

    fn editor_edit(fx: &mut Fixture, rng: &mut Rng) {
        match rng.below(12) {
            0..=4 => {
                let b = [b'a', b'Q', b' ', b'\n', b'\t', b'9'][rng.below(6) as usize];
                let _ = fx.editor.insert(b);
            }
            5 => fx.editor.backspace(),
            6 => fx.editor.left(),
            7 => fx.editor.right(),
            8 => fx.editor.vertical(rng.below(2) == 0),
            9 => fx.editor.home(),
            10 => fx.top = rng.below(3) as usize,
            _ => fx.appearance ^= 1,
        }
    }

    #[test]
    fn terminal_partial_repaint_equals_full_redraw() {
        for seed in [1, 7, 0x5eed] {
            let (partial, total) = check(TERMINAL, seed, 120, terminal_edit);
            assert!(
                partial * 2 > total,
                "too few partial repaints {partial}/{total}"
            );
        }
    }

    #[test]
    fn editor_partial_repaint_equals_full_redraw() {
        for seed in [3, 11, 0xed17] {
            let (partial, total) = check(EDITOR, seed, 160, editor_edit);
            assert!(
                partial * 2 > total,
                "too few partial repaints {partial}/{total}"
            );
        }
    }

    /// The same equivalence at other surface sizes (resized windows),
    /// and a size change always repaints and publishes the whole surface.
    #[test]
    fn partial_repaint_equals_full_redraw_at_every_size() {
        for size in [(400, 200), (712, 470), (1024, 768)] {
            let (p, t) = check_at(TERMINAL, 99, 60, terminal_edit, size);
            assert!(p * 2 > t, "{size:?} {p}/{t}");
            let (p, t) = check_at(EDITOR, 98, 60, editor_edit, size);
            assert!(p * 2 > t, "{size:?} {p}/{t}");
        }
        let mut fx = Fixture::new(EDITOR);
        let mut a = Bands::EMPTY;
        fx.view().bands(&mut a);
        fx.size = (600, 400);
        let mut b = Bands::EMPTY;
        fx.view().bands(&mut b);
        let d = dirty(&a, &b);
        assert!(d.is_full());
        assert_eq!((d.rects[0].width, d.rects[0].height), (600, 400));
    }

    #[test]
    fn terminal_typing_repaints_only_the_input_line() {
        let mut fx = Fixture::new(TERMINAL);
        let mut a = Bands::EMPTY;
        fx.view().bands(&mut a);
        fx.terminal.key(u16::from(b'x'));
        let mut b = Bands::EMPTY;
        fx.view().bands(&mut b);
        let d = dirty(&a, &b);
        assert_eq!(d.n, 1);
        let l = Layout::DEFAULT;
        assert_eq!(d.rects[0].y, l.INPUT_Y);
        assert_eq!(d.rects[0].height as i32, l.STATUS_Y - l.INPUT_Y);
    }

    #[test]
    fn editor_caret_motion_repaints_old_and_new_rows_only() {
        let mut fx = Fixture::new(EDITOR);
        for b in b"one\ntwo\nthree" {
            fx.editor.insert(*b).unwrap();
        }
        let mut a = Bands::EMPTY;
        fx.view().bands(&mut a);
        fx.editor.vertical(false); // up one visual row
        let mut b = Bands::EMPTY;
        fx.view().bands(&mut b);
        let d = dirty(&a, &b);
        let rows: u32 = d.rects().iter().map(|r| r.height).sum();
        // Header (caret line:column) + two text rows, nothing else.
        let l = Layout::DEFAULT;
        assert!(
            rows <= (l.HEADER_BOTTOM + 1) as u32 + 2 * m::LINE_HEIGHT as u32,
            "{d:?}"
        );
        assert!(d.rects().iter().any(|r| r.y > l.HEADER_BOTTOM));
    }

    #[test]
    fn dirty_runs_merge_and_stay_bounded() {
        let mut a = Bands::EMPTY;
        let mut b = Bands::EMPTY;
        for i in 0..20 {
            a.push(i * 10, i * 10 + 10, i as u64);
            b.push(i * 10, i * 10 + 10, if i % 2 == 0 { 99 } else { i as u64 });
        }
        let d = dirty(&a, &b);
        assert_eq!(d.n, MAX_DIRTY);
        // Every changed band is still covered.
        for i in (0..20).step_by(2) {
            let y = i * 10;
            assert!(
                d.rects()
                    .iter()
                    .any(|r| r.y <= y && y + 10 <= r.y + r.height as i32)
            );
        }
        // Adjacent changed bands collapse into one rectangle.
        let mut c = Bands::EMPTY;
        for i in 0..20 {
            c.push(
                i * 10,
                i * 10 + 10,
                if (3..9).contains(&i) { 77 } else { i as u64 },
            );
        }
        let d = dirty(&a, &c);
        assert_eq!(d.n, 1);
        assert_eq!((d.rects[0].y, d.rects[0].height), (30, 60));
    }
}
