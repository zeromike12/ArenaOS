//! Designer-editable application geometry. Logic uses these hit regions too.
//!
//! Window anatomy (local pixels, 448x288):
//!   0..28    title bar (compositor chrome)
//!   28..66   header band: toolbar controls or a page header
//!   66       header divider
//!   67..264  content
//!   264..288 status band
//! The editor's text origin stays at EDIT_TEXT + 6 because the controller's
//! pointer-to-caret mapping uses that inset.
use arena_gfxkit::Rect;
use arena_ui::metrics as m;
pub const TOOL_Y: i32 = m::TITLE_HEIGHT + 6;
pub const TOOL_H: i32 = m::CONTROL_HEIGHT;
pub const HEADER_BOTTOM: i32 = m::HEADER_BOTTOM;
pub const CONTENT_Y: i32 = HEADER_BOTTOM + m::XS;
pub const ROW_H: i32 = m::ROW_HEIGHT;
pub const STATUS_Y: i32 = m::FOOTER_Y;
/// Right-hand header area used for document / page facts.
pub const HEADER_INFO_X: i32 = 320;
const fn tool(x: i32, width: u32) -> Rect {
    Rect {
        x,
        y: TOOL_Y,
        width,
        height: TOOL_H as u32,
    }
}
pub const NAME_FIELD: Rect = tool(m::CONTENT_INSET, 230);
pub const PRIMARY: Rect = tool(250, 82);
pub const SECONDARY: Rect = tool(340, 96);
pub const NEW: Rect = tool(m::CONTENT_INSET, 60);
pub const SAVE: Rect = tool(80, 68);
pub const SAVE_AS: Rect = tool(156, 80);
pub const OPEN: Rect = tool(244, 68);
pub const DELETE: Rect = tool(320, 68);
/// Files: list pane (left) and preview pane (right).
pub const FILE_LIST: Rect = Rect {
    x: 8,
    y: CONTENT_Y,
    width: 204,
    height: (FILE_ROWS as i32 * ROW_H) as u32,
};
pub const PREVIEW: Rect = Rect {
    x: 224,
    y: CONTENT_Y,
    width: 212,
    height: (STATUS_Y - CONTENT_Y - m::M) as u32,
};
/// Preview text starts below the preview's name/size header.
pub const PREVIEW_TEXT_Y: i32 = PREVIEW.y + 28;
pub const EDIT_TEXT: Rect = Rect {
    x: m::CONTENT_INSET,
    y: CONTENT_Y,
    width: 424,
    height: (STATUS_Y - CONTENT_Y - m::M) as u32,
};
/// Settings: each whole preference row is its toggle target.
pub const APPEARANCE: Rect = Rect {
    x: m::CONTENT_INSET,
    y: 82,
    width: 424,
    height: 35,
};
pub const MOTION: Rect = Rect {
    x: m::CONTENT_INSET,
    y: 117,
    width: 424,
    height: 35,
};
/// Terminal: console fills the content area; the input line sits at its foot.
pub const INPUT_Y: i32 = STATUS_Y - 24;
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
pub const FILE_ROWS: usize = ((STATUS_Y - CONTENT_Y - m::M) / ROW_H) as usize;
pub const PREVIEW_ROWS: usize =
    ((PREVIEW.y + PREVIEW.height as i32 - PREVIEW_TEXT_Y - m::S) / m::LINE_HEIGHT) as usize;
pub const PREVIEW_COLUMNS: usize =
    ((PREVIEW.width as i32 - m::SPACE[4]) / m::FONT_ADVANCE) as usize;
pub const TERMINAL_ROWS: usize = ((INPUT_Y - m::S - (CONTENT_Y + 6)) / m::LINE_HEIGHT) as usize;
/// Process rows in the Monitor list (header row excluded).
pub const MONITOR_ROW_Y: i32 = CONTENT_Y + 28;
pub const MONITOR_ROWS: usize = ((STATUS_Y - m::S - MONITOR_ROW_Y) / m::LINE_HEIGHT) as usize;
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

#[cfg(test)]
mod tests {
    use super::*;
    // Interaction points used by the guest suites (window-local).
    #[test]
    fn controls_keep_tested_interaction_points() {
        assert!(hit(NEW, 35, 50) && hit(SAVE, 115, 50) && hit(SAVE_AS, 195, 50));
        assert!(hit(OPEN, 275, 50) && hit(DELETE, 350, 50));
        assert!(hit(PRIMARY, 290, 50) && hit(SECONDARY, 380, 50));
        assert!(hit(FILE_LIST, 40, 82) && (82 - FILE_LIST.y) / ROW_H == 0);
        assert!(hit(APPEARANCE, 200, 100) && hit(MOTION, 200, 140));
        assert!(!hit(APPEARANCE, 200, 140) && !hit(MOTION, 200, 100));
        assert_eq!((EDIT_TEXT.x + 6, EDIT_TEXT.y + 6), (18, 74));
        // Every tool control stays inside the header band.
        for r in [
            NAME_FIELD, PRIMARY, SECONDARY, NEW, SAVE, SAVE_AS, OPEN, DELETE,
        ] {
            assert!(r.y >= m::TITLE_HEIGHT && r.y + r.height as i32 <= HEADER_BOTTOM);
            assert!(r.x + r.width as i32 <= m::WINDOW_WIDTH as i32 - m::CONTENT_INSET);
        }
        assert!(FILE_LIST.y + FILE_LIST.height as i32 <= STATUS_Y);
        assert!(MONITOR_ROW_Y + MONITOR_ROWS as i32 * m::LINE_HEIGHT <= STATUS_Y);
        assert!(CONTENT_Y + 6 + TERMINAL_ROWS as i32 * m::LINE_HEIGHT <= INPUT_Y);
    }
}
