//! Designer-editable application geometry. Logic uses these hit regions too.
use arena_gfxkit::Rect;
use arena_ui::metrics as m;
pub const TOOL_Y: i32 = m::TITLE_HEIGHT + m::SPACE[2];
pub const TOOL_H: i32 = m::CONTROL_HEIGHT;
pub const CONTENT_Y: i32 = TOOL_Y + TOOL_H + m::SPACE[2];
pub const ROW_H: i32 = m::LINE_HEIGHT + 4;
pub const STATUS_Y: i32 = m::WINDOW_HEIGHT as i32 - 20;
pub const NAME_FIELD: Rect = Rect {
    x: m::CONTENT_INSET,
    y: TOOL_Y,
    width: 230,
    height: TOOL_H as u32,
};
pub const PRIMARY: Rect = Rect {
    x: 250,
    y: TOOL_Y,
    width: 82,
    height: TOOL_H as u32,
};
pub const SECONDARY: Rect = Rect {
    x: 340,
    y: TOOL_Y,
    width: 96,
    height: TOOL_H as u32,
};
pub const NEW: Rect = Rect {
    x: m::CONTENT_INSET,
    y: TOOL_Y,
    width: 60,
    height: TOOL_H as u32,
};
pub const SAVE: Rect = Rect {
    x: 80,
    y: TOOL_Y,
    width: 68,
    height: TOOL_H as u32,
};
pub const SAVE_AS: Rect = Rect {
    x: 156,
    y: TOOL_Y,
    width: 80,
    height: TOOL_H as u32,
};
pub const OPEN: Rect = Rect {
    x: 244,
    y: TOOL_Y,
    width: 68,
    height: TOOL_H as u32,
};
pub const DELETE: Rect = Rect {
    x: 320,
    y: TOOL_Y,
    width: 68,
    height: TOOL_H as u32,
};
pub const FILE_LIST: Rect = Rect {
    x: m::CONTENT_INSET,
    y: CONTENT_Y,
    width: 200,
    height: (STATUS_Y - CONTENT_Y - 6) as u32,
};
pub const PREVIEW: Rect = Rect {
    x: 220,
    y: CONTENT_Y,
    width: 216,
    height: (STATUS_Y - CONTENT_Y - 6) as u32,
};
pub const EDIT_TEXT: Rect = Rect {
    x: m::CONTENT_INSET,
    y: CONTENT_Y,
    width: 424,
    height: (STATUS_Y - CONTENT_Y - 6) as u32,
};
pub const APPEARANCE: Rect = Rect {
    x: 128,
    y: CONTENT_Y + 24,
    width: 240,
    height: TOOL_H as u32,
};
pub const MOTION: Rect = Rect {
    x: 128,
    y: CONTENT_Y + 64,
    width: 240,
    height: TOOL_H as u32,
};
pub fn hit(r: Rect, x: i32, y: i32) -> bool {
    x >= r.x
        && y >= r.y
        && (x as i64) < i64::from(r.x) + i64::from(r.width)
        && (y as i64) < i64::from(r.y) + i64::from(r.height)
}
