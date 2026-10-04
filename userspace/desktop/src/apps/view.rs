//! Presentation-only application views. No service calls or capabilities.
//!
//! Every application shares one anatomy (see layout.rs): compositor title
//! bar, a header band for tools or page identity, content, and a status
//! band. Colours come only from the theme; geometry only from layout.rs.
use super::{
    layout::Layout,
    model::{Editor, Line, Terminal},
};
use arena_gfxkit::{Canvas, Rect};
use arena_ui::{
    components::{self as c, Glyph, Kind, State, Style, Tone},
    metrics as m,
    theme::Theme,
};
pub fn string(bytes: &[u8]) -> &str {
    core::str::from_utf8(bytes).unwrap_or("Unsupported text")
}

/// Presentation tone of a controller status line. The controllers own the
/// words; this only chooses how loudly to say them (glyph + colour).
pub fn tone(status: &str) -> Tone {
    const ERROR: [&str; 6] = [
        "REFUSED",
        "FILE NOT FOUND",
        "FILE ALREADY EXISTS",
        "FILESYSTEM ERROR",
        "SERVICE ",
        "Unsupported",
    ];
    const WARNING: [&str; 4] = ["MODIFIED", "UNSAVED", "SAVE FIRST", "Unknown command"];
    const SUCCESS: [&str; 7] = [
        "SAVED",
        "CREATED",
        "DELETED",
        "APPEARANCE COMMITTED",
        "MOTION PREFERENCE COMMITTED",
        "OPENED",
        "REFRESHED",
    ];
    if ERROR.iter().any(|p| status.starts_with(p)) {
        Tone::Error
    } else if WARNING.iter().any(|p| status.starts_with(p)) {
        Tone::Warning
    } else if SUCCESS.iter().any(|p| status.starts_with(p)) {
        Tone::Success
    } else {
        Tone::Neutral
    }
}

pub fn frame(canvas: &mut Canvas<'_>, title: &str, status: &str, t: Theme) {
    let l = Layout::of(canvas);
    canvas.clear(t.elevated);
    c::rect(
        canvas,
        1,
        m::TITLE_HEIGHT,
        l.W - 2,
        l.HEADER_BOTTOM - m::TITLE_HEIGHT,
        t.header,
    );
    c::hline(canvas, 1, l.HEADER_BOTTOM, l.W - 2, t.divider);
    c::status_band(canvas, l.STATUS_Y, status, tone(status), t);
    c::chrome(canvas, title, true, t);
}

/// Page header used by applications without a toolbar. Its two lines end
/// at y54 and the band stays plain down to the divider: window-local rows
/// 56..65 are where a cascaded window's title strip lands, and the guest
/// oracles read a plain strip there as "no new window".
fn page_header(canvas: &mut Canvas<'_>, title: &str, detail: &str, t: Theme) {
    c::text(canvas, m::CONTENT_INSET, 35, title, Style::Strong, t.text);
    c::text_fit(
        canvas,
        m::CONTENT_INSET,
        48,
        detail,
        Style::Body,
        t.secondary,
        300,
    );
}

