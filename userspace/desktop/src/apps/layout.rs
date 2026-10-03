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

// Viewport dimensions are shared by painting, scrolling and pointer-to-caret
// conversion, so font/spacing changes do not require application IPC edits.
pub const EDIT_COLUMNS: usize = ((EDIT_TEXT.width as i32 - m::SPACE[3]) / m::FONT_ADVANCE) as usize;
pub const EDIT_ROWS: usize = ((EDIT_TEXT.height as i32 - m::SPACE[3]) / m::LINE_HEIGHT) as usize;
pub const FILE_ROWS: usize = FILE_LIST.height as usize / ROW_H as usize;
pub const PREVIEW_ROWS: usize = ((PREVIEW.height as i32 - m::SPACE[4]) / m::LINE_HEIGHT) as usize;
pub const PREVIEW_COLUMNS: usize =
    ((PREVIEW.width as i32 - m::SPACE[4]) / m::FONT_ADVANCE) as usize;
pub const TERMINAL_ROWS: usize = ((STATUS_Y - 20 - (CONTENT_Y + 6)) / m::LINE_HEIGHT) as usize;
pub const MONITOR_ROWS: usize =
    ((STATUS_Y - CONTENT_Y - m::SPACE[4] - m::SPACE[3]) / m::LINE_HEIGHT) as usize;
pub fn visual_cursor(e: &super::model::Editor, target_row: usize, target_column: usize) -> usize {
    let mut row = 0;
    let mut column = 0;
    for i in 0..=e.len {
        if row > target_row || (row == target_row && column >= target_column) || i == e.len {
            return i;
        }
        let b = e.data[i];
        if b == b'\n' {
            if row == target_row {
                return i;
            }
            row += 1;
            column = 0;
        } else {
            column += if b == b'\t' { 4 - column % 4 } else { 1 };
            if column >= EDIT_COLUMNS {
                row += 1;
                column = 0;
            }
        }
    }
    e.len
}
pub fn visual_row(e: &super::model::Editor) -> usize {
    let mut row = 0;
    let mut col = 0;
    for b in &e.data[..e.cursor] {
        if *b == b'\n' {
            row += 1;
            col = 0;
        } else {
            col += if *b == b'\t' { 4 - col % 4 } else { 1 };
            if col >= EDIT_COLUMNS {
                row += 1;
                col = 0;
            }
        }
    }
    row
}
