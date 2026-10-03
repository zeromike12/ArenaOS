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
pub fn chrome(canvas: &mut Canvas<'_>, window: &Window, title: &str, focused: bool, t: Theme) {
    c::rect(
        canvas,
        window.x,
        window.y,
        window.width as i32,
        m::TITLE_HEIGHT,
        if focused {
            t.chrome_active
        } else {
            t.chrome_inactive
        },
    );
    c::label(
        canvas,
        window.x + m::CONTENT_INSET,
        window.y + 10,
        title,
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
pub fn system(canvas: &mut Canvas<'_>, state: &State, t: Theme) {
    let (w, h) = canvas.size();
    c::rect(canvas, 0, 0, w as i32, m::SYSTEM_BAR_HEIGHT, t.panel);
    c::label(canvas, m::CONTENT_INSET, 10, "ARENAOS", t.text);
    c::label(
        canvas,
        100,
        10,
        if state.focused().is_some() {
            "APPLICATION ACTIVE"
        } else {
            "DESKTOP"
        },
        t.secondary,
    );
    let dx = (w as i32 - m::DOCK_ITEM_WIDTH) / 2;
    c::rect(
        canvas,
        dx,
        h as i32 - m::DOCK_HEIGHT,
        m::DOCK_ITEM_WIDTH,
        m::DOCK_HEIGHT,
        t.elevated,
    );
    c::icon(canvas, dx + 20, h as i32 - m::DOCK_HEIGHT + 8, 0, t);
    c::label(canvas, dx + 8, h as i32 - 16, "GALLERY", t.text);
    let (x, y) = state.pointer;
    c::rect(canvas, x, y, 2, 12, t.text);
    c::rect(canvas, x, y, 8, 2, t.text);
}