pub fn terminal(canvas: &mut Canvas<'_>, model: &Terminal, older: usize, t: Theme) {
    let l = Layout::of(canvas);
    page_header(
        canvas,
        "Arena session",
        "help ls cat ps put rm launch clear echo",
        t,
    );
    // Scrollback position: real line count, capacity and history offset.
    let mut n = [0u8; 32];
    let count = c::decimal(&mut n, model.count as u64, false);
    let right = l.W - m::CONTENT_INSET;
    let x = right - c::measure("/32 LINES", Style::Caption);
    c::text(canvas, x, 35, "/32 LINES", Style::Caption, t.muted);
    c::text_right(canvas, x, 35, count, Style::Caption, t.secondary);
    if older > 0 {
        // History view: how many lines above the live tail are shown.
        let mut b = [0u8; 32];
        let mut label = [0u8; 32];
        let back = c::decimal(&mut b, older as u64, false);
        label[..5].copy_from_slice(b"BACK ");
        label[5..5 + back.len()].copy_from_slice(back.as_bytes());
        let label = string(&label[..5 + back.len()]);
        let w = c::measure(label, Style::Caption) + 2 * m::M;
        c::chip(canvas, right - w, 46, label, Tone::Accent, t);
    }
    // Console fills the content area edge to edge.
    let top = l.HEADER_BOTTOM + 1;
    c::rect(canvas, 1, top, l.W - 2, l.STATUS_Y - top, t.terminal);
    let visible = l.TERMINAL_ROWS;
    let start = model.count.saturating_sub(visible).saturating_sub(older);
    for (row, index) in (start..model.count).take(visible).enumerate() {
        let text = string(&model.lines[index][..model.sizes[index]]);
        let ink = match tone(text) {
            Tone::Error => t.error,
            Tone::Warning => t.warning,
            _ => t.terminal_text,
        };
        c::text(
            canvas,
            m::CONTENT_INSET + 6,
            l.CONTENT_Y + 6 + row as i32 * m::LINE_HEIGHT,
            text,
            Style::Body,
            ink,
        );
    }
    // Input line: hairline, Signal chevron prompt, block caret.
    c::hline(canvas, 1, l.INPUT_Y, l.W - 2, t.terminal_edge);
    let y = l.INPUT_Y + (l.STATUS_Y - l.INPUT_Y - m::FONT_HEIGHT) / 2;
    c::glyph(
        canvas,
        m::CONTENT_INSET,
        y - 1,
        Glyph::Chevron,
        t.terminal_prompt,
    );
    let x = m::CONTENT_INSET + 14;
    c::text(
        canvas,
        x,
        y,
        string(&model.input[..model.len]),
        Style::Body,
        t.terminal_text,
    );
    let cx = x + model.cursor as i32 * m::FONT_ADVANCE - 1;
    c::rect(
        canvas,
        cx,
        y - 2,
        m::FONT_ADVANCE + 1,
        m::FONT_HEIGHT + 4,
        t.terminal_prompt,
    );
    if model.cursor < model.len {
        let glyph = [model.input[model.cursor]];
        c::text(canvas, cx + 1, y, string(&glyph), Style::Body, t.terminal);
    }
}

pub fn dialog(canvas: &mut Canvas<'_>, line: &Line, label: &str, t: Theme) {
    let l = Layout::of(canvas);
    c::field(
        canvas,
        l.NAME_FIELD,
        string(&line.bytes[..line.len]),
        Some(line.cursor),
        State::Focused,
        t,
    );
    c::button(
        canvas,
        l.PRIMARY,
        label,
        None,
        Kind::Primary,
        State::Normal,
        t,
    );
    c::button(
        canvas,
        l.SECONDARY,
        "Cancel",
        None,
        Kind::Standard,
        State::Normal,
        t,
    );
}

/// Document facts in the editor header: name, saved state, caret position.
///
/// Only the name (which changes solely through the Open/Save As dialogs)
/// sits inside the toolbar strip x12..366. Saved state and caret position
/// are right-aligned beyond it, because guest oracles treat a change in
/// that strip as "a dialog is now open" before typing into it.
fn document_info(canvas: &mut Canvas<'_>, model: &Editor, t: Theme) {
    let l = Layout::of(canvas);
    let right = l.W - m::CONTENT_INSET;
    let end = model.path.iter().position(|b| *b == 0).unwrap_or(32);
    let name = if end == 0 {
        "Untitled"
    } else {
        string(&model.path[..end])
    };
    // Fixed reservation so caret movement never re-truncates the name.
    let position_w = 7 * m::FONT_ADVANCE;
    let x = l.HEADER_INFO_X;
    c::text_fit(
        canvas,
        x,
        36,
        name,
        Style::Strong,
        t.text,
        right - position_w - m::S - x,
    );
    let state = if model.dirty {
        ("Unsaved", Tone::Warning)
    } else if end == 0 {
        ("New", Tone::Neutral)
    } else {
        ("On disk", Tone::Neutral)
    };
    let chip_w = c::measure(state.0, Style::Caption) + 2 * m::M;
    c::chip(canvas, right - chip_w, 47, state.0, state.1, t);
    // Line:column of the caret, counted from the real buffer.
    let (mut line, mut column) = (1u64, 1u64);
    for b in &model.data[..model.cursor] {
        if *b == b'\n' {
            line += 1;
            column = 1;
        } else {
            column += 1;
        }
    }
    let mut buffer = [0u8; 32];
    let mut text = [0u8; 32];
    let a = c::decimal(&mut buffer, line, false);
    let mut n = a.len();
    text[..n].copy_from_slice(a.as_bytes());
    text[n] = b':';
    n += 1;
    let b = c::decimal(&mut buffer, column, false);
    text[n..n + b.len()].copy_from_slice(b.as_bytes());
    n += b.len();
    c::text_right(canvas, right, 36, string(&text[..n]), Style::Body, t.muted);
}

