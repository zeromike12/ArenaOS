//! ArenaOS palettes. Every colour the shell and applications draw comes from
//! one of these semantic tokens; views never invent literal colours. These
//! tokens never enter service wires and confer no authority.
//!
//! Light ("Paper") and dark ("Ink") are designed separately around the same
//! roles: a calm field, quiet chrome, one teal Signal accent, and distinct
//! warm/red tones that are always paired with a non-colour cue (glyph,
//! weight, border or position) by the components that use them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Theme {
    // Desktop shell.
    /// Desktop field behind every window.
    pub desktop: u32,
    /// Arena emblem rings drawn on the empty desktop.
    pub desktop_mark: u32,
    /// Empty-desktop hint text.
    pub desktop_text: u32,
    /// System bar surface, its lower hairline and its text roles.
    pub bar: u32,
    pub bar_edge: u32,
    pub bar_text: u32,
    pub bar_muted: u32,
    /// Floating dock panel, outline and hovered/active tile wells.
    pub dock: u32,
    pub dock_edge: u32,
    pub dock_well: u32,
    /// Pointer outline and body; the outline differs from every desktop
    /// colour so the arrow stays legible over any surface.
    pub pointer_ink: u32,
    pub pointer_fill: u32,
    // Window surfaces.
    /// Window content surface. The compositor also fills unpublished
    /// windows with it while they reveal, so the name is part of its API.
    pub elevated: u32,
    /// Title bar, toolbar band and status band ("header" material).
    pub header: u32,
    /// Hairline separators inside a window.
    pub divider: u32,
    /// Window outline: unfocused, focused.
    pub frame: u32,
    pub frame_focus: u32,
    /// Opaque drop ledge under windows and floating shell panels.
    pub shadow: u32,
    // Text roles.
    pub text: u32,
    pub secondary: u32,
    pub muted: u32,
    pub disabled: u32,
    // Signal accent and state tones (each has a strong and a soft form).
    pub accent: u32,
    pub accent_strong: u32,
    pub accent_soft: u32,
    pub on_accent: u32,
    pub error: u32,
    pub error_soft: u32,
    pub warning: u32,
    pub warning_soft: u32,
    pub success: u32,
    pub success_soft: u32,
    // Controls.
    pub control: u32,
    pub control_edge: u32,
    pub field: u32,
    pub field_edge: u32,
    /// Unfilled part of meters, switches and progress.
    pub track: u32,
    /// Sidebar / inset group surface.
    pub sidebar: u32,
    // Console (Terminal) palette.
    pub terminal: u32,
    pub terminal_text: u32,
    pub terminal_muted: u32,
    pub terminal_prompt: u32,
    pub terminal_edge: u32,
    // Editor palette.
    pub editor_line: u32,
    pub editor_caret: u32,
    // Application identity tiles: Terminal, Files, Editor, Settings,
    // Monitor, Gallery. Glyphs on tiles use `on_tile`.
    pub tiles: [u32; 6],
    pub on_tile: u32,
}

/// "Paper": cool white material on a steel field.
pub const LIGHT: Theme = Theme {
    desktop: 0x4d6474,
    desktop_mark: 0x587083,
    desktop_text: 0xc3d0da,
    bar: 0xf5f7f9,
    bar_edge: 0xc6ced6,
    bar_text: 0x141a21,
    bar_muted: 0x5b6773,
    dock: 0xf5f7f9,
    dock_edge: 0xb9c3cc,
    dock_well: 0xe1e7ec,
    pointer_ink: 0x0a0d11,
    pointer_fill: 0xffffff,
    elevated: 0xfdfdfe,
    header: 0xf0f3f5,
    divider: 0xdbe1e6,
    frame: 0x9aa6b1,
    frame_focus: 0x0e7c77,
    shadow: 0x34495a,
    text: 0x141a21,
    secondary: 0x4a5663,
    muted: 0x76828e,
    disabled: 0xa9b2bb,
    accent: 0x0e7c77,
    accent_strong: 0x085e5a,
    accent_soft: 0xd6eeec,
    on_accent: 0xffffff,
    error: 0xc0392b,
    error_soft: 0xfae3df,
    warning: 0x9a6200,
    warning_soft: 0xfbeed3,
    success: 0x1d7f4a,
    success_soft: 0xdcf0e3,
    control: 0xf6f8fa,
    control_edge: 0xbcc5ce,
    field: 0xffffff,
    field_edge: 0xaeb8c2,
    track: 0xdde3e8,
    sidebar: 0xf0f3f5,
    terminal: 0x18202a,
    terminal_text: 0xd9e3ea,
    terminal_muted: 0x7f8d9a,
    terminal_prompt: 0x3fc9bf,
    terminal_edge: 0x2a3440,
    editor_line: 0xeef6f5,
    editor_caret: 0x0e7c77,
    tiles: [0x26303b, 0xc0841f, 0x3a72c4, 0x667483, 0x24936b, 0x7d5bc9],
    on_tile: 0xffffff,
};

