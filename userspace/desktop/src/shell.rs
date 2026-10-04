//! Designer-editable desktop shell: no IPC, lifecycle or device authority.
//!
//! Draw order is fixed by the compositor (see compose.rs): background, then
//! each window (owned raster + chrome), then `system` (bar, notice, dock,
//! pointer). Shell surfaces drawn by `system` leave their rounded corners
//! unpainted, so windows or the desktop show through them honestly.
//!
//! Every input `system` draws from is a field of [`Shell`], and every pixel
//! it can touch lies inside one of the region functions below; compose.rs
//! relies on both facts to repaint only what changed.
use crate::{
    apps::{DOCK, TITLES},
    model::MAX_WINDOWS,
};
use arena_gfxkit::{Canvas, Rect};
use arena_ui::{
    components::{self as c, Glyph, Style},
    metrics as m,
    theme::Theme,
};

/// Empty desktop: a plain field with the Arena emblem and the real
/// keyboard bindings. Nothing here is decorative hardware state.
pub fn background(canvas: &mut Canvas<'_>, t: Theme) {
    canvas.clear(t.desktop);
    let (w, h) = canvas.size();
    let (w, h) = (w as i32, h as i32);
    let cx = w / 2;
    let cy = (m::SYSTEM_BAR_HEIGHT + h - m::DOCK_HEIGHT) / 2 - 12;
    let ew = (w * 3 / 10).min(240);
    let eh = ew * 11 / 20;
    c::emblem(
        canvas,
        Rect {
            x: cx - ew / 2,
            y: cy - eh / 2,
            width: ew as u32,
            height: eh as u32,
        },
        t.desktop_mark,
        t.desktop,
    );
    let hint = "F1-F6 open  /  F7 next window  /  F8 close";
    c::text_centered(
        canvas,
        0,
        w,
        cy + eh / 2 + 20,
        hint,
        Style::Caption,
        t.desktop_text,
    );
}

/// Descriptive facts the shell draws. Comparable, so the compositor can
/// tell exactly which shell regions changed between frames.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Shell {
    pub pointer: (i32, i32),
    /// Windows currently open (all sessions with a window).
    pub open: usize,
    /// Some window has keyboard focus (possibly a non-builtin one).
    pub focused_any: bool,
    /// Running sessions per built-in kind (including not-yet-shown ones).
    pub running: [u8; 6],
    /// Kind of the focused built-in application.
    pub active: Option<u8>,
    pub notice: Option<&'static str>,
    pub uptime: u64,
}

impl Shell {
    pub const EMPTY: Shell = Shell {
        pointer: (0, 0),
        open: 0,
        focused_any: false,
        running: [0; 6],
        active: None,
        notice: None,
        uptime: 0,
    };
}

fn rect(x: i32, y: i32, w: i32, h: i32) -> Rect {
    Rect {
        x,
        y,
        width: w.max(0) as u32,
        height: h.max(0) as u32,
    }
}

/// Region holding the system bar.
pub fn bar_region(w: i32) -> Rect {
    rect(0, 0, w, m::SYSTEM_BAR_HEIGHT)
}

/// Region holding the transient notice toast and its ledge.
pub fn notice_region(w: i32) -> Rect {
    rect(0, m::SYSTEM_BAR_HEIGHT, w, 30)
}

fn dock_x(w: i32) -> i32 {
    (w - m::DOCK_ITEM_WIDTH * DOCK.len() as i32) / 2
}

/// Region holding the dock panel, its ledge and hover wells.
pub fn dock_region(w: i32, h: i32) -> Rect {
    let strip = m::DOCK_ITEM_WIDTH * DOCK.len() as i32;
    rect(
        dock_x(w) - 6,
        h - m::DOCK_HEIGHT,
        strip + 12,
        m::DOCK_HEIGHT,
    )
}

/// Region covered by the pointer drawn at `(x, y)`.
pub fn pointer_region(x: i32, y: i32) -> Rect {
    rect(x, y, c::POINTER_WIDTH, c::POINTER_HEIGHT)
}

