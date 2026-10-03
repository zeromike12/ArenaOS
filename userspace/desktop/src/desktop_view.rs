//! Designer-editable desktop layout: no IPC, lifecycle or device authority.
use arena_desktop::model::{State, Window};
use arena_gfxkit::{Canvas, Rect};
use arena_ui::{components as c, metrics as m, theme::Theme};
pub fn background(canvas: &mut Canvas<'_>, t: Theme) {
    canvas.clear(t.background);
    let (w, h) = canvas.size();
    c::label(
        canvas,
        (w as i32 - 18 * m::FONT_ADVANCE) / 2,
        (h as i32) / 2,
        "ARENAOS DESKTOP",
        t.secondary,
    );
}
pub fn chrome(
    canvas: &mut Canvas<'_>,
    window: &Window,
    title: &str,
    focused: bool,
    amount: i32,
    t: Theme,
) {
    c::rect(
        canvas,
        window.x,
        window.y,
        window.width as i32,
        m::TITLE_HEIGHT,
        arena_ui::motion::color(t.chrome_inactive, t.chrome_active, amount),
    );
    c::heading(
        canvas,
        window.x + m::CONTENT_INSET,
        window.y + (m::TITLE_HEIGHT - m::TITLE_FONT_HEIGHT) / 2,
        &title[..title.len().min(
            ((window.width as i32 - m::CONTENT_INSET - m::CLOSE_WIDTH).max(0)
                / m::TITLE_FONT_ADVANCE) as usize,
        )],
        t.text,
    );
    c::label(
        canvas,
        window.x + window.width as i32 - m::CLOSE_WIDTH + 10,
        window.y + 10,
        "X",
        t.secondary,
    );
    c::border(
        canvas,
        Rect {
            x: window.x,
            y: window.y,
            width: window.width as u32,
            height: window.height as u32,
        },
        if focused { t.accent } else { t.border },
    );
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
    c::rect(canvas, 0, 0, w as i32, m::SYSTEM_BAR_HEIGHT, t.panel);
    c::label(canvas, m::CONTENT_INSET, 10, "ARENAOS", t.text);
    c::label(
        canvas,
        100,
        10,
        active
            .map(|kind| arena_desktop::apps::TITLES[kind as usize])
            .or_else(|| state.focused().map(|_| "APPLICATION"))
            .unwrap_or("DESKTOP"),
        t.secondary,
    );
    // Actual monotonic uptime supplied by the service; no wall-clock claim.
    let mut digits = [0u8; 20];
    let mut n = 0;
    let mut value = uptime;
    loop {
        digits[n] = b'0' + (value % 10) as u8;
        n += 1;
        value /= 10;
        if value == 0 {
            break;
        }
    }
    let mut label = [0u8; 24];
    label[..3].copy_from_slice(b"UP ");
    for (i, digit) in digits[..n].iter().rev().enumerate() {
        label[i + 3] = *digit;
    }
    label[n + 3] = b'S';
    c::label(
        canvas,
        w as i32 - (n as i32 + 4) * m::FONT_ADVANCE - m::CONTENT_INSET,
        10,
        core::str::from_utf8(&label[..n + 4]).unwrap_or("UP"),
        t.secondary,
    );
    let width = m::DOCK_ITEM_WIDTH * 6;
    let dx = (w as i32 - width) / 2;
    c::rect(
        canvas,
        dx,
        h as i32 - m::DOCK_HEIGHT,
        width,
        m::DOCK_HEIGHT,
        t.elevated,
    );
    for (i, label) in arena_desktop::apps::DOCK.iter().enumerate() {
        let x = dx + i as i32 * m::DOCK_ITEM_WIDTH;
        c::icon(canvas, x + 20, h as i32 - m::DOCK_HEIGHT + 8, i as u8, t);
        c::label(canvas, x + 8, h as i32 - 16, label, t.text);
        if running[i] > 0 {
            c::rect(
                canvas,
                x + 24,
                h as i32 - 6,
                10,
                2,
                if active == Some(i as u8) {
                    t.accent
                } else {
                    t.secondary
                },
            );
        }
    }
    if let Some(text) = notice {
        c::rect(
            canvas,
            0,
            m::SYSTEM_BAR_HEIGHT,
            w as i32,
            m::CONTROL_HEIGHT,
            t.elevated,
        );
        c::label(
            canvas,
            m::CONTENT_INSET,
            m::SYSTEM_BAR_HEIGHT + 8,
            text,
            t.error,
        );
    }
    let (x, y) = state.pointer;
    c::rect(canvas, x, y, 2, 12, t.text);
    c::rect(canvas, x, y, 8, 2, t.text);
}