pub fn editor(
    canvas: &mut Canvas<'_>,
    model: &Editor,
    top: usize,
    line: &Line,
    dialogue: u8,
    t: Theme,
) {
    let l = Layout::of(canvas);
    if dialogue == 4 {
        c::button(
            canvas,
            l.NAME_FIELD,
            "Save changes",
            Some(Glyph::Save),
            Kind::Primary,
            State::Normal,
            t,
        );
        c::button(
            canvas,
            l.PRIMARY,
            "Discard",
            Some(Glyph::Delete),
            Kind::Destructive,
            State::Normal,
            t,
        );
        c::button(
            canvas,
            l.SECONDARY,
            "Cancel",
            None,
            Kind::Standard,
            State::Normal,
            t,
        );
    } else if dialogue != 0 {
        dialog(canvas, line, if dialogue == 2 { "Open" } else { "Save" }, t);
    } else {
        c::button(
            canvas,
            l.NEW,
            "New",
            Some(Glyph::Plus),
            Kind::Standard,
            State::Normal,
            t,
        );
        c::button(
            canvas,
            l.SAVE,
            "Save",
            Some(Glyph::Save),
            Kind::Primary,
            State::Normal,
            t,
        );
        c::button(
            canvas,
            l.SAVE_AS,
            "Save As",
            Some(Glyph::Rename),
            Kind::Standard,
            State::Normal,
            t,
        );
        c::button(
            canvas,
            l.OPEN,
            "Open",
            Some(Glyph::Open),
            Kind::Standard,
            State::Normal,
            t,
        );
        document_info(canvas, model, t);
    }
    // Full-bleed text canvas with the caret's line softly highlighted.
    let caret_row = l.visual_row(model);
    if caret_row >= top && caret_row < top + l.EDIT_ROWS {
        c::rect(
            canvas,
            1,
            l.EDIT_TEXT.y + 3 + (caret_row - top) as i32 * m::LINE_HEIGHT,
            l.W - 2,
            m::LINE_HEIGHT,
            t.editor_line,
        );
        c::rect(
            canvas,
            1,
            l.EDIT_TEXT.y + 3 + (caret_row - top) as i32 * m::LINE_HEIGHT,
            2,
            m::LINE_HEIGHT,
            t.editor_caret,
        );
    }
    let columns = l.EDIT_COLUMNS;
    let mut row = 0usize;
    let mut column = 0usize;
    for i in 0..=model.len {
        if i == model.cursor && row >= top && row < top + l.EDIT_ROWS {
            c::caret(
                canvas,
                l.EDIT_TEXT.x + 6 + column as i32 * m::FONT_ADVANCE,
                l.EDIT_TEXT.y + 6 + (row - top) as i32 * m::LINE_HEIGHT,
                t.editor_caret,
            );
        }
        if i == model.len {
            break;
        }
        let b = model.data[i];
        if b == b'\n' {
            row += 1;
            column = 0;
            continue;
        }
        if row >= top && row < top + l.EDIT_ROWS && b != b'\t' {
            let glyph = [b];
            c::label(
                canvas,
                l.EDIT_TEXT.x + 6 + column as i32 * m::FONT_ADVANCE,
                l.EDIT_TEXT.y + 6 + (row - top) as i32 * m::LINE_HEIGHT,
                string(&glyph),
                t.text,
            );
        }
        column += if b == b'\t' { 4 - column % 4 } else { 1 };
        if column >= columns {
            row += 1;
            column = 0;
        }
    }
    // Independent of dialog state: the text area changes only when the
    // document does (guest oracles wait on that to know a file loaded).
    if model.len == 0 {
        c::label(
            canvas,
            l.EDIT_TEXT.x + 14,
            l.EDIT_TEXT.y + 6,
            "Empty document / ASCII text up to 4096 bytes",
            t.muted,
        );
    }
}

