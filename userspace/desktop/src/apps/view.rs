//! Presentation-only application views. No service calls or capabilities.
use super::{
    layout as l,
    model::{Editor, Line, Terminal},
};
use arena_gfxkit::{Canvas, Rect};
use arena_ui::{components as c, metrics as m, theme::Theme};
pub fn string(bytes: &[u8]) -> &str {
    core::str::from_utf8(bytes).unwrap_or("Unsupported text")
}
pub fn frame(canvas: &mut Canvas<'_>, title: &str, status: &str, t: Theme) {
    canvas.clear(t.elevated);
    c::chrome(canvas, title, true, t);
    c::label(canvas, m::CONTENT_INSET, l::STATUS_Y, status, t.secondary);
}
pub fn terminal(canvas: &mut Canvas<'_>, model: &Terminal, older: usize, t: Theme) {
    c::rect(
        canvas,
        m::CONTENT_INSET,
        l::CONTENT_Y,
        424,
        l::STATUS_Y - l::CONTENT_Y - 6,
        t.terminal,
    );
    let visible = 12;
    let start = model.count.saturating_sub(visible).saturating_sub(older);
    for (row, index) in (start..model.count).take(visible).enumerate() {
        c::label(
            canvas,
            m::CONTENT_INSET + 6,
            l::CONTENT_Y + 6 + row as i32 * m::LINE_HEIGHT,
            string(&model.lines[index][..model.sizes[index]]),
            t.terminal_text,
        );
    }
    c::label(
        canvas,
        m::CONTENT_INSET,
        l::TOOL_Y + 8,
        "ARENAOS SESSION / HELP",
        t.secondary,
    );
    let y = l::STATUS_Y - 20;
    c::label(canvas, m::CONTENT_INSET + 6, y, ">", t.terminal_text);
    c::label(
        canvas,
        m::CONTENT_INSET + 18,
        y,
        string(&model.input[..model.len]),
        t.terminal_text,
    );
    c::rect(
        canvas,
        m::CONTENT_INSET + 18 + model.cursor as i32 * m::FONT_ADVANCE,
        y - 1,
        1,
        m::FONT_HEIGHT + 2,
        t.terminal_text,
    );
}
pub fn dialog(canvas: &mut Canvas<'_>, line: &Line, label: &str, t: Theme) {
    c::field(
        canvas,
        l::NAME_FIELD,
        string(&line.bytes[..line.len]),
        Some(line.cursor),
        c::State::Focused,
        t,
    );
    c::button(canvas, l::PRIMARY, label, c::State::Normal, t);
    c::button(canvas, l::SECONDARY, "CANCEL", c::State::Normal, t);
}
pub fn editor(
    canvas: &mut Canvas<'_>,
    model: &Editor,
    top: usize,
    line: &Line,
    dialogue: u8,
    t: Theme,
) {
    if dialogue == 4 {
        c::button(canvas, l::NAME_FIELD, "SAVE", c::State::Normal, t);
        c::button(canvas, l::PRIMARY, "DISCARD", c::State::Normal, t);
        c::button(canvas, l::SECONDARY, "CANCEL", c::State::Normal, t);
    } else if dialogue != 0 {
        dialog(canvas, line, "SAVE", t);
    } else {
        c::button(canvas, l::NEW, "NEW", c::State::Normal, t);
        c::button(canvas, l::SAVE, "SAVE", c::State::Normal, t);
        c::button(canvas, l::SAVE_AS, "SAVE AS", c::State::Normal, t);
        c::button(canvas, l::OPEN, "OPEN", c::State::Normal, t);
    }
    c::border(canvas, l::EDIT_TEXT, t.border);
    let columns = 68usize;
    let mut row = 0usize;
    let mut column = 0usize;
    for i in 0..=model.len {
        if i == model.cursor && row >= top && row < top + 13 {
            c::rect(
                canvas,
                l::EDIT_TEXT.x + 6 + column as i32 * m::FONT_ADVANCE,
                l::EDIT_TEXT.y + 6 + (row - top) as i32 * m::LINE_HEIGHT - 1,
                1,
                m::FONT_HEIGHT + 2,
                t.accent,
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
        if row >= top && row < top + 13 && b != b'\t' {
            let glyph = [b];
            c::label(
                canvas,
                l::EDIT_TEXT.x + 6 + column as i32 * m::FONT_ADVANCE,
                l::EDIT_TEXT.y + 6 + (row - top) as i32 * m::LINE_HEIGHT,
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
}
pub fn settings(canvas: &mut Canvas<'_>, dark: bool, motion: bool, display: (u16, u16), t: Theme) {
    c::rect(canvas, m::CONTENT_INSET, l::CONTENT_Y, 104, 182, t.panel);
    c::label(
        canvas,
        m::CONTENT_INSET + 8,
        l::CONTENT_Y + 12,
        "APPEARANCE",
        t.text,
    );
    c::label(
        canvas,
        128,
        l::CONTENT_Y + 4,
        "DURABLE DESKTOP THEME",
        t.secondary,
    );
    c::button(
        canvas,
        l::APPEARANCE,
        if dark {
            "USE LIGHT THEME"
        } else {
            "USE DARK THEME"
        },
        c::State::Normal,
        t,
    );
    c::button(
        canvas,
        l::MOTION,
        if motion {
            "DISABLE MOTION"
        } else {
            "ENABLE MOTION"
        },
        c::State::Normal,
        t,
    );
    c::label(
        canvas,
        128,
        l::CONTENT_Y + 78,
        "DISPLAY / XRGB8888",
        t.secondary,
    );
    let mut text = [0u8; 32];
    let mut n = 0;
    for v in [display.0, display.1] {
        if n != 0 {
            text[n] = b'X';
            n += 1;
        }
        let mut digits = [0; 5];
        let mut len = 0;
        let mut v = v;
        loop {
            digits[len] = b'0' + (v % 10) as u8;
            len += 1;
            v /= 10;
            if v == 0 {
                break;
            }
        }
        for b in digits[..len].iter().rev() {
            text[n] = *b;
            n += 1;
        }
    }
    c::label(canvas, 128, l::CONTENT_Y + 100, string(&text[..n]), t.text);
    c::label(
        canvas,
        128,
        l::CONTENT_Y + 130,
        "BITMAP FONT 5X7 / OPAQUE PIXELS",
        t.secondary,
    );
}
pub fn file_row(canvas: &mut Canvas<'_>, row: usize, name: &[u8], selected: bool, t: Theme) {
    c::row(
        canvas,
        Rect {
            x: l::FILE_LIST.x,
            y: l::FILE_LIST.y + row as i32 * l::ROW_H,
            width: l::FILE_LIST.width,
            height: l::ROW_H as u32,
        },
        string(name),
        if selected {
            c::State::Selected
        } else {
            c::State::Normal
        },
        t,
    );
}
pub fn preview(canvas: &mut Canvas<'_>, bytes: &[u8], t: Theme) {
    c::border(canvas, l::PREVIEW, t.border);
    for (row, line) in bytes.split(|b| *b == b'\n').take(12).enumerate() {
        c::label(
            canvas,
            l::PREVIEW.x + 6,
            l::PREVIEW.y + 6 + row as i32 * m::LINE_HEIGHT,
            string(&line[..line.len().min(33)]),
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
    let FilesView {
        names,
        top,
        selected,
        bytes,
        line,
        dialogue,
    } = model;
    if dialogue {
        dialog(canvas, line, "CREATE", t);
    } else {
        c::button(canvas, l::NEW, "NEW", c::State::Normal, t);
        c::button(canvas, l::SAVE, "REFRESH", c::State::Normal, t);
        c::button(canvas, l::OPEN, "OPEN", c::State::Normal, t);
        c::button(
            canvas,
            l::DELETE,
            "DELETE",
            if names.is_empty() {
                c::State::Disabled
            } else {
                c::State::Normal
            },
            t,
        );
    }
    for (row, (i, name)) in names.iter().enumerate().skip(top).take(10).enumerate() {
        let end = name.iter().position(|b| *b == 0).unwrap_or(32);
        file_row(canvas, row, &name[..end], i == selected, t);
    }
    preview(canvas, bytes, t);
}
pub fn monitor(
    canvas: &mut Canvas<'_>,
    counts: &[u64; 9],
    processes: &[(u64, u64)],
    top: usize,
    t: Theme,
) {
    for (row, (label, value)) in [
        (b"FREE FRAMES ".as_slice(), counts[0]),
        (b"TOTAL FRAMES ", counts[1]),
        (b"LIVE PROCESSES ", counts[3]),
        (b"SHARED REGIONS ", counts[4]),
        (b"SHARED PAGES ", counts[5]),
        (b"SHARED MAPS ", counts[6]),
        (b"OWN CAPS ", counts[7]),
        (b"UPTIME SECONDS ", counts[8] / 1_000_000),
    ]
    .into_iter()
    .enumerate()
    {
        let mut b = [0; 64];
        let mut n = 0;
        append(&mut b, &mut n, label);
        number(&mut b, &mut n, value);
        c::label(
            canvas,
            12,
            l::CONTENT_Y + row as i32 * m::LINE_HEIGHT,
            string(&b[..n]),
            t.text,
        );
    }
    c::label(canvas, 230, l::CONTENT_Y, "PID / THREADS", t.secondary);
    for (row, (pid, threads)) in processes.iter().skip(top).take(12).enumerate() {
        let mut b = [0; 64];
        let mut n = 0;
        number(&mut b, &mut n, *pid);
        append(&mut b, &mut n, b" / ");
        number(&mut b, &mut n, *threads);
        c::label(
            canvas,
            230,
            l::CONTENT_Y + 16 + row as i32 * m::LINE_HEIGHT,
            string(&b[..n]),
            t.text,
        );
    }
}
fn append(b: &mut [u8; 64], n: &mut usize, s: &[u8]) {
    let count = s.len().min(64 - *n);
    b[*n..*n + count].copy_from_slice(&s[..count]);
    *n += count;
}
fn number(b: &mut [u8; 64], n: &mut usize, mut v: u64) {
    let mut d = [0; 20];
    let mut len = 0;
    loop {
        d[len] = b'0' + (v % 10) as u8;
        len += 1;
        v /= 10;
        if v == 0 {
            break;
        }
    }
    for digit in d[..len].iter().rev() {
        append(b, n, core::slice::from_ref(digit));
    }
}
