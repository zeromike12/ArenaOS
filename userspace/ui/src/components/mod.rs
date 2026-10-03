//! Reusable raster components; labels and state are supplied by views.
//!
//! One component library serves the compositor (chrome, shell) and every
//! application. Components only draw: no IPC, process, file or device work.
mod chrome;
mod controls;
mod gallery;
mod icons;
mod shapes;
mod text;

pub use chrome::{chrome, window_chrome};
pub use controls::{
    Kind, Tone, button, caret, chip, field, focus_ring, meter, progress, row, section, status_band,
    switch,
};
pub use gallery::gallery;
pub use icons::{Glyph, app_tile, draw_mask, emblem, glyph, lamp, pointer, well};
pub use shapes::{
    border, disc, dotted, fill, hline, inset, isqrt, outlined, rect, rounded, stadium,
    stadium_ring, vline,
};
pub use text::{
    Style, advance, caption, decimal, fit, heading, height as text_height, label, measure, strong,
    text, text_centered, text_fit, text_right,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Normal,
    Focused,
    Selected,
    Disabled,
    Refused,
}
