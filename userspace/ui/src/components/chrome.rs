//! Window chrome, shared by the compositor and by clients' own rasters so
//! there is exactly one definition of what an ArenaOS window looks like.
//!
//! Anatomy: square-cornered frame (the compositor cannot see what lies
//! under a window corner, so it never pretends to round one), a 2px Signal
//! rail across the top edge, a focus lamp, a double-struck title and a
//! close well inside the 28px close strip. Focus is shown four ways at once
//! — rail, lamp fill, frame tone and title ink — plus a deeper drop ledge,
//! so it never depends on colour alone.
use super::{
    icons::{Glyph, glyph, lamp},
    shapes::{border, outlined, rect},
    text::{Style, text_fit},
};
use crate::{metrics as m, motion::color, theme::Theme};
use arena_gfxkit::{Canvas, Rect};

const FULL: i32 = 65_536;

/// `amount` is the shared focus motion value: 0 resting, 65536 focused.
#[allow(clippy::too_many_arguments)]
pub fn window_chrome(
    c: &mut Canvas<'_>,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    title: &str,
    amount: i32,
    t: Theme,
) {
    let a = amount.clamp(0, FULL);
    // Opaque drop ledge: deeper for the focused window (stacking cue).
    let s = m::SHADOW_RESTING + ((m::SHADOW_FOCUSED - m::SHADOW_RESTING) * a + FULL / 2) / FULL;
    rect(c, x + s, y + h, w, s, t.shadow);
    rect(c, x + w, y + s, s, h, t.shadow);
    // Title bar shares the header material with the app's own header band.
    rect(c, x, y, w, m::TITLE_HEIGHT, t.header);
    let frame = color(t.frame, t.frame_focus, a);
    border(
        c,
        Rect {
            x,
            y,
            width: w as u32,
            height: h as u32,
        },
        frame,
    );
    // Signal rail: second pixel row joins the frame's top edge when focused.
    rect(
        c,
        x + 1,
        y + 1,
        w - 2,
        m::FOCUS_RAIL - 1,
        color(t.header, t.accent, a),
    );
    lamp(c, x + m::L, y + 11, a, t.header, t);
    let close_x = x + w - m::CLOSE_WIDTH;
    text_fit(
        c,
        x + 28,
        y + 11,
        title,
        Style::Strong,
        color(t.secondary, t.text, a),
        close_x - (x + 28) - m::S,
    );
    // Close well: always visible, quieter at rest.
    let well = Rect {
        x: close_x + 4,
        y: y + 6,
        width: 19,
        height: 18,
    };
    outlined(
        c,
        well,
        color(t.header, t.control, a),
        color(t.divider, t.control_edge, a),
        m::RADIUS,
    );
    glyph(
        c,
        well.x + 5,
        well.y + 4,
        Glyph::Close,
        color(t.muted, t.secondary, a),
    );
}

/// Client-side chrome at the canvas origin (the compositor draws the same
/// chrome over the published raster).
pub fn chrome(c: &mut Canvas<'_>, title: &str, focused: bool, t: Theme) {
    let (w, h) = c.size();
    window_chrome(
        c,
        0,
        0,
        w as i32,
        h as i32,
        title,
        if focused { FULL } else { 0 },
        t,
    );
}

/// Depth of the opaque drop ledge under a transient surface (menu, tooltip,
/// dialog). Part of the surface's damage bounds.
pub const POPUP_SHADOW: i32 = 2;

/// Frame of a transient surface drawn by the compositor around the
/// client's published pixels: a hairline border inside the surface edge
/// and an opaque drop ledge outside it, so it reads as floating above its
/// owner window.
pub fn popup_frame(c: &mut Canvas<'_>, x: i32, y: i32, w: i32, h: i32, t: Theme) {
    rect(c, x + POPUP_SHADOW, y + h, w, POPUP_SHADOW, t.shadow);
    rect(c, x + w, y + POPUP_SHADOW, POPUP_SHADOW, h, t.shadow);
    border(
        c,
        Rect {
            x,
            y,
            width: w as u32,
            height: h as u32,
        },
        t.frame_focus,
    );
}
