//! All common geometry is presentation policy, expressed in pixels.
//!
//! Window-policy and hit-test geometry (title bar, close strip, dock strip,
//! system bar, window size/cascade) is shared with the window manager and
//! its tests; changing those values changes interaction, not just looks.

// Spacing scale. Everything aligns to a 2px base grid; 4/8/12/16/24 are the
// working steps. `SPACE` is kept as the indexed form used by layouts.
pub const SPACE: [i32; 6] = [2, 4, 8, 12, 16, 24];
pub const XS: i32 = 2;
pub const S: i32 = 4;
pub const M: i32 = 8;
pub const L: i32 = 12;
pub const XL: i32 = 16;
pub const XXL: i32 = 24;

// Typography: the Arena-owned 5x7 bitmap face at integer scales only.
pub const FONT_SCALE: u8 = 1;
/// Display style (page titles, primary figures).
pub const TITLE_SCALE: u8 = 2;
pub const FONT_WIDTH: i32 = 5 * FONT_SCALE as i32;
pub const FONT_HEIGHT: i32 = 7 * FONT_SCALE as i32;
pub const FONT_ADVANCE: i32 = 6 * FONT_SCALE as i32;
pub const TITLE_FONT_HEIGHT: i32 = 7 * TITLE_SCALE as i32;
pub const TITLE_FONT_ADVANCE: i32 = 6 * TITLE_SCALE as i32;
/// Strong (double-struck) and tracked caption styles advance one extra
/// pixel so emboldened strokes never touch their neighbour.
pub const TRACKED_ADVANCE: i32 = FONT_ADVANCE + 1;
pub const LINE_HEIGHT: i32 = 14;

// Shell geometry (shared with window policy).
pub const TITLE_HEIGHT: i32 = 28;
pub const SYSTEM_BAR_HEIGHT: i32 = 26;
pub const DOCK_HEIGHT: i32 = 56;
pub const DOCK_ITEM_WIDTH: i32 = 58;
pub const CLOSE_WIDTH: i32 = 28;
pub const WINDOW_WIDTH: usize = 448;
pub const WINDOW_HEIGHT: usize = 288;
pub const WINDOW_START_X: i32 = 70;
pub const WINDOW_START_Y: i32 = 60;
pub const CASCADE_X: i32 = 26;
pub const CASCADE_Y: i32 = 24;
pub const VISIBLE_TITLE_WIDTH: i32 = 36;

// Shell presentation.
/// Visible floating dock panel inside the dock hit strip.
pub const DOCK_PANEL_HEIGHT: i32 = 48;
pub const DOCK_PANEL_BOTTOM: i32 = 5;
/// Opaque drop ledge under focused / unfocused windows.
pub const SHADOW_FOCUSED: i32 = 3;
pub const SHADOW_RESTING: i32 = 1;
/// Focused windows carry a Signal rail along their top edge.
pub const FOCUS_RAIL: i32 = 2;

// Window anatomy: title bar, header band (toolbar or page header), content,
// status band. Header and status share the `header` material.
pub const HEADER_BOTTOM: i32 = 66;
pub const FOOTER_Y: i32 = WINDOW_HEIGHT as i32 - 24;

// Controls.
pub const CONTROL_HEIGHT: i32 = 24;
pub const ROW_HEIGHT: i32 = 20;
pub const RADIUS: i32 = 2;
pub const RADIUS_PANEL: i32 = 3;
pub const ICON_SIZE: i32 = 16;
pub const TILE_SIZE: i32 = 26;
pub const GLYPH_SMALL: i32 = 9;
pub const SWITCH_WIDTH: i32 = 30;
pub const SWITCH_HEIGHT: i32 = 16;

pub const CONTENT_INSET: i32 = 12;
pub const SIDEBAR_WIDTH: i32 = 96;