/// "Ink": deep blue-black field with low-glare graphite material.
pub const DARK: Theme = Theme {
    desktop: 0x0b1015,
    desktop_mark: 0x172029,
    desktop_text: 0x66778a,
    bar: 0x131a21,
    bar_edge: 0x232d38,
    bar_text: 0xe4eaef,
    bar_muted: 0x8c99a6,
    dock: 0x151c24,
    dock_edge: 0x2b3642,
    dock_well: 0x222c37,
    pointer_ink: 0x000000,
    pointer_fill: 0xf4f7f9,
    elevated: 0x161d25,
    header: 0x1c242e,
    divider: 0x29333e,
    frame: 0x323d49,
    frame_focus: 0x2fb3a9,
    shadow: 0x030507,
    text: 0xe4eaef,
    secondary: 0xa3afbb,
    muted: 0x7a8794,
    disabled: 0x535e6a,
    accent: 0x2fb3a9,
    accent_strong: 0x6dd6ce,
    accent_soft: 0x153a39,
    on_accent: 0x04201e,
    error: 0xff7a68,
    error_soft: 0x3b1d1a,
    warning: 0xefb54e,
    warning_soft: 0x392b12,
    success: 0x5ccb8a,
    success_soft: 0x15321f,
    control: 0x222b35,
    control_edge: 0x37434f,
    field: 0x10161c,
    field_edge: 0x3a4653,
    track: 0x2a3440,
    sidebar: 0x1a222b,
    terminal: 0x0a0e13,
    terminal_text: 0xcfdcd9,
    terminal_muted: 0x657380,
    terminal_prompt: 0x2fb3a9,
    terminal_edge: 0x232d38,
    editor_line: 0x1b2730,
    editor_caret: 0x6dd6ce,
    tiles: [0x323d4a, 0xb67d1f, 0x4079cc, 0x5f6d7c, 0x2a9a72, 0x8463cf],
    on_tile: 0xffffff,
};

pub const fn palette(dark: bool) -> Theme {
    if dark { DARK } else { LIGHT }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    /// WCAG-style relative luminance contrast, x1000, integer only.
    fn contrast(a: u32, b: u32) -> u64 {
        fn lin(c: u32) -> f64 {
            let c = f64::from(c & 255) / 255.0;
            if c <= 0.03928 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        }
        fn lum(p: u32) -> f64 {
            0.2126 * lin(p >> 16) + 0.7152 * lin(p >> 8) + 0.0722 * lin(p)
        }
        let (x, y) = (lum(a), lum(b));
        let (hi, lo) = if x > y { (x, y) } else { (y, x) };
        ((hi + 0.05) / (lo + 0.05) * 1000.0) as u64
    }
    #[test]
    fn legible_roles_in_both_palettes() {
        for t in [LIGHT, DARK] {
            // Body and secondary text on every window surface.
            for surface in [t.elevated, t.header, t.field, t.control, t.sidebar] {
                assert!(contrast(t.text, surface) >= 7000);
                assert!(contrast(t.secondary, surface) >= 4500);
                assert!(contrast(t.muted, surface) >= 3000);
            }
            assert!(contrast(t.bar_text, t.bar) >= 7000);
            assert!(contrast(t.bar_muted, t.bar) >= 4500);
            assert!(contrast(t.on_accent, t.accent) >= 4500);
            assert!(contrast(t.error, t.elevated) >= 4500);
            assert!(contrast(t.error, t.error_soft) >= 4000);
            assert!(contrast(t.warning, t.warning_soft) >= 4000);
            assert!(contrast(t.success, t.success_soft) >= 4000);
            assert!(contrast(t.accent, t.accent_soft) >= 3000);
            assert!(contrast(t.text, t.accent_soft) >= 7000);
            assert!(contrast(t.terminal_text, t.terminal) >= 7000);
            assert!(contrast(t.terminal_prompt, t.terminal) >= 4500);
            assert!(contrast(t.terminal_muted, t.terminal) >= 3000);
            assert!(contrast(t.desktop_text, t.desktop) >= 3000);
            // Disabled must read as present but clearly weaker than muted.
            assert!(contrast(t.disabled, t.elevated) >= 1800);
            assert!(contrast(t.disabled, t.elevated) < contrast(t.muted, t.elevated));
            for tile in t.tiles {
                assert!(contrast(t.on_tile, tile) >= 3000);
            }
        }
        assert_ne!(LIGHT, DARK);
    }
}