/// One Settings preference row: title, live description, switch.
fn preference(canvas: &mut Canvas<'_>, r: Rect, title: &str, detail: &str, on: bool, t: Theme) {
    c::text(canvas, r.x + m::L, r.y + 8, title, Style::Strong, t.text);
    c::text(
        canvas,
        r.x + m::L,
        r.y + 21,
        detail,
        Style::Body,
        t.secondary,
    );
    let sx = r.x + r.width as i32 - m::L - m::SWITCH_WIDTH;
    let sy = r.y + (r.height as i32 - m::SWITCH_HEIGHT) / 2;
    c::switch(canvas, sx, sy, on, State::Normal, t);
    c::text_right(
        canvas,
        sx - m::M,
        sy + 5,
        if on { "On" } else { "Off" },
        Style::Body,
        t.secondary,
    );
}

fn fact(canvas: &mut Canvas<'_>, y: i32, name: &str, value: &str, t: Theme) {
    let l = Layout::of(canvas);
    c::text(
        canvas,
        m::CONTENT_INSET + m::L,
        y,
        name,
        Style::Body,
        t.secondary,
    );
    c::text_right(
        canvas,
        l.W - m::CONTENT_INSET - m::L,
        y,
        value,
        Style::Body,
        t.text,
    );
}

pub fn settings(canvas: &mut Canvas<'_>, dark: bool, motion: bool, display: (u16, u16), t: Theme) {
    let l = Layout::of(canvas);
    page_header(
        canvas,
        "Appearance",
        "Stored on disk first, then applied to every window",
        t,
    );
    c::section(canvas, m::CONTENT_INSET, 72, "Theme and motion", None, t);
    let group = Rect {
        x: m::CONTENT_INSET,
        y: l.APPEARANCE.y,
        width: l.APPEARANCE.width,
        height: (l.MOTION.y + l.MOTION.height as i32 - l.APPEARANCE.y) as u32,
    };
    c::outlined(canvas, group, t.field, t.control_edge, m::RADIUS_PANEL);
    c::hline(
        canvas,
        group.x + m::L,
        l.MOTION.y,
        group.width as i32 - 2 * m::L,
        t.divider,
    );
    preference(
        canvas,
        l.APPEARANCE,
        "Dark appearance",
        if dark {
            "Ink palette on every window"
        } else {
            "Paper palette on every window"
        },
        dark,
        t,
    );
    preference(
        canvas,
        l.MOTION,
        "Interface motion",
        if motion {
            "Windows open, close and focus smoothly"
        } else {
            "Changes apply instantly"
        },
        motion,
        t,
    );
    c::section(canvas, m::CONTENT_INSET, 164, "Display", None, t);
    let mut text = [0u8; 32];
    let mut n = 0;
    for v in [display.0, display.1] {
        if n != 0 {
            text[n..n + 3].copy_from_slice(b" x ");
            n += 3;
        }
        let mut digits = [0u8; 32];
        let d = c::decimal(&mut digits, u64::from(v), false);
        text[n..n + d.len()].copy_from_slice(d.as_bytes());
        n += d.len();
    }
    let group = Rect {
        x: m::CONTENT_INSET,
        y: 174,
        width: l.APPEARANCE.width,
        height: 66,
    };
    c::outlined(canvas, group, t.elevated, t.divider, m::RADIUS_PANEL);
    fact(canvas, 182, "Resolution", string(&text[..n]), t);
    c::hline(
        canvas,
        group.x + m::L,
        196,
        l.APPEARANCE.width as i32 - 2 * m::L,
        t.divider,
    );
    fact(canvas, 204, "Pixel format", "XRGB8888, opaque", t);
    c::hline(
        canvas,
        group.x + m::L,
        218,
        l.APPEARANCE.width as i32 - 2 * m::L,
        t.divider,
    );
    fact(canvas, 226, "Type", "ArenaOS 5x7 bitmap", t);
}

