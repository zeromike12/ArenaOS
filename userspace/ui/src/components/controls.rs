//! Interactive controls and state presentation.
//!
//! Every state carries a non-colour cue as well as a tone: focus adds a
//! two-pixel ring, selection adds a leading bar and stronger weight,
//! disabled uses a dotted outline, refusal adds the error glyph.
use super::{
    State,
    icons::{Glyph, glyph},
    shapes::{dotted, hline, inset, outlined, rect, rounded, stadium, vline},
    text::{Style, measure, text, text_fit},
};
use crate::{metrics as m, theme::Theme};
use arena_gfxkit::{Canvas, Rect};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Standard,
    /// The one recommended action of a surface.
    Primary,
    /// Removes data; the label carries the error tone.
    Destructive,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    Neutral,
    Accent,
    Success,
    Warning,
    Error,
}

impl Tone {
    /// (strong, soft) colours for this tone.
    pub const fn colors(self, t: Theme) -> (u32, u32) {
        match self {
            Tone::Neutral => (t.secondary, t.header),
            Tone::Accent => (t.accent, t.accent_soft),
            Tone::Success => (t.success, t.success_soft),
            Tone::Warning => (t.warning, t.warning_soft),
            Tone::Error => (t.error, t.error_soft),
        }
    }
    pub const fn glyph(self) -> Glyph {
        match self {
            Tone::Neutral | Tone::Accent => Glyph::Idle,
            Tone::Success => Glyph::Success,
            Tone::Warning => Glyph::Warning,
            Tone::Error => Glyph::Error,
        }
    }
}

/// Two-pixel rounded focus ring drawn just inside `r`.
pub fn focus_ring(c: &mut Canvas<'_>, r: Rect, inner: u32, t: Theme) {
    outlined(c, r, t.accent, t.accent, m::RADIUS);
    rounded(c, inset(r, 2), inner, 1);
}

pub fn button(
    c: &mut Canvas<'_>,
    r: Rect,
    label: &str,
    icon: Option<Glyph>,
    kind: Kind,
    state: State,
    t: Theme,
) {
    let (fill, edge, ink, mark) = match (kind, state) {
        (_, State::Disabled) => (t.control, t.disabled, t.disabled, t.disabled),
        (_, State::Refused) => (t.error_soft, t.error, t.error, t.error),
        (Kind::Primary, _) => (t.accent, t.accent_strong, t.on_accent, t.on_accent),
        (_, State::Selected) => (t.accent_soft, t.accent, t.text, t.accent),
        (Kind::Destructive, _) => (t.control, t.control_edge, t.error, t.error),
        (Kind::Standard, _) => (t.control, t.control_edge, t.text, t.secondary),
    };
    match state {
        State::Disabled => {
            rounded(c, r, fill, m::RADIUS);
            dotted(c, inset(r, 1), edge);
        }
        State::Focused => {
            focus_ring(c, r, fill, t);
        }
        _ => outlined(c, r, fill, edge, m::RADIUS),
    }
    let icon = if state == State::Refused {
        Some(Glyph::Error)
    } else {
        icon
    };
    let style = if kind == Kind::Primary || state == State::Selected {
        Style::Strong
    } else {
        Style::Body
    };
    let room = r.width as i32 - 2 * m::S;
    let icon_w = if icon.is_some() {
        m::GLYPH_SMALL + 4
    } else {
        0
    };
    let label_w = measure(label, style).min(room - icon_w);
    let mut x = r.x + (r.width as i32 - icon_w - label_w) / 2;
    let y = r.y + (r.height as i32 - m::FONT_HEIGHT) / 2;
    if let Some(g) = icon {
        glyph(c, x, y - 1, g, mark);
        x += icon_w;
    }
    text_fit(c, x, y, label, style, ink, room - icon_w);
}

/// I-beam caret for the glyph cell whose top-left is (x, y).
pub fn caret(c: &mut Canvas<'_>, x: i32, y: i32, color: u32) {
    vline(c, x - 1, y - 2, m::FONT_HEIGHT + 4, color);
    hline(c, x - 2, y - 2, 3, color);
    hline(c, x - 2, y + m::FONT_HEIGHT + 1, 3, color);
}

pub fn field(
    c: &mut Canvas<'_>,
    r: Rect,
    value: &str,
    cursor: Option<usize>,
    state: State,
    t: Theme,
) {
    match state {
        State::Focused => focus_ring(c, r, t.field, t),
        State::Refused => {
            outlined(c, r, t.field, t.error, m::RADIUS);
            vline(c, r.x + 1, r.y + 2, r.height as i32 - 4, t.error);
        }
        State::Disabled => {
            rounded(c, r, t.control, m::RADIUS);
            dotted(c, inset(r, 1), t.disabled);
        }
        _ => outlined(c, r, t.field, t.field_edge, m::RADIUS),
    }
    let x = r.x + m::M;
    let y = r.y + (r.height as i32 - m::FONT_HEIGHT) / 2;
    let ink = match state {
        State::Disabled => t.disabled,
        _ => t.text,
    };
    text_fit(c, x, y, value, Style::Body, ink, r.width as i32 - 2 * m::M);
    if let Some(pos) = cursor {
        caret(c, x + pos as i32 * m::FONT_ADVANCE, y, t.accent);
    }
}

