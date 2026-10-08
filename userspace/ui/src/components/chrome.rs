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

/// Whether one more 28px title-bar well fits left of `edge` in a window
/// starting at `x` (it must stay right of the focus lamp strip).
pub const fn control_fits(x: i32, edge: i32) -> bool {
    edge - m::CLOSE_WIDTH >= x + 24
}

/// Title-bar controls beside the close well (Phase 11.4). `hover` names
/// the well under the pointer: 1 close, 2 maximize, 3 minimize.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Controls {
    pub minimize: bool,
    pub maximize: bool,
    pub maximized: bool,
    pub hover: u8,
}
impl Controls {
    /// Close only (a client drawing chrome into its own raster).
    pub const CLOSE: Controls = Controls {
        minimize: false,
        maximize: false,
        maximized: false,
        hover: 0,
    };
}

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
    controls: Controls,
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
    // Maximize and minimize wells sit left of the close well, each in its
    // own 28px strip (the window policy's hit zones).
    let mut left = close_x;
    for (shown, kind) in [(controls.maximize, 2u8), (controls.minimize, 3u8)] {
        // A well is drawn (and hit-tested by the window policy) only where
        // it fits right of the focus lamp: `control_fits`.
        if !shown || !control_fits(x, left) {
            continue;
        }
        left -= m::CLOSE_WIDTH;
        let well = Rect {
            x: left + 4,
            y: y + 6,
            width: 19,
            height: 18,
        };
        let hot = controls.hover == kind;
        outlined(
            c,
            well,
            if hot {
                t.control
            } else {
                color(t.header, t.control, a)
            },
            if hot {
                t.control_edge
            } else {
                color(t.divider, t.control_edge, a)
            },
            m::RADIUS,
        );
        let ink = if hot {
            t.text
        } else {
            color(t.muted, t.secondary, a)
        };
        let (gx, gy) = (well.x + 5, well.y + 4);
        if kind == 3 {
            rect(c, gx, gy + 8, 9, 2, ink);
        } else if controls.maximized {
            // Restore: two offset frames.
            border(
                c,
                Rect {
                    x: gx + 2,
                    y: gy,
                    width: 7,
                    height: 6,
                },
                ink,
            );
            rect(c, gx, gy + 3, 7, 7, color(t.header, t.control, a));
            border(
                c,
                Rect {
                    x: gx,
                    y: gy + 3,
                    width: 7,
                    height: 7,
                },
                ink,
            );
        } else {
            border(
                c,
                Rect {
                    x: gx,
                    y: gy + 1,
                    width: 9,
                    height: 8,
                },
                ink,
            );
            rect(c, gx, gy + 1, 9, 2, ink);
        }
    }
    text_fit(
        c,
        x + 28,
        y + 11,
        title,
        Style::Strong,
        color(t.secondary, t.text, a),
        left - (x + 28) - m::S,
    );
    // Close well: always visible, quieter at rest.
    let well = Rect {
        x: close_x + 4,
        y: y + 6,
        width: 19,
        height: 18,
    };
    let hot = controls.hover == 1;
    outlined(
        c,
        well,
        if hot {
            t.control
        } else {
            color(t.header, t.control, a)
        },
        if hot {
            t.control_edge
        } else {
            color(t.divider, t.control_edge, a)
        },
        m::RADIUS,
    );
    glyph(
        c,
        well.x + 5,
        well.y + 4,
        Glyph::Close,
        if hot {
            t.text
        } else {
            color(t.muted, t.secondary, a)
        },
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
        Controls::CLOSE,
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