pub fn file_row(canvas: &mut Canvas<'_>, row: usize, name: &[u8], selected: bool, t: Theme) {
    let l = Layout::of(canvas);
    c::row(
        canvas,
        Rect {
            x: l.FILE_LIST.x,
            y: l.FILE_LIST.y + row as i32 * l.ROW_H,
            width: l.FILE_LIST.width,
            height: l.ROW_H as u32,
        },
        string(name),
        Some(Glyph::Document),
        if selected {
            State::Selected
        } else {
            State::Normal
        },
        t,
    );
}

pub fn preview(canvas: &mut Canvas<'_>, name: Option<&[u8]>, bytes: &[u8], t: Theme) {
    let l = Layout::of(canvas);
    let p = l.PREVIEW;
    c::outlined(canvas, p, t.field, t.control_edge, m::RADIUS_PANEL);
    let x = p.x + m::M + 2;
    let right = p.x + p.width as i32 - m::M - 2;
    c::glyph(canvas, x, p.y + 8, Glyph::Document, t.accent);
    let Some(name) = name else {
        c::text(
            canvas,
            x + 15,
            p.y + 9,
            "Nothing selected",
            Style::Strong,
            t.secondary,
        );
        c::hline(canvas, p.x + 1, p.y + 22, p.width as i32 - 2, t.divider);
        return;
    };
    c::text_fit(
        canvas,
        x + 15,
        p.y + 9,
        string(name),
        Style::Strong,
        t.text,
        120,
    );
    let mut digits = [0u8; 32];
    let mut size = [0u8; 32];
    let d = c::decimal(&mut digits, bytes.len() as u64, true);
    size[..d.len()].copy_from_slice(d.as_bytes());
    size[d.len()..d.len() + 2].copy_from_slice(b" B");
    c::text_right(
        canvas,
        right,
        p.y + 9,
        string(&size[..d.len() + 2]),
        Style::Body,
        t.muted,
    );
    c::hline(canvas, p.x + 1, p.y + 22, p.width as i32 - 2, t.divider);
    if bytes.is_empty() {
        c::text(
            canvas,
            x,
            l.PREVIEW_TEXT_Y,
            "No previewable text",
            Style::Body,
            t.muted,
        );
        return;
    }
    for (row, line) in bytes
        .split(|b| *b == b'\n')
        .take(l.PREVIEW_ROWS)
        .enumerate()
    {
        // Tabs have no glyph; show them as single spaces in the preview.
        let mut shown = [0u8; 64];
        let n = line.len().min(l.PREVIEW_COLUMNS).min(64);
        for (i, b) in line[..n].iter().enumerate() {
            shown[i] = if *b == b'\t' { b' ' } else { *b };
        }
        c::text(
            canvas,
            x,
            l.PREVIEW_TEXT_Y + row as i32 * m::LINE_HEIGHT,
            string(&shown[..n]),
            Style::Body,
            t.text,
        );
    }
}

