//! Designer-editable desktop layout: no IPC, lifecycle or device authority.
//!
//! Draw order is fixed by the compositor: background, then each window
//! (owned raster + `chrome`), then `system` (bar, notice, dock, pointer).
//! Shell surfaces drawn by `system` leave their rounded corners unpainted,
//! so windows or the desktop show through them honestly.
use arena_desktop::{
    apps::{DOCK, TITLES},
    model::{MAX_WINDOWS, State, Window},
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

pub fn chrome(
    canvas: &mut Canvas<'_>,
    window: &Window,
    title: &str,
    _focused: bool,
    amount: i32,
    t: Theme,
) {
    c::window_chrome(
        canvas,
        window.x,
        window.y,
        window.width as i32,
        window.height as i32,
        title,
        amount,
        t,
    );
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

pub fn system(
    canvas: &mut Canvas<'_>,
    state: &State,
    running: &[u8; 6],
    active: Option<u8>,
    notice: Option<&str>,
    uptime: u64,
    t: Theme,
) {
    let (w, h) = canvas.size();
    let (w, h) = (w as i32, h as i32);
    let bar = m::SYSTEM_BAR_HEIGHT;
    let ty = (bar - 1 - m::FONT_HEIGHT) / 2;
    // System bar: identity, active application, real session/uptime facts.
    c::rect(canvas, 0, 0, w, bar - 1, t.bar);
    c::hline(canvas, 0, bar - 1, w, t.bar_edge);
    c::emblem(
        canvas,
        Rect {
            x: m::L,
            y: ty - 1,
            width: 16,
            height: 9,
        },
        t.accent,
        t.bar,
    );
    let x = c::text(canvas, m::L + 22, ty, "ArenaOS", Style::Strong, t.bar_text);
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
        None if state.focused().is_some() => {
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
    let open = state.windows().count();
    let mut count = [0u8; 32];
    let n = c::decimal(&mut count, open as u64, false).len();
    count[n] = b'/';
    count[n + 1] = b'0' + MAX_WINDOWS as u8;
    let count = core::str::from_utf8(&count[..n + 2]).unwrap_or("?");
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
    let full = state.windows().count() >= MAX_WINDOWS;
    let (px, py) = state.pointer;
    for (i, name) in DOCK.iter().enumerate() {
        let x = dock_x + i as i32 * m::DOCK_ITEM_WIDTH;
        let cx = x + m::DOCK_ITEM_WIDTH / 2;
        let hovered = py >= h - m::DOCK_HEIGHT && px >= x && px < x + m::DOCK_ITEM_WIDTH;
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