/// Dock item under the pointer, if any (drawn as a hover well).
pub fn hover_item(w: i32, h: i32, (px, py): (i32, i32)) -> Option<usize> {
    let x0 = dock_x(w);
    let strip = m::DOCK_ITEM_WIDTH * DOCK.len() as i32;
    (py >= h - m::DOCK_HEIGHT && px >= x0 && px < x0 + strip)
        .then(|| ((px - x0) / m::DOCK_ITEM_WIDTH) as usize)
}

/// Formats monotonic seconds as H:MM:SS (an uptime counter, not a clock).
fn uptime_text(out: &mut [u8; 32], seconds: u64) -> &str {
    let mut digits = [0u8; 32];
    let hours = c::decimal(&mut digits, seconds / 3600, false);
    let mut n = hours.len();
    out[..n].copy_from_slice(hours.as_bytes());
    for part in [(seconds / 60) % 60, seconds % 60] {
        out[n] = b':';
        out[n + 1] = b'0' + (part / 10) as u8;
        out[n + 2] = b'0' + (part % 10) as u8;
        n += 3;
    }
    core::str::from_utf8(&out[..n]).unwrap_or("?")
}

pub fn system(canvas: &mut Canvas<'_>, shell: &Shell, t: Theme) {
    let Shell {
        pointer,
        open,
        focused_any,
        running,
        active,
        notice,
        uptime,
    } = *shell;
    let (w, h) = canvas.size();
    let (w, h) = (w as i32, h as i32);
    let bar = m::SYSTEM_BAR_HEIGHT;
    let ty = (bar - 1 - m::FONT_HEIGHT) / 2;
    // System bar: identity, active application, real session/uptime facts.
    c::rect(canvas, 0, 0, w, bar - 1, t.bar);
    c::hline(canvas, 0, bar - 1, w, t.bar_edge);
    // Wordmark only: a small capsule mark here could be misread as a
    // battery or toggle indicator, which ArenaOS does not have.
    let x = c::text(canvas, m::L, ty, "Arena", Style::Strong, t.bar_text);
    let x = c::text(canvas, x, ty, "OS", Style::Strong, t.accent);
    c::vline(canvas, x + m::M, 7, bar - 14, t.bar_edge);
    let x = x + m::M + 1 + m::M + 1;
    match active {
        Some(kind) => {
            c::app_tile(canvas, x, ty - 2, 11, kind, false, t.bar, t);
            c::text(
                canvas,
                x + 17,
                ty,
                TITLES[kind as usize],
                Style::Body,
                t.bar_text,
            );
        }
        None if focused_any => {
            c::glyph(canvas, x + 1, ty - 1, Glyph::Idle, t.bar_muted);
            c::text(canvas, x + 17, ty, "Application", Style::Body, t.bar_text);
        }
        None => {
            c::text(canvas, x, ty, "Desktop", Style::Body, t.bar_muted);
        }
    }
    let mut buffer = [0u8; 32];
    let value = uptime_text(&mut buffer, uptime);
    let right = w - m::L;
    c::text_right(canvas, right, ty, value, Style::Body, t.bar_text);
    let x = right - c::measure(value, Style::Body) - m::S - 2;
    c::text_right(canvas, x, ty, "UPTIME", Style::Caption, t.bar_muted);
    let x = x - c::measure("UPTIME", Style::Caption) - m::L;
    c::vline(canvas, x, 7, bar - 14, t.bar_edge);
    let mut count = [0u8; 32];
    let n = c::decimal(&mut count, open as u64, false).len();
    count[n] = b'/';
    let mut max = [0u8; 32];
    let digits = c::decimal(&mut max, MAX_WINDOWS as u64, false).as_bytes();
    count[n + 1..n + 1 + digits.len()].copy_from_slice(digits);
    let count = core::str::from_utf8(&count[..n + 1 + digits.len()]).unwrap_or("?");
    let x = x - m::L;
    c::text_right(
        canvas,
        x,
        ty,
        count,
        Style::Body,
        if open >= MAX_WINDOWS {
            t.warning
        } else {
            t.bar_text
        },
    );
    let x = x - c::measure(count, Style::Body) - m::S - 2;
    c::text_right(canvas, x, ty, "WINDOWS", Style::Caption, t.bar_muted);

    // Transient shell notice (e.g. capacity refusal): an error toast.
    if let Some(text) = notice {
        let width = c::measure(text, Style::Caption) + m::GLYPH_SMALL + 3 * m::M + 2;
        let r = Rect {
            x: m::M,
            y: bar + 4,
            width: width as u32,
            height: 22,
        };
        c::rect(canvas, r.x + 2, r.y + 22, width, 2, t.shadow);
        c::outlined(canvas, r, t.error_soft, t.error, m::RADIUS_PANEL);
        c::glyph(canvas, r.x + m::M + 1, r.y + 6, Glyph::Error, t.error);
        c::text(
            canvas,
            r.x + m::M + m::GLYPH_SMALL + 7,
            r.y + 8,
            text,
            Style::Caption,
            t.error,
        );
    }

    // Dock: a floating panel inside the (unchanged) dock hit strip.
    let items = DOCK.len() as i32;
    let strip = m::DOCK_ITEM_WIDTH * items;
    let dock_x = (w - strip) / 2;
    let panel = Rect {
        x: dock_x - 6,
        y: h - m::DOCK_PANEL_BOTTOM - m::DOCK_PANEL_HEIGHT,
        width: (strip + 12) as u32,
        height: m::DOCK_PANEL_HEIGHT as u32,
    };
    c::rect(
        canvas,
        panel.x + 3,
        panel.y + panel.height as i32,
        panel.width as i32 - 3,
        2,
        t.shadow,
    );
    c::outlined(canvas, panel, t.dock, t.dock_edge, m::RADIUS_PANEL);
    let full = open >= MAX_WINDOWS;
    let (px, py) = pointer;
    let hover = hover_item(w, h, pointer);
    for (i, name) in DOCK.iter().enumerate() {
        let x = dock_x + i as i32 * m::DOCK_ITEM_WIDTH;
        let cx = x + m::DOCK_ITEM_WIDTH / 2;
        let hovered = hover == Some(i);
        let is_active = active == Some(i as u8);
        if hovered {
            c::well(
                canvas,
                Rect {
                    x: x + 2,
                    y: panel.y + 3,
                    width: (m::DOCK_ITEM_WIDTH - 4) as u32,
                    height: (m::DOCK_PANEL_HEIGHT - 6) as u32,
                },
                t.dock_well,
                None,
            );
        }
        let under = if hovered { t.dock_well } else { t.dock };
        c::app_tile(
            canvas,
            cx - m::TILE_SIZE / 2,
            panel.y + 4,
            m::TILE_SIZE,
            i as u8,
            full,
            under,
            t,
        );
        let (style, ink) = if is_active {
            (Style::Strong, t.bar_text)
        } else if running[i] > 0 || hovered {
            (Style::Body, t.bar_text)
        } else {
            (Style::Body, t.bar_muted)
        };
        c::text_centered(
            canvas,
            x,
            m::DOCK_ITEM_WIDTH,
            panel.y + 34,
            name,
            style,
            ink,
        );
        // Running instances: one dot each; the focused app gets a Signal bar.
        let iy = panel.y + 44;
        if is_active {
            c::rect(canvas, cx - 7, iy, 14, 2, t.accent);
        } else if running[i] > 0 {
            let n = i32::from(running[i].min(4));
            let start = cx - (n * 4 - 2) / 2;
            for k in 0..n {
                c::rect(canvas, start + k * 4, iy, 2, 2, t.bar_muted);
            }
        }
    }
    c::pointer(canvas, px, py, t);
}