/// File list and controls share geometry with the application's hit testing.
pub struct FilesView<'a> {
    pub names: &'a [[u8; 32]],
    pub top: usize,
    pub selected: usize,
    pub bytes: &'a [u8],
    pub line: &'a Line,
    pub dialogue: bool,
}
pub fn files(canvas: &mut Canvas<'_>, model: FilesView<'_>, t: Theme) {
    let l = Layout::of(canvas);
    let FilesView {
        names,
        top,
        selected,
        bytes,
        line,
        dialogue,
    } = model;
    let none = if names.is_empty() {
        State::Disabled
    } else {
        State::Normal
    };
    if dialogue {
        dialog(canvas, line, "Create", t);
    } else {
        c::button(
            canvas,
            l.NEW,
            "New",
            Some(Glyph::Plus),
            Kind::Standard,
            State::Normal,
            t,
        );
        c::button(
            canvas,
            l.SAVE,
            "Refresh",
            Some(Glyph::Refresh),
            Kind::Standard,
            State::Normal,
            t,
        );
        c::button(
            canvas,
            l.OPEN,
            "Open",
            Some(Glyph::Open),
            Kind::Standard,
            none,
            t,
        );
        c::button(
            canvas,
            l.DELETE,
            "Delete",
            Some(Glyph::Delete),
            Kind::Destructive,
            none,
            t,
        );
        // Flat namespace: a count, never a path or folder breadcrumb.
        let mut digits = [0u8; 32];
        let n = c::decimal(&mut digits, names.len() as u64, false);
        c::text_right(canvas, l.W - m::CONTENT_INSET, 37, n, Style::Strong, t.text);
        c::text_right(
            canvas,
            l.W - m::CONTENT_INSET,
            50,
            "FILES",
            Style::Caption,
            t.muted,
        );
    }
    // List pane on the sidebar material.
    let pane_h = l.STATUS_Y - l.HEADER_BOTTOM - 1;
    c::rect(
        canvas,
        1,
        l.HEADER_BOTTOM + 1,
        l.FILE_LIST.x + l.FILE_LIST.width as i32 + 3,
        pane_h,
        t.sidebar,
    );
    c::vline(
        canvas,
        l.FILE_LIST.x + l.FILE_LIST.width as i32 + 4,
        l.HEADER_BOTTOM + 1,
        pane_h,
        t.divider,
    );
    if names.is_empty() {
        let x = l.FILE_LIST.x + m::M;
        c::glyph(canvas, x, l.CONTENT_Y + 10, Glyph::Document, t.muted);
        c::text(
            canvas,
            x + 15,
            l.CONTENT_Y + 11,
            "No user files",
            Style::Strong,
            t.text,
        );
        c::text(
            canvas,
            x,
            l.CONTENT_Y + 30,
            "New creates an empty",
            Style::Body,
            t.secondary,
        );
        c::text(
            canvas,
            x,
            l.CONTENT_Y + 44,
            "user-* file on AFS1.",
            Style::Body,
            t.secondary,
        );
    }
    for (row, (i, name)) in names
        .iter()
        .enumerate()
        .skip(top)
        .take(l.FILE_ROWS)
        .enumerate()
    {
        let end = name.iter().position(|b| *b == 0).unwrap_or(32);
        file_row(canvas, row, &name[..end], i == selected, t);
    }
    let name = names
        .get(selected)
        .map(|n| &n[..n.iter().position(|b| *b == 0).unwrap_or(32)]);
    preview(canvas, name, bytes, t);
}

fn stat(canvas: &mut Canvas<'_>, x: i32, y: i32, name: &str, value: u64, t: Theme) {
    let mut digits = [0u8; 32];
    c::text(canvas, x, y, name, Style::Caption, t.muted);
    c::text(
        canvas,
        x,
        y + 11,
        c::decimal(&mut digits, value, true),
        Style::Strong,
        t.text,
    );
}

