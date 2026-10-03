//! UI Gallery: the canonical, deterministic statement of the ArenaOS
//! visual system. It draws only the shared components the shell and the
//! applications use; nothing here is a gallery-only rendering path.
use super::{
    State,
    controls::{
        Kind, Tone, button, chip, field, meter, progress, row, section, status_band, switch,
    },
    icons::{Glyph, app_tile, glyph, lamp},
    shapes::{hline, outlined, rect, rounded},
    text::{Style, text},
};
use crate::{metrics as m, motion, theme::Theme};
use arena_gfxkit::{Canvas, Rect};

const COL: [i32; 3] = [12, 156, 300];
const COL_W: i32 = 136;
const ROW1: i32 = 72;
const ROW2: i32 = 174;

const fn r(x: i32, y: i32, w: i32, h: i32) -> Rect {
    Rect {
        x,
        y,
        width: w as u32,
        height: h as u32,
    }
}

pub fn gallery(c: &mut Canvas<'_>, t: Theme) {
    let (w, _) = c.size();
    let w = w as i32;
    let dark = t == crate::theme::DARK;
    c.clear(t.elevated);
    super::chrome::chrome(c, "UI Gallery", true, t);
    // Page header: identity on the left, chrome focus states on the right.
    rect(
        c,
        1,
        m::TITLE_HEIGHT,
        w - 2,
        m::HEADER_BOTTOM - m::TITLE_HEIGHT,
        t.header,
    );
    hline(c, 0, m::HEADER_BOTTOM, w, t.divider);
    text(c, 12, 35, "Design system", Style::Strong, t.text);
    text(
        c,
        12,
        48,
        if dark {
            "Ink palette / 5x7 type / 2px grid"
        } else {
            "Paper palette / 5x7 type / 2px grid"
        },
        Style::Body,
        t.secondary,
    );
    for (i, (name, amount)) in [("Focused", 65_536), ("Resting", 0)]
        .into_iter()
        .enumerate()
    {
        let x = 300 + i as i32 * 70;
        let sw = r(x, 36, 66, 20);
        let frame = motion::color(t.frame, t.frame_focus, amount);
        outlined(c, sw, t.elevated, frame, 0);
        rect(
            c,
            x + 1,
            37,
            64,
            1,
            motion::color(t.elevated, t.accent, amount),
        );
        lamp(c, x + 5, 42, amount, t.elevated, t);
        text(
            c,
            x + 17,
            43,
            name,
            Style::Body,
            motion::color(t.secondary, t.text, amount),
        );
    }

    // Buttons: kinds across, states down.
    section(c, COL[0], ROW1, "Buttons", Some(COL[0] + COL_W), t);
    let bw = (COL_W - 6) / 2;
    for (i, (label, icon, kind, state)) in [
        ("Button", None, Kind::Standard, State::Normal),
        ("Primary", None, Kind::Primary, State::Normal),
        ("Focused", None, Kind::Standard, State::Focused),
        (
            "Delete",
            Some(Glyph::Delete),
            Kind::Destructive,
            State::Normal,
        ),
        ("Disabled", None, Kind::Standard, State::Disabled),
        ("Refused", None, Kind::Standard, State::Refused),
    ]
    .into_iter()
    .enumerate()
    {
        let x = COL[0] + (i as i32 % 2) * (bw + 6);
        let y = 84 + (i as i32 / 2) * 28;
        button(
            c,
            r(x, y, bw, m::CONTROL_HEIGHT),
            label,
            icon,
            kind,
            state,
            t,
        );
    }

    // Fields: focused with caret, resting, refused.
    section(c, COL[1], ROW1, "Fields", Some(COL[1] + COL_W), t);
    field(
        c,
        r(COL[1], 84, COL_W, 24),
        "user-note",
        Some(9),
        State::Focused,
        t,
    );
    field(
        c,
        r(COL[1], 112, COL_W, 24),
        "Resting field",
        None,
        State::Normal,
        t,
    );
    field(
        c,
        r(COL[1], 140, COL_W, 24),
        "bad name?",
        None,
        State::Refused,
        t,
    );

    // Sidebar rows: every list state on the inset sidebar material.
    section(c, COL[2], ROW1, "Sidebar rows", Some(COL[2] + COL_W), t);
    rounded(c, r(COL[2], 82, COL_W, 84), t.sidebar, m::RADIUS);
    for (i, (label, state)) in [
        ("user-note", State::Normal),
        ("user-copy", State::Selected),
        ("user-full", State::Disabled),
        ("user-bin", State::Refused),
    ]
    .into_iter()
    .enumerate()
    {
        row(
            c,
            r(COL[2], 84 + i as i32 * 20, COL_W, 20),
            label,
            Some(Glyph::Document),
            state,
            t,
        );
    }

    // Console sample in the Terminal palette.
    section(c, COL[0], ROW2, "Console", Some(COL[0] + COL_W), t);
    let console = r(COL[0], 186, COL_W, 70);
    outlined(c, console, t.terminal, t.terminal_edge, m::RADIUS);
    let lines: [(bool, &str); 4] = [
        (true, "ls"),
        (false, "user-note"),
        (false, "user-copy"),
        (true, "help"),
    ];
    for (i, (prompt, s)) in lines.into_iter().enumerate() {
        let y = 194 + i as i32 * 14;
        let x = COL[0] + 8;
        if prompt {
            glyph(c, x - 2, y - 1, Glyph::Chevron, t.terminal_prompt);
            text(c, x + 10, y, s, Style::Body, t.terminal_text);
        } else {
            text(c, x + 10, y, s, Style::Body, t.terminal_muted);
        }
    }
    rect(c, COL[0] + 8 + 10 + 4 * 6, 236, 6, 11, t.terminal_prompt);

    // State: switches, progress and status chips.
    section(c, COL[1], ROW2, "State", Some(COL[1] + COL_W), t);
    switch(c, COL[1], 186, true, State::Normal, t);
    text(c, COL[1] + 36, 191, "On", Style::Body, t.text);
    switch(c, COL[1] + 70, 186, false, State::Normal, t);
    text(c, COL[1] + 106, 191, "Off", Style::Body, t.secondary);
    progress(c, r(COL[1], 210, COL_W, 6), 65, t);
    meter(c, r(COL[1], 220, COL_W, 6), 2, 9, t.warning, t);
    let x = COL[1] + chip(c, COL[1], 232, "Saved", Tone::Success, t) + 4;
    chip(c, x, 232, "Modified", Tone::Warning, t);
    let x = COL[1] + chip(c, COL[1], 248, "Refused", Tone::Error, t) + 4;
    chip(c, x, 248, "Ready", Tone::Neutral, t);

    // Motion: the real shared Smooth curve over the real OPEN duration,
    // plotted from Motion::sample, then the application icon family.
    section(c, COL[2], ROW2, "Motion 120ms", Some(COL[2] + COL_W), t);
    let (top, height) = (186, 34);
    hline(c, COL[2], top + height, COL_W, t.divider);
    hline(c, COL[2], top, COL_W, t.divider);
    let mut sample = motion::Motion::fixed(0);
    sample.retarget(height, 0, motion::OPEN_US, motion::Easing::Smooth);
    for i in 0..=COL_W / 2 {
        let v = sample.sample(motion::OPEN_US * i as u64 / (COL_W / 2) as u64);
        rect(c, COL[2] + i * 2, top + height - v, 2, 2, t.accent);
    }
    for i in 0..6u8 {
        app_tile(c, COL[2] + i as i32 * 23, 234, 20, i, false, t.elevated, t);
    }

    status_band(
        c,
        m::FOOTER_Y,
        "Deterministic reference / T switches palette",
        Tone::Neutral,
        t,
    );
}
