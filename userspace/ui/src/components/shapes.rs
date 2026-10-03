//! Opaque shape primitives built only from clipped rectangles.
//!
//! Rounded shapes leave their corner pixels unpainted, so whatever opaque
//! surface was drawn first shows through. That gives true rounded corners
//! without alpha, provided the caller paints the parent surface first.
use arena_gfxkit::{Canvas, Rect};

pub fn rect(c: &mut Canvas<'_>, x: i32, y: i32, w: i32, h: i32, color: u32) {
    if w > 0 && h > 0 {
        c.fill_rect(
            Rect {
                x,
                y,
                width: w as u32,
                height: h as u32,
            },
            color,
        );
    }
}

pub fn fill(c: &mut Canvas<'_>, r: Rect, color: u32) {
    rect(c, r.x, r.y, r.width as i32, r.height as i32, color);
}

pub fn hline(c: &mut Canvas<'_>, x: i32, y: i32, w: i32, color: u32) {
    rect(c, x, y, w, 1, color);
}

pub fn vline(c: &mut Canvas<'_>, x: i32, y: i32, h: i32, color: u32) {
    rect(c, x, y, 1, h, color);
}

pub fn border(c: &mut Canvas<'_>, r: Rect, color: u32) {
    let (w, h) = (r.width as i32, r.height as i32);
    hline(c, r.x, r.y, w, color);
    hline(c, r.x, r.y + h - 1, w, color);
    vline(c, r.x, r.y, h, color);
    vline(c, r.x + w - 1, r.y, h, color);
}

/// One-pixel dotted outline: the non-colour cue for disabled controls.
pub fn dotted(c: &mut Canvas<'_>, r: Rect, color: u32) {
    let (w, h) = (r.width as i32, r.height as i32);
    for i in (0..w).step_by(2) {
        rect(c, r.x + i, r.y, 1, 1, color);
        rect(c, r.x + i, r.y + h - 1, 1, 1, color);
    }
    for j in (0..h).step_by(2) {
        rect(c, r.x, r.y + j, 1, 1, color);
        rect(c, r.x + w - 1, r.y + j, 1, 1, color);
    }
}

pub fn inset(r: Rect, d: i32) -> Rect {
    Rect {
        x: r.x + d,
        y: r.y + d,
        width: (r.width as i32 - 2 * d).max(0) as u32,
        height: (r.height as i32 - 2 * d).max(0) as u32,
    }
}

/// Corner step tables: pixels skipped on each of the first rows.
const fn steps(radius: i32) -> &'static [i32] {
    match radius {
        0 => &[],
        1 => &[1],
        2 => &[2, 1],
        3 => &[3, 1, 1],
        _ => &[4, 2, 1, 1],
    }
}

fn row_inset(radius: i32, row: i32, height: i32) -> i32 {
    let s = steps(radius);
    let d = row.min(height - 1 - row);
    if d >= 0 && (d as usize) < s.len() {
        s[d as usize]
    } else {
        0
    }
}

/// Filled rounded rectangle; corner pixels are left untouched.
pub fn rounded(c: &mut Canvas<'_>, r: Rect, color: u32, radius: i32) {
    let (w, h) = (r.width as i32, r.height as i32);
    let radius = radius.min(h / 2).min(w / 2);
    let top = steps(radius).len() as i32;
    for row in 0..top.min(h) {
        for y in [row, h - 1 - row] {
            let i = row_inset(radius, y, h);
            rect(c, r.x + i, r.y + y, w - 2 * i, 1, color);
        }
    }
    rect(c, r.x, r.y + top, w, h - 2 * top, color);
}

/// Rounded rectangle with a one-pixel outline that follows the corner steps.
pub fn outlined(c: &mut Canvas<'_>, r: Rect, fill_color: u32, edge: u32, radius: i32) {
    let (w, h) = (r.width as i32, r.height as i32);
    if w <= 0 || h <= 0 {
        return;
    }
    let radius = radius.min(h / 2).min(w / 2);
    rounded(c, r, fill_color, radius);
    for y in 0..h {
        let i = row_inset(radius, y, h);
        if y == 0 || y == h - 1 {
            rect(c, r.x + i, r.y + y, w - 2 * i, 1, edge);
            continue;
        }
        // The outline meets the next row toward the nearer edge.
        let toward = if y <= h - 1 - y { y - 1 } else { y + 1 };
        let span = (row_inset(radius, toward, h) - i).max(1);
        rect(c, r.x + i, r.y + y, span, 1, edge);
        rect(c, r.x + w - i - span, r.y + y, span, 1, edge);
    }
}

/// Integer square root (floor).
pub const fn isqrt(v: i64) -> i64 {
    if v <= 0 {
        return 0;
    }
    let mut x = v;
    let mut y = (x + 1) / 2;
    while y < x {
        x = y;
        y = (x + v / x) / 2;
    }
    x
}

/// Filled stadium (rounded rectangle whose ends are full half-circles).
/// Used for the Arena emblem, switch tracks and pills.
pub fn stadium(c: &mut Canvas<'_>, r: Rect, color: u32) {
    let (w, h) = (r.width as i32, r.height as i32);
    if w <= 0 || h <= 0 {
        return;
    }
    let d = i64::from(h.min(w));
    for row in 0..h {
        let yc = i64::from(2 * row + 1 - h);
        let half = if yc.abs() >= d {
            0
        } else {
            isqrt(d * d - yc * yc)
        };
        let inset = ((d - half) / 2) as i32;
        rect(c, r.x + inset, r.y + row, w - 2 * inset, 1, color);
    }
}

/// Stadium outline of `thickness` pixels over an already painted `inner`.
pub fn stadium_ring(c: &mut Canvas<'_>, r: Rect, thickness: i32, color: u32, inner: u32) {
    stadium(c, r, color);
    stadium(c, inset(r, thickness), inner);
}

/// Small filled disc (diameter up to 9) used for lamps and indicators.
pub fn disc(c: &mut Canvas<'_>, x: i32, y: i32, size: i32, color: u32) {
    stadium(
        c,
        Rect {
            x,
            y,
            width: size as u32,
            height: size as u32,
        },
        color,
    );
}