pub fn monitor(
    canvas: &mut Canvas<'_>,
    counts: &[u64; 9],
    processes: &[(u64, u64)],
    top: usize,
    t: Theme,
) {
    let l = Layout::of(canvas);
    page_header(
        canvas,
        "Live system counters",
        "Kernel observations, sampled twice a second",
        t,
    );
    let mut digits = [0u8; 32];
    let seconds = c::decimal(&mut digits, counts[8] / 1_000_000, true);
    let right = l.W - m::CONTENT_INSET;
    c::text_right(canvas, right, 35, "s", Style::Body, t.secondary);
    c::text_right(canvas, right - 9, 35, seconds, Style::Strong, t.text);
    c::text_right(canvas, right, 48, "UPTIME", Style::Caption, t.muted);
    // Memory: used frames derived from the real free/total counters.
    let left = m::CONTENT_INSET;
    let col_w = 224;
    c::section(
        canvas,
        left,
        72,
        "Memory frames in use",
        Some(left + col_w),
        t,
    );
    let (free, total) = (counts[0], counts[1]);
    let used = total.saturating_sub(free);
    let end = c::text(
        canvas,
        left,
        84,
        c::decimal(&mut digits, used, true),
        Style::Display,
        t.text,
    );
    let mut whole = [0u8; 32];
    let of = c::decimal(&mut whole, total, true);
    let x = c::text(canvas, end + m::S, 91, "of ", Style::Body, t.secondary);
    c::text(canvas, x, 91, of, Style::Body, t.secondary);
    c::meter(
        canvas,
        Rect {
            x: left,
            y: 104,
            width: col_w as u32,
            height: 6,
        },
        used,
        total,
        t.accent,
        t,
    );
    c::section(canvas, left, 120, "Kernel objects", Some(left + col_w), t);
    for (i, (name, value)) in [
        ("Processes", counts[3]),
        ("Spawn records", counts[2]),
        ("Shared regions", counts[4]),
        ("Shared pages", counts[5]),
        ("Shared maps", counts[6]),
        ("Monitor caps", counts[7]),
    ]
    .into_iter()
    .enumerate()
    {
        let x = left + (i as i32 % 2) * (col_w / 2 + 4);
        let y = 132 + (i as i32 / 2) * 30;
        stat(canvas, x, y, name, value, t);
    }
    // Process list: real PIDs and thread counts; labels are descriptive.
    let lx = 252;
    let lw = l.W - m::CONTENT_INSET - lx;
    c::section(canvas, lx, 72, "Processes", None, t);
    let mut total = [0u8; 32];
    let total = c::decimal(&mut total, processes.len() as u64, false);
    c::text_right(canvas, lx + lw, 72, total, Style::Caption, t.secondary);
    c::text(canvas, lx + m::S, 84, "PID", Style::Caption, t.muted);
    c::text_right(
        canvas,
        lx + lw - m::S,
        84,
        "THREADS",
        Style::Caption,
        t.muted,
    );
    c::hline(canvas, lx, 93, lw, t.divider);
    for (row, (pid, threads)) in processes.iter().skip(top).take(l.MONITOR_ROWS).enumerate() {
        let y = l.MONITOR_ROW_Y + row as i32 * m::LINE_HEIGHT;
        if (row + top) % 2 == 1 {
            c::rect(canvas, lx, y - 3, lw, m::LINE_HEIGHT, t.header);
        }
        c::text(
            canvas,
            lx + m::S,
            y,
            c::decimal(&mut digits, *pid, false),
            Style::Body,
            t.text,
        );
        c::text_right(
            canvas,
            lx + lw - m::S,
            y,
            c::decimal(&mut digits, *threads, false),
            Style::Body,
            t.secondary,
        );
    }
    // Scroll position when the real list exceeds the viewport.
    if processes.len() > l.MONITOR_ROWS {
        let shown_end = (top + l.MONITOR_ROWS).min(processes.len());
        let mut a = [0u8; 32];
        let mut b = [0u8; 32];
        let mut text = [0u8; 32];
        let first = c::decimal(&mut a, top as u64 + 1, false);
        let last = c::decimal(&mut b, shown_end as u64, false);
        let n = first.len() + 1 + last.len();
        text[..first.len()].copy_from_slice(first.as_bytes());
        text[first.len()] = b'-';
        text[first.len() + 1..n].copy_from_slice(last.as_bytes());
        let x = lx + c::measure("PROCESSES", Style::Caption) + m::M;
        c::text(canvas, x, 72, string(&text[..n]), Style::Caption, t.accent);
    }
}

