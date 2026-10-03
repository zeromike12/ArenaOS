//! Reusable raster components; labels and state are supplied by app views.
use crate::{metrics as m, theme::Theme};
use arena_gfxkit::{Canvas, Rect};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Normal,
    Focused,
    Selected,
    Disabled,
    Refused,
}
pub fn rect(c: &mut Canvas<'_>, x: i32, y: i32, w: i32, h: i32, color: u32) {
    if w > 0 && h > 0 {
        c.fill_rect(
            Rect {
                x,
                y,
                width: w as u32,
                height: h as u32,
            },
            color,
        );
    }
}
pub fn label(c: &mut Canvas<'_>, x: i32, y: i32, s: &str, color: u32) {
    // Split long strings without weakening gfxkit's bounded input contract.
    for (i, chunk) in s.as_bytes().chunks(96).enumerate() {
        if let Ok(text) = core::str::from_utf8(chunk) {
            let _ = c.text_scaled(
                x + (i * 96) as i32 * m::FONT_ADVANCE,
                y,
                text,
                color,
                m::FONT_SCALE,
            );
        }
    }
}
pub fn heading(c: &mut Canvas<'_>, x: i32, y: i32, s: &str, color: u32) {
    let _ = c.text_scaled(x, y, s, color, m::TITLE_SCALE);
}
pub fn border(c: &mut Canvas<'_>, r: Rect, color: u32) {
    let (w, h) = (r.width as i32, r.height as i32);
    rect(c, r.x, r.y, w, 1, color);
    rect(c, r.x, r.y + h - 1, w, 1, color);
    rect(c, r.x, r.y, 1, h, color);
    rect(c, r.x + w - 1, r.y, 1, h, color);
}
pub fn button(c: &mut Canvas<'_>, r: Rect, text: &str, state: State, t: Theme) {
    let bg = match state {
        State::Focused | State::Selected => t.selection,
        _ => t.panel,
    };
    rect(c, r.x, r.y, r.width as i32, r.height as i32, bg);
    border(
        c,
        r,
        match state {
            State::Focused => t.accent,
            State::Refused => t.error,
            _ => t.border,
        },
    );
    label(
        c,
        r.x + m::SPACE[2],
        r.y + (r.height as i32 - m::FONT_HEIGHT) / 2,
        text,
        if state == State::Disabled {
            t.disabled
        } else {
            t.text
        },
    );
}
pub fn field(
    c: &mut Canvas<'_>,
    r: Rect,
    text: &str,
    cursor: Option<usize>,
    state: State,
    t: Theme,
) {
    rect(c, r.x, r.y, r.width as i32, r.height as i32, t.elevated);
    border(
        c,
        r,
        if state == State::Focused {
            t.accent
        } else {
            t.border
        },
    );
    label(c, r.x + m::SPACE[2], r.y + m::SPACE[2], text, t.text);
    if let Some(pos) = cursor {
        rect(
            c,
            r.x + m::SPACE[2] + pos as i32 * m::FONT_ADVANCE,
            r.y + m::SPACE[2] - 1,
            1,
            m::FONT_HEIGHT + 2,
            t.accent,
        );
    }
}
pub fn row(c: &mut Canvas<'_>, r: Rect, text: &str, state: State, t: Theme) {
    rect(
        c,
        r.x,
        r.y,
        r.width as i32,
        r.height as i32,
        if state == State::Selected {
            t.selection
        } else {
            t.elevated
        },
    );
    label(
        c,
        r.x + m::SPACE[2],
        r.y + m::SPACE[2],
        text,
        if state == State::Disabled {
            t.disabled
        } else {
            t.text
        },
    );
}
pub fn progress(c: &mut Canvas<'_>, r: Rect, value: u8, t: Theme) {
    rect(c, r.x, r.y, r.width as i32, r.height as i32, t.panel);
    rect(
        c,
        r.x,
        r.y,
        (u64::from(r.width) * u64::from(value.min(100)) / 100) as i32,
        r.height as i32,
        t.accent,
    );
    border(c, r, t.border);
}
pub fn chrome(c: &mut Canvas<'_>, title: &str, focused: bool, t: Theme) {
    let (w, h) = c.size();
    rect(
        c,
        0,
        0,
        w as i32,
        m::TITLE_HEIGHT,
        if focused {
            t.chrome_active
        } else {
            t.chrome_inactive
        },
    );
    heading(
        c,
        m::CONTENT_INSET,
        (m::TITLE_HEIGHT - m::TITLE_FONT_HEIGHT) / 2,
        title,
        if focused { t.text } else { t.secondary },
    );
    label(
        c,
        w as i32 - m::CLOSE_WIDTH + 8,
        m::SPACE[3] - 2,
        "X",
        t.error,
    );
    border(
        c,
        Rect {
            x: 0,
            y: 0,
            width: w as u32,
            height: h as u32,
        },
        if focused { t.accent } else { t.border },
    );
}
pub fn icon(c: &mut Canvas<'_>, x: i32, y: i32, index: u8, t: Theme) {
    border(
        c,
        Rect {
            x,
            y,
            width: m::ICON_SIZE as u32,
            height: m::ICON_SIZE as u32,
        },
        t.accent,
    );
    // Owned geometric icon vocabulary, with no external artwork.
    for i in 0..3 {
        if index & (1 << i) != 0 {
            rect(c, x + 3, y + 3 + i * 4, 10, 2, t.accent);
        }
    }
}
pub fn gallery(c: &mut Canvas<'_>, t: Theme) {
    c.clear(t.elevated);
    chrome(c, "ArenaOS UI Gallery", true, t);
    let control = |x, y, w| Rect {
        x,
        y,
        width: w,
        height: m::CONTROL_HEIGHT as u32,
    };
    label(c, 12, 40, "Component gallery", t.text);
    rect(c, 246, 34, 84, 18, t.chrome_active);
    label(c, 254, 40, "Focused", t.text);
    rect(c, 342, 34, 88, 18, t.chrome_inactive);
    label(c, 350, 40, "Unfocused", t.secondary);
    for (i, (text, state)) in [
        ("Button", State::Normal),
        ("Focused", State::Focused),
        ("Disabled", State::Disabled),
        ("Refused", State::Refused),
    ]
    .iter()
    .enumerate()
    {
        button(c, control(12 + i as i32 * 106, 58, 98), text, *state, t);
    }
    field(
        c,
        control(12, 94, 202),
        "Text field",
        Some(10),
        State::Focused,
        t,
    );
    field(
        c,
        control(228, 94, 202),
        "Unfocused field",
        None,
        State::Normal,
        t,
    );
    rect(c, 12, 130, m::SIDEBAR_WIDTH, 108, t.panel);
    label(c, 20, 140, "Sidebar", t.secondary);
    row(c, control(118, 130, 200), "List row", State::Normal, t);
    row(
        c,
        control(118, 158, 200),
        "Selected row",
        State::Selected,
        t,
    );
    row(
        c,
        control(118, 186, 200),
        "Disabled row",
        State::Disabled,
        t,
    );
    rect(c, 330, 130, 100, 60, t.terminal);
    label(c, 338, 140, "arena> help", t.terminal_text);
    label(c, 338, 156, "Terminal", t.terminal_text);
    for i in 0..5 {
        icon(c, 122 + i * 24, 224, (i + 1) as u8, t);
    }
    progress(
        c,
        Rect {
            x: 254,
            y: 224,
            width: 176,
            height: 12,
        },
        65,
        t,
    );
    label(c, 12, 252, "Motion", t.secondary);
    let mut motion = crate::motion::Motion::fixed(0);
    motion.retarget(
        100,
        0,
        crate::motion::OPEN_US,
        crate::motion::Easing::Smooth,
    );
    for (i, text) in ["0%", "50%", "100%"].iter().enumerate() {
        let x = 84 + i as i32 * 116;
        label(c, x, 252, text, t.secondary);
        progress(
            c,
            Rect {
                x: x + 32,
                y: 252,
                width: 64,
                height: 8,
            },
            motion.sample(crate::motion::OPEN_US * i as u64 / 2) as u8,
            t,
        );
    }
}
