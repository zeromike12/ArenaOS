//! Original ArenaOS icon language, authored here as one-bit masks.
//!
//! Two families share one construction rule — even pixel grid, solid forms
//! with knocked-out detail, no outlines thinner than the surrounding mass:
//! * 16x16 application glyphs, always placed on a rounded identity tile;
//! * 9x9 interface glyphs for toolbars, status and window controls.
//!
//! Masks are compiled into the binary (about 0.4 KiB in total); there is no
//! asset loader and no imported artwork.
use super::shapes::{disc, outlined, rect, rounded, stadium, stadium_ring};
use crate::{metrics as m, motion, theme::Theme};
use arena_gfxkit::{Canvas, Rect};

const fn mask<const N: usize>(rows: [&str; N], on: u8) -> [u16; N] {
    let mut out = [0u16; N];
    let mut r = 0;
    while r < N {
        let b = rows[r].as_bytes();
        let mut i = 0;
        let mut v = 0u16;
        while i < b.len() {
            v <<= 1;
            if b[i] == on {
                v |= 1;
            }
            i += 1;
        }
        out[r] = v;
        r += 1;
    }
    out
}

/// Draws set bits of `rows` (each `width` bits wide, MSB left) as runs.
pub fn draw_mask(c: &mut Canvas<'_>, x: i32, y: i32, rows: &[u16], width: i32, color: u32) {
    for (j, bits) in rows.iter().enumerate() {
        let mut i = 0;
        while i < width {
            if bits & (1 << (width - 1 - i)) == 0 {
                i += 1;
                continue;
            }
            let start = i;
            while i < width && bits & (1 << (width - 1 - i)) != 0 {
                i += 1;
            }
            rect(c, x + start, y + j as i32, i - start, 1, color);
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Glyph {
    Close,
    Plus,
    Save,
    Rename,
    Open,
    Refresh,
    Delete,
    Error,
    Warning,
    Success,
    Idle,
    Document,
    Chevron,
}

const CLOSE: [u16; 9] = mask(
    [
        ".........",
        "##.....##",
        ".##...##.",
        "..##.##..",
        "...###...",
        "..##.##..",
        ".##...##.",
        "##.....##",
        ".........",
    ],
    b'#',
);
const PLUS: [u16; 9] = mask(
    [
        "....#....",
        "....#....",
        "....#....",
        "....#....",
        "#########",
        "....#....",
        "....#....",
        "....#....",
        "....#....",
    ],
    b'#',
);
const SAVE: [u16; 9] = mask(
    [
        "....#....",
        "....#....",
        "....#....",
        ".#..#..#.",
        "..#.#.#..",
        "...###...",
        "#...#...#",
        "#.......#",
        "#########",
    ],
    b'#',
);
const RENAME: [u16; 9] = mask(
    [
        "......##.",
        ".....#..#",
        "....#..#.",
        "...#..#..",
        "..#..#...",
        ".#..#....",
        "#..#.....",
        "#.#......",
        "##.......",
    ],
    b'#',
);
const OPEN: [u16; 9] = mask(
    [
        "...######",
        "........#",
        "#.....#.#",
        "#....#..#",
        "#...#...#",
        "#..#.....",
        "#........",
        "#........",
        "######...",
    ],
    b'#',
);
const REFRESH: [u16; 9] = mask(
    [
        "...####.#",
        ".##....##",
        "#.....###",
        "#........",
        "#.......#",
        "........#",
        "###.....#",
        "##....##.",
        "#.####...",
    ],
    b'#',
);
const DELETE: [u16; 9] = mask(
    [
        "...###...",
        "#########",
        ".........",
        ".#######.",
        ".#.#.#.#.",
        ".#.#.#.#.",
        ".#.#.#.#.",
        ".#.#.#.#.",
        "..#####..",
    ],
    b'#',
);
const ERROR: [u16; 9] = mask(
    [
        "..#####..",
        ".#######.",
        "####.####",
        "####.####",
        "####.####",
        "#########",
        "####.####",
        ".#######.",
        "..#####..",
    ],
    b'#',
);
const WARNING: [u16; 9] = mask(
    [
        "....#....",
        "...###...",
        "..##.##..",
        ".###.###.",
        "####.####",
        ".#######.",
        "..##.##..",
        "...###...",
        "....#....",
    ],
    b'#',
);
const SUCCESS: [u16; 9] = mask(
    [
        "..#####..",
        ".#######.",
        "#######.#",
        "######.##",
        "#.###.###",
        "##.#.####",
        "###.#####",
        ".#######.",
        "..#####..",
    ],
    b'#',
);
const IDLE: [u16; 9] = mask(
    [
        "..#####..",
        ".##...##.",
        "##.....##",
        "#.......#",
        "#.......#",
        "#.......#",
        "##.....##",
        ".##...##.",
        "..#####..",
    ],
    b'#',
);
const DOCUMENT: [u16; 9] = mask(
    [
        ".#####...",
        ".#...##..",
        ".#...#.#.",
        ".#...###.",
        ".#.....#.",
        ".#.###.#.",
        ".#.....#.",
        ".#.###.#.",
        ".#######.",
    ],
    b'#',
);
const CHEVRON: [u16; 9] = mask(
    [
        ".........",
        "..##.....",
        "...##....",
        "....##...",
        ".....##..",
        "....##...",
        "...##....",
        "..##.....",
        ".........",
    ],
    b'#',
);

pub const fn glyph_mask(g: Glyph) -> &'static [u16; 9] {
    match g {
        Glyph::Close => &CLOSE,
        Glyph::Plus => &PLUS,
        Glyph::Save => &SAVE,
        Glyph::Rename => &RENAME,
        Glyph::Open => &OPEN,
        Glyph::Refresh => &REFRESH,
        Glyph::Delete => &DELETE,
        Glyph::Error => &ERROR,
        Glyph::Warning => &WARNING,
        Glyph::Success => &SUCCESS,
        Glyph::Idle => &IDLE,
        Glyph::Document => &DOCUMENT,
        Glyph::Chevron => &CHEVRON,
    }
}

pub fn glyph(c: &mut Canvas<'_>, x: i32, y: i32, g: Glyph, color: u32) {
    draw_mask(c, x, y, glyph_mask(g), m::GLYPH_SMALL, color);
}

// Application glyphs: '#' is drawn in the tile's glyph colour and '+' in
// its secondary tone (Signal teal for the Terminal prompt cursor, otherwise
// a half-tone of the glyph colour).
const TERMINAL: [&str; 16] = [
    "................",
    "................",
    "................",
    "..##............",
    "...##...........",
    "....##..........",
    ".....##.........",
    "......##........",
    ".....##.........",
    "....##..........",
    "...##...........",
    "..##............",
    "................",
    "........++++++..",
    "........++++++..",
    "................",
];
const FILES: [&str; 16] = [
    "................",
    "......########..",
    "......#......#..",
    "......#......#..",
    "..#########..#..",
    "..#########..#..",
    "..#########..#..",
    "..##+++++##..#..",
    "..#########..#..",
    "..#########.##..",
    "..##+++++##.....",
    "..#########.....",
    "..#########.....",
    "..##+++++##.....",
    "..#########.....",
    "................",
];
const EDITOR: [&str; 16] = [
    "................",
    ".#########......",
    ".#.......#......",
    ".#.......#..###.",
    ".#.#####.#...#..",
    ".#.......#...#..",
    ".#.+++++.#...#..",
    ".#.......#...#..",
    ".#.#####.#...#..",
    ".#.......#...#..",
    ".#.+++...#...#..",
    ".#.......#..###.",
    ".#.......#......",
    ".#########......",
    "................",
    "................",
];
const SETTINGS: [&str; 16] = [
    "................",
    "...##########...",
    "..############..",
    "..########...#..",
    "..########...#..",
    "..############..",
    "...##########...",
    "................",
    "................",
    "...##########...",
    "..#++++......#..",
    "..#++++......#..",
    "..#++++......#..",
    "..#++++......#..",
    "...##########...",
    "................",
];
const MONITOR: [&str; 16] = [
    "................",
    "................",
    "................",
    ".##########++++.",
    ".##########++++.",
    "................",
    "................",
    ".#####+++++++++.",
    ".#####+++++++++.",
    "................",
    "................",
    ".############++.",
    ".############++.",
    "................",
    "................",
    "................",
];
const GALLERY: [&str; 16] = [
    "................",
    ".######...####..",
    ".######..#....#.",
    ".######..#....#.",
    ".######..#....#.",
    ".######..#....#.",
    ".######...####..",
    "................",
    "................",
    ".######...####..",
    ".#++++#..######.",
    ".#++++#..######.",
    ".#++++#..######.",
    ".#++++#..######.",
    ".######...####..",
    "................",
];

const APP_PRIMARY: [[u16; 16]; 6] = [
    mask(TERMINAL, b'#'),
    mask(FILES, b'#'),
    mask(EDITOR, b'#'),
    mask(SETTINGS, b'#'),
    mask(MONITOR, b'#'),
    mask(GALLERY, b'#'),
];
const APP_SECONDARY: [[u16; 16]; 6] = [
    mask(TERMINAL, b'+'),
    mask(FILES, b'+'),
    mask(EDITOR, b'+'),
    mask(SETTINGS, b'+'),
    mask(MONITOR, b'+'),
    mask(GALLERY, b'+'),
];

/// Application tile: identity colour, rounded corners, 16px glyph.
/// `dim` blends the tile toward `under` for unavailable launch targets.
#[allow(clippy::too_many_arguments)]
pub fn app_tile(
    c: &mut Canvas<'_>,
    x: i32,
    y: i32,
    size: i32,
    kind: u8,
    dim: bool,
    under: u32,
    t: Theme,
) {
    let k = (kind as usize).min(5);
    let mut tile = t.tiles[k];
    let mut ink = t.on_tile;
    if dim {
        tile = motion::color(tile, under, 26_000);
        ink = motion::color(ink, under, 26_000);
    }
    let second = if k == 0 {
        if dim {
            motion::color(t.terminal_prompt, under, 26_000)
        } else {
            t.terminal_prompt
        }
    } else {
        motion::color(ink, tile, 30_000)
    };
    rounded(
        c,
        Rect {
            x,
            y,
            width: size as u32,
            height: size as u32,
        },
        tile,
        if size >= 20 {
            m::RADIUS_PANEL
        } else {
            m::RADIUS
        },
    );
    if size >= m::ICON_SIZE + 4 {
        let o = (size - m::ICON_SIZE) / 2;
        draw_mask(c, x + o, y + o, &APP_PRIMARY[k], m::ICON_SIZE, ink);
        draw_mask(c, x + o, y + o, &APP_SECONDARY[k], m::ICON_SIZE, second);
    }
}

/// Arena emblem: a stadium track around a field with a centre line and
/// centre circle, seen from above. Scales from the 16px bar mark upward.
pub fn emblem(c: &mut Canvas<'_>, r: Rect, ring: u32, field: u32) {
    let (w, h) = (r.width as i32, r.height as i32);
    if h < 20 {
        // Compact mark: one ring and a centre dot.
        stadium_ring(c, r, 2, ring, field);
        disc(c, r.x + w / 2 - 1, r.y + h / 2 - 1, 2, ring);
        return;
    }
    let t = (h / 40).max(2);
    stadium_ring(c, r, t, ring, field);
    let inner = super::shapes::inset(r, t + h / 9);
    stadium_ring(c, inner, 1, ring, field);
    let cx = r.x + w / 2;
    rect(c, cx, inner.y, 1, inner.height as i32, ring);
    let d = h / 3;
    stadium_ring(
        c,
        Rect {
            x: cx - d / 2,
            y: r.y + (h - d) / 2,
            width: d as u32,
            height: d as u32,
        },
        1,
        ring,
        field,
    );
    rect(c, cx, r.y + (h - d) / 2, 1, d, ring);
    disc(c, cx - 2, r.y + h / 2 - 2, 5, ring);
}

/// The ArenaOS pointer: a notched arrowhead, ink outline, light fill.
const POINTER_INK: [u16; 13] = mask(
    [
        "#.........",
        "##........",
        "#.#.......",
        "#..#......",
        "#...#.....",
        "#....#....",
        "#.....#...",
        "#......#..",
        "#...#####.",
        "#..#......",
        "#.#.......",
        "##........",
        "#.........",
    ],
    b'#',
);
const POINTER_FILL: [u16; 13] = mask(
    [
        "..........",
        "..........",
        ".#........",
        ".##.......",
        ".###......",
        ".####.....",
        ".#####....",
        ".######...",
        ".###......",
        ".##.......",
        ".#........",
        "..........",
        "..........",
    ],
    b'#',
);

pub fn pointer(c: &mut Canvas<'_>, x: i32, y: i32, t: Theme) {
    draw_mask(c, x, y, &POINTER_INK, 10, t.pointer_ink);
    draw_mask(c, x, y, &POINTER_FILL, 10, t.pointer_fill);
}

/// Window focus lamp: filled Signal disc when focused, hollow ring at rest,
/// cross-faded by the shared focus motion amount (0..=65536).
pub fn lamp(c: &mut Canvas<'_>, x: i32, y: i32, amount: i32, under: u32, t: Theme) {
    let edge = motion::color(t.muted, t.accent, amount);
    let core = motion::color(under, t.accent, amount);
    let r = Rect {
        x,
        y,
        width: 8,
        height: 8,
    };
    stadium(c, r, edge);
    stadium(c, super::shapes::inset(r, 1), core);
}

/// Toolbar-sized square well used behind dock tiles and window controls.
pub fn well(c: &mut Canvas<'_>, r: Rect, fill_color: u32, edge: Option<u32>) {
    match edge {
        Some(e) => outlined(c, r, fill_color, e, m::RADIUS_PANEL),
        None => rounded(c, r, fill_color, m::RADIUS_PANEL),
    }
}