/// List row. The caller paints the list surface first.
pub fn row(c: &mut Canvas<'_>, r: Rect, label: &str, icon: Option<Glyph>, state: State, t: Theme) {
    let h = r.height as i32;
    let (ink, mark, style) = match state {
        State::Selected => {
            rect(c, r.x, r.y, r.width as i32, h, t.accent_soft);
            rect(c, r.x, r.y, 2, h, t.accent);
            (t.text, t.accent, Style::Strong)
        }
        State::Focused => {
            outlined(c, r, t.header, t.accent, m::RADIUS);
            (t.text, t.secondary, Style::Body)
        }
        State::Disabled => (t.disabled, t.disabled, Style::Body),
        State::Refused => (t.error, t.error, Style::Body),
        State::Normal => (t.text, t.muted, Style::Body),
    };
    let icon = if state == State::Refused {
        Some(Glyph::Error)
    } else {
        icon
    };
    let y = r.y + (h - m::FONT_HEIGHT) / 2;
    let mut x = r.x + m::M + 2;
    if let Some(g) = icon {
        glyph(c, x, y - 1, g, mark);
        x += m::GLYPH_SMALL + 6;
    }
    text_fit(c, x, y, label, style, ink, r.x + r.width as i32 - x - m::M);
}

/// Binary switch: knob position and fill both encode the state.
pub fn switch(c: &mut Canvas<'_>, x: i32, y: i32, on: bool, state: State, t: Theme) {
    let r = Rect {
        x,
        y,
        width: m::SWITCH_WIDTH as u32,
        height: m::SWITCH_HEIGHT as u32,
    };
    let disabled = state == State::Disabled;
    let knob = m::SWITCH_HEIGHT - 4;
    if on {
        stadium(c, r, if disabled { t.disabled } else { t.accent });
        let k = Rect {
            x: x + m::SWITCH_WIDTH - knob - 2,
            y: y + 2,
            width: knob as u32,
            height: knob as u32,
        };
        stadium(c, k, if disabled { t.control } else { t.on_accent });
    } else {
        stadium(c, r, if disabled { t.disabled } else { t.field_edge });
        stadium(c, inset(r, 1), t.track);
        let k = Rect {
            x: x + 2,
            y: y + 2,
            width: knob as u32,
            height: knob as u32,
        };
        stadium(c, k, if disabled { t.disabled } else { t.field_edge });
        stadium(c, inset(k, 1), t.field);
    }
    if state == State::Focused {
        let ring = Rect {
            x: x - 3,
            y: y - 3,
            width: (m::SWITCH_WIDTH + 6) as u32,
            height: (m::SWITCH_HEIGHT + 6) as u32,
        };
        outlined(c, ring, t.elevated, t.accent, m::RADIUS_PANEL);
    }
}

/// Determinate progress / ratio meter. `value` is clamped to 0..=100.
pub fn progress(c: &mut Canvas<'_>, r: Rect, value: u8, t: Theme) {
    meter(c, r, u64::from(value.min(100)), 100, t.accent, t);
}

pub fn meter(c: &mut Canvas<'_>, r: Rect, part: u64, whole: u64, color: u32, t: Theme) {
    stadium(c, r, t.track);
    if whole == 0 || part == 0 {
        return;
    }
    let w = (u128::from(r.width) * u128::from(part.min(whole)) / u128::from(whole)) as u32;
    let w = w.max(r.height.min(r.width));
    stadium(
        c,
        Rect {
            x: r.x,
            y: r.y,
            width: w,
            height: r.height,
        },
        color,
    );
}

/// Pill badge with caption text. Returns its width.
pub fn chip(c: &mut Canvas<'_>, x: i32, y: i32, label: &str, tone: Tone, t: Theme) -> i32 {
    let (strong, soft) = tone.colors(t);
    let w = measure(label, Style::Caption) + 2 * m::M;
    let r = Rect {
        x,
        y,
        width: w as u32,
        height: 13,
    };
    if tone == Tone::Neutral {
        outlined(c, r, t.elevated, t.divider, m::RADIUS);
    } else {
        rounded(c, r, soft, m::RADIUS);
    }
    text(c, x + m::M, y + 3, label, Style::Caption, strong);
    w
}

/// Window status band from `y` to the bottom of the canvas.
pub fn status_band(c: &mut Canvas<'_>, y: i32, message: &str, tone: Tone, t: Theme) {
    let (w, h) = c.size();
    let (w, h) = (w as i32, h as i32);
    let (strong, soft) = tone.colors(t);
    let band = match tone {
        Tone::Error | Tone::Warning => soft,
        _ => t.header,
    };
    rect(c, 0, y, w, h - y, band);
    hline(
        c,
        0,
        y,
        w,
        if band == t.header { t.divider } else { strong },
    );
    let ty = y + (h - y - m::FONT_HEIGHT) / 2;
    let ink = match tone {
        Tone::Neutral => t.secondary,
        _ => strong,
    };
    let mark = match tone {
        Tone::Neutral => t.muted,
        _ => strong,
    };
    glyph(c, m::CONTENT_INSET, ty - 1, tone.glyph(), mark);
    text_fit(
        c,
        m::CONTENT_INSET + m::GLYPH_SMALL + 6,
        ty,
        message,
        Style::Caption,
        ink,
        w - 2 * m::CONTENT_INSET - m::GLYPH_SMALL - 6,
    );
}

/// Section label: caption in muted ink with an optional trailing rule.
pub fn section(c: &mut Canvas<'_>, x: i32, y: i32, label: &str, rule_to: Option<i32>, t: Theme) {
    let end = text(c, x, y, label, Style::Caption, t.muted);
    if let Some(right) = rule_to {
        hline(c, end + m::S, y + 3, right - end - m::S, t.divider);
    }
}
