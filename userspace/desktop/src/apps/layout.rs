//! Designer-editable application geometry. Logic uses these hit regions too.
//!
//! Window anatomy (local pixels; the 448x288 default shown):
//!   0..28    title bar (compositor chrome)
//!   28..66   header band: toolbar controls or a page header
//!   66       header divider
//!   67..H-24 content (grows with the window)
//!   H-24..H  status band
//! Phase 11.3: windows are resizable, so the geometry is computed from the
//! surface size. At 448x288 every value equals the Phase-10 constant, so
//! the historical interaction points and pixel oracles are unchanged.
//! Left-anchored toolbar controls stay put; dialog buttons and header
//! facts are right-anchored; panes and text areas grow.
//! The editor's text origin stays at EDIT_TEXT + 6 because the controller's
//! pointer-to-caret mapping uses that inset.
use arena_gfxkit::Rect;
use arena_ui::metrics as m;
pub const TOOL_Y: i32 = m::TITLE_HEIGHT + 6;
pub const TOOL_H: i32 = m::CONTROL_HEIGHT;
pub const HEADER_BOTTOM: i32 = m::HEADER_BOTTOM;
pub const CONTENT_Y: i32 = HEADER_BOTTOM + m::XS;
pub const ROW_H: i32 = m::ROW_HEIGHT;
/// The smallest width every application lays out correctly at: the header
/// facts and the right-anchored controls need the default width.
pub const MIN_WIDTH: u16 = 448;
/// The smallest height of text-viewport applications (Terminal, Editor,
/// Files): a few rows remain visible.
pub const MIN_HEIGHT: u16 = 200;
/// Fixed-content pages (Settings, Monitor) keep the default height.
pub const MIN_PAGE_HEIGHT: u16 = 288;
/// Minimum surface of application `kind`.
pub fn min_size(kind: u8) -> (u16, u16) {
    match kind {
        super::SETTINGS | super::MONITOR => (MIN_WIDTH, MIN_PAGE_HEIGHT),
        _ => (MIN_WIDTH, MIN_HEIGHT),
    }
}
const fn tool(x: i32, width: u32) -> Rect {
    Rect {
        x,
        y: TOOL_Y,
        width,
        height: TOOL_H as u32,
    }
}
const fn preference_row(w: i32, y: i32) -> Rect {
    Rect {
        x: m::CONTENT_INSET,
        y,
        width: (w - 2 * m::CONTENT_INSET) as u32,
        height: 35,
    }
}
pub fn hit(r: Rect, x: i32, y: i32) -> bool {
    x >= r.x
        && y >= r.y
        && (x as i64) < i64::from(r.x) + i64::from(r.width)
        && (y as i64) < i64::from(r.y) + i64::from(r.height)
}
/// Geometry of one application surface size. Field names keep the
/// Phase-10 constant names so views read the same.
#[allow(non_snake_case)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Layout {
    pub W: i32,
    pub H: i32,
    pub TOOL_Y: i32,
    pub HEADER_BOTTOM: i32,
    pub CONTENT_Y: i32,
    pub ROW_H: i32,
    pub STATUS_Y: i32,
    /// Right-hand header area used for document / page facts.
    pub HEADER_INFO_X: i32,
    pub NAME_FIELD: Rect,
    pub PRIMARY: Rect,
    pub SECONDARY: Rect,
    pub NEW: Rect,
    pub SAVE: Rect,
    pub SAVE_AS: Rect,
    pub OPEN: Rect,
    pub DELETE: Rect,
    /// Files: list pane (left) and preview pane (right).
    pub FILE_LIST: Rect,
    pub PREVIEW: Rect,
    /// Preview text starts below the preview's name/size header.
    pub PREVIEW_TEXT_Y: i32,
    pub EDIT_TEXT: Rect,
    /// Settings: each whole preference row is its toggle target.
    pub APPEARANCE: Rect,
    pub MOTION: Rect,
    /// Terminal: console fills the content area; the input line sits at its foot.
    pub INPUT_Y: i32,
    // Viewport dimensions are shared by painting, scrolling and
    // pointer-to-caret conversion.
    pub EDIT_COLUMNS: usize,
    pub EDIT_ROWS: usize,
    pub FILE_ROWS: usize,
    pub PREVIEW_ROWS: usize,
    pub PREVIEW_COLUMNS: usize,
    pub TERMINAL_ROWS: usize,
    /// Process rows in the Monitor list (header row excluded).
    pub MONITOR_ROW_Y: i32,
    pub MONITOR_ROWS: usize,
}
impl Layout {
    pub const DEFAULT: Layout = Layout::new(m::WINDOW_WIDTH as u16, m::WINDOW_HEIGHT as u16);
    pub fn of(canvas: &arena_gfxkit::Canvas<'_>) -> Layout {
        let (w, h) = canvas.size();
        Layout::new(w as u16, h as u16)
    }
    pub const fn new(width: u16, height: u16) -> Layout {
        let w = width as i32;
        let h = height as i32;
        let status_y = h - 24;
        let list_w = {
            let v = w * 204 / 448;
            if v < 140 {
                140
            } else if v > 360 {
                360
            } else {
                v
            }
        };
        let file_list_h_rows = ((status_y - CONTENT_Y - m::M) / ROW_H) as usize;
        let preview_x = 8 + list_w + 12;
        let preview = Rect {
            x: preview_x,
            y: CONTENT_Y,
            width: (w - preview_x - 12) as u32,
            height: (status_y - CONTENT_Y - m::M) as u32,
        };
        let preview_text_y = preview.y + 28;
        let edit = Rect {
            x: m::CONTENT_INSET,
            y: CONTENT_Y,
            width: (w - 2 * m::CONTENT_INSET) as u32,
            height: (status_y - CONTENT_Y - m::M) as u32,
        };
        let input_y = status_y - 24;
        let monitor_row_y = CONTENT_Y + 28;
        Layout {
            W: w,
            H: h,
            TOOL_Y,
            HEADER_BOTTOM,
            CONTENT_Y,
            ROW_H,
            STATUS_Y: status_y,
            HEADER_INFO_X: w - 128,
            NAME_FIELD: tool(m::CONTENT_INSET, (w - 198 - 8 - m::CONTENT_INSET) as u32),
            PRIMARY: tool(w - 198, 82),
            SECONDARY: tool(w - 108, 96),
            NEW: tool(m::CONTENT_INSET, 60),
            SAVE: tool(80, 68),
            SAVE_AS: tool(156, 80),
            OPEN: tool(244, 68),
            DELETE: tool(320, 68),
            FILE_LIST: Rect {
                x: 8,
                y: CONTENT_Y,
                width: list_w as u32,
                height: (file_list_h_rows as i32 * ROW_H) as u32,
            },
            PREVIEW: preview,
            PREVIEW_TEXT_Y: preview_text_y,
            EDIT_TEXT: edit,
            APPEARANCE: preference_row(w, 82),
            MOTION: preference_row(w, 117),
            INPUT_Y: input_y,
            EDIT_COLUMNS: ((edit.width as i32 - m::SPACE[3]) / m::FONT_ADVANCE) as usize,
            EDIT_ROWS: ((edit.height as i32 - m::SPACE[3]) / m::LINE_HEIGHT) as usize,
            FILE_ROWS: file_list_h_rows,
            PREVIEW_ROWS: ((preview.y + preview.height as i32 - preview_text_y - m::S)
                / m::LINE_HEIGHT) as usize,
            PREVIEW_COLUMNS: ((preview.width as i32 - m::SPACE[4]) / m::FONT_ADVANCE) as usize,
            TERMINAL_ROWS: ((input_y - m::S - (CONTENT_Y + 6)) / m::LINE_HEIGHT) as usize,
            MONITOR_ROW_Y: monitor_row_y,
            MONITOR_ROWS: ((status_y - m::S - monitor_row_y) / m::LINE_HEIGHT) as usize,
        }
    }
    pub fn hit(&self, r: Rect, x: i32, y: i32) -> bool {
        hit(r, x, y)
    }
    pub fn visual_cursor(
        &self,
        e: &super::model::Editor,
        target_row: usize,
        target_column: usize,
    ) -> usize {
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
                if column >= self.EDIT_COLUMNS {
                    row += 1;
                    column = 0;
                }
            }
        }
        e.len
    }
    /// Visual rows of the whole document (wrapped at `EDIT_COLUMNS`).
    pub fn visual_row_count(&self, e: &super::model::Editor) -> usize {
        let mut row = 1;
        let mut col = 0;
        for b in &e.data[..e.len] {
            if *b == b'\n' {
                row += 1;
                col = 0;
            } else {
                col += if *b == b'\t' { 4 - col % 4 } else { 1 };
                if col >= self.EDIT_COLUMNS {
                    row += 1;
                    col = 0;
                }
            }
        }
        row
    }
    pub fn visual_row(&self, e: &super::model::Editor) -> usize {
        let mut row = 0;
        let mut col = 0;
        for b in &e.data[..e.cursor] {
            if *b == b'\n' {
                row += 1;
                col = 0;
            } else {
                col += if *b == b'\t' { 4 - col % 4 } else { 1 };
                if col >= self.EDIT_COLUMNS {
                    row += 1;
                    col = 0;
                }
            }
        }
        row
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    // Interaction points used by the guest suites (window-local).
    #[test]
    fn controls_keep_tested_interaction_points() {
        let l = Layout::DEFAULT;
        assert!(hit(l.NEW, 35, 50) && hit(l.SAVE, 115, 50) && hit(l.SAVE_AS, 195, 50));
        assert!(hit(l.OPEN, 275, 50) && hit(l.DELETE, 350, 50));
        assert!(hit(l.PRIMARY, 290, 50) && hit(l.SECONDARY, 380, 50));
        assert!(hit(l.FILE_LIST, 40, 82) && (82 - l.FILE_LIST.y) / ROW_H == 0);
        assert!(hit(l.APPEARANCE, 200, 100) && hit(l.MOTION, 200, 140));
        assert!(!hit(l.APPEARANCE, 200, 140) && !hit(l.MOTION, 200, 100));
        assert_eq!((l.EDIT_TEXT.x + 6, l.EDIT_TEXT.y + 6), (18, 74));
    }
    /// The Phase-10 constants, exactly, at the default size.
    #[test]
    fn default_size_is_the_phase10_geometry() {
        let l = Layout::DEFAULT;
        assert_eq!((l.STATUS_Y, l.HEADER_INFO_X, l.INPUT_Y), (264, 320, 240));
        assert_eq!(l.NAME_FIELD, tool(12, 230));
        assert_eq!((l.PRIMARY, l.SECONDARY), (tool(250, 82), tool(340, 96)));
        assert_eq!((l.FILE_LIST.x, l.FILE_LIST.width), (8, 204));
        assert_eq!((l.PREVIEW.x, l.PREVIEW.width), (224, 212));
        assert_eq!((l.EDIT_TEXT.width, l.APPEARANCE.width), (424, 424));
        assert_eq!((l.APPEARANCE.y, l.MOTION.y), (82, 117));
    }
    /// Every supported size keeps every control inside its band and every
    /// viewport inside the content area.
    #[test]
    fn every_size_lays_out_inside_the_surface() {
        for w in (MIN_WIDTH..=1024).step_by(37) {
            for h in (MIN_HEIGHT..=768).step_by(29) {
                let l = Layout::new(w, h);
                let (w, h) = (i32::from(w), i32::from(h));
                for r in [
                    l.NAME_FIELD,
                    l.PRIMARY,
                    l.SECONDARY,
                    l.NEW,
                    l.SAVE,
                    l.SAVE_AS,
                    l.OPEN,
                    l.DELETE,
                ] {
                    assert!(r.y >= m::TITLE_HEIGHT && r.y + r.height as i32 <= HEADER_BOTTOM);
                    assert!(r.x >= 0 && r.x + r.width as i32 <= w - m::CONTENT_INSET);
                }
                assert!(l.NAME_FIELD.x + l.NAME_FIELD.width as i32 <= l.PRIMARY.x);
                assert!(l.DELETE.x + l.DELETE.width as i32 <= w - m::CONTENT_INSET);
                assert!(l.FILE_LIST.y + l.FILE_LIST.height as i32 <= l.STATUS_Y);
                assert!(l.PREVIEW.x + l.PREVIEW.width as i32 <= w);
                assert!(l.PREVIEW.width >= 100 && l.EDIT_COLUMNS >= 20 && l.EDIT_ROWS >= 4);
                assert!(l.FILE_ROWS >= 4 && l.TERMINAL_ROWS >= 3 && l.MONITOR_ROWS >= 3);
                assert!(l.MONITOR_ROW_Y + l.MONITOR_ROWS as i32 * m::LINE_HEIGHT <= l.STATUS_Y);
                assert!(CONTENT_Y + 6 + l.TERMINAL_ROWS as i32 * m::LINE_HEIGHT <= l.INPUT_Y);
                assert!(l.STATUS_Y + 24 == h && l.MOTION.y + 35 <= l.STATUS_Y);
            }
        }
    }
}