/// Height of one context-menu item and the menu's inner padding.
pub const MENU_ITEM: i32 = 24;
pub const MENU_PAD: i32 = 4;
pub const MENU_WIDTH: u16 = 184;
pub fn menu_height(items: usize) -> u16 {
    (items as i32 * MENU_ITEM + 2 * MENU_PAD) as u16
}
/// Item under transient-local (`x`, `y`), if any.
pub fn menu_item(items: usize, x: i32, y: i32) -> Option<usize> {
    let row = (y - MENU_PAD).div_euclid(MENU_ITEM);
    (x >= 0 && x < i32::from(MENU_WIDTH) && y >= MENU_PAD && (row as usize) < items)
        .then_some(row as usize)
}
/// A context menu painted into its own transient surface.
pub fn menu(canvas: &mut Canvas<'_>, items: &[&str], hover: Option<usize>, t: Theme) {
    let (w, h) = canvas.size();
    c::rect(canvas, 0, 0, w as i32, h as i32, t.elevated);
    for (i, item) in items.iter().enumerate() {
        let y = MENU_PAD + i as i32 * MENU_ITEM;
        let on = hover == Some(i);
        if on {
            c::rect(canvas, 3, y, w as i32 - 6, MENU_ITEM, t.accent);
        }
        c::text(
            canvas,
            m::L,
            y + (MENU_ITEM - m::FONT_HEIGHT) / 2,
            item,
            Style::Body,
            if on { t.on_accent } else { t.text },
        );
    }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    fn render(f: impl Fn(&mut Canvas<'_>)) -> std::vec::Vec<u32> {
        let mut pixels = std::vec![0u32; m::WINDOW_WIDTH * m::WINDOW_HEIGHT];
        let mut canvas = Canvas::new(
            &mut pixels,
            m::WINDOW_WIDTH,
            m::WINDOW_HEIGHT,
            m::WINDOW_WIDTH,
        )
        .unwrap();
        f(&mut canvas);
        drop(canvas);
        pixels
    }
    fn region(p: &[u32], x: i32, y: i32, w: i32, h: i32) -> impl Iterator<Item = u32> + '_ {
        (y..y + h).flat_map(move |r| {
            (x..x + w).map(move |c| p[r as usize * m::WINDOW_WIDTH + c as usize])
        })
    }
    /// Guest oracles synchronise keyboard input with pointer clicks by
    /// waiting for the editor toolbar strip (x12..366, y36..60) to change,
    /// and for the text area to change once a document has loaded. Neither
    /// may change for unrelated state (dirty flag, caret, dialog-only).
    #[test]
    fn editor_probe_regions_change_only_for_their_events() {
        let line = Line::new();
        for dark in [false, true] {
            let t = arena_ui::theme::palette(dark);
            let mut doc = Editor::new();
            doc.load(b"hello\nworld", "user-note").unwrap();
            let clean = render(|c| editor(c, &doc, 0, &line, 0, t));
            doc.insert(b'x').unwrap();
            doc.cursor = 8;
            let dirty = render(|c| editor(c, &doc, 0, &line, 0, t));
            assert!(region(&clean, 12, 36, 354, 24).eq(region(&dirty, 12, 36, 354, 24)));
            let dialog = render(|c| editor(c, &doc, 0, &line, 1, t));
            assert!(!region(&clean, 12, 36, 354, 24).eq(region(&dialog, 12, 36, 354, 24)));
            let empty = Editor::new();
            let blank = render(|c| editor(c, &empty, 0, &line, 0, t));
            let opening = render(|c| editor(c, &empty, 0, &line, 2, t));
            assert!(region(&blank, 18, 74, 330, 140).eq(region(&opening, 18, 74, 330, 140)));
        }
    }
    /// Guest oracles detect a new cascaded window by its title strip, which
    /// lands over the Terminal's local rows 56..65 (x 64..244). That strip
    /// must stay plain so a window that never opened is never mistaken for
    /// one that did (test_m10_boundaries_red readonly-diagnostic-grant).
    #[test]
    fn terminal_header_keeps_cascade_probe_strip_plain() {
        let mut model = Terminal::new();
        model.write(b"ArenaOS ordinary command session\nType help.");
        for dark in [false, true] {
            let mut pixels = [0u32; m::WINDOW_WIDTH * m::WINDOW_HEIGHT];
            let mut canvas = Canvas::new(
                &mut pixels,
                m::WINDOW_WIDTH,
                m::WINDOW_HEIGHT,
                m::WINDOW_WIDTH,
            )
            .unwrap();
            let t = arena_ui::theme::palette(dark);
            frame(&mut canvas, "Terminal", "READY", t);
            terminal(&mut canvas, &model, 3, t);
            drop(canvas);
            let probe = pixels[56 * m::WINDOW_WIDTH + 64];
            for y in 56..66 {
                for x in 64..244 {
                    assert_eq!(pixels[y * m::WINDOW_WIDTH + x], probe, "({x},{y})");
                }
            }
        }
    }
}
