//! The file explorer's pixels and hit testing (Phase 11.8): toolbar with
//! back/forward/up and a breadcrumb, a sidebar of places, a sortable list
//! or an icon grid, inline rename, a drag badge and a properties sheet.
//! Geometry is a pure function of the window size and the sidebar width,
//! shared by drawing and hit testing.
use super::explorer::{Explorer, Sort, View, kind_label, kind_of};
use arena_gfxkit::{Canvas, Rect, measure13};
use arena_ui::components::{self as c, Glyph};
use arena_ui::widgets::{self as w, Column, ListHit, ScrollHit};
use arena_ui::{metrics as m, theme::Theme};

pub const PLACES: [(&str, &[u8], Glyph); 4] = [
    ("Home", b"", Glyph::Home),
    ("Desktop", b"Desktop", Glyph::Desktop),
    ("Documents", b"Documents", Glyph::Folder),
    ("Trash", b".Trash", Glyph::Trash),
];
pub const SIDEBAR_MIN: i32 = 96;
pub const SIDEBAR_MAX: i32 = 200;
const TOOL: i32 = 26;
const PLACE_H: i32 = 22;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Geometry {
    pub w: i32,
    pub h: i32,
    pub back: Rect,
    pub forward: Rect,
    pub up: Rect,
    pub crumbs: Rect,
    pub as_list: Rect,
    pub as_grid: Rect,
    pub sidebar: Rect,
    pub content: Rect,
    pub status_y: i32,
}

pub fn geometry(width: u16, height: u16, sidebar: i32) -> Geometry {
    let (w, h) = (i32::from(width), i32::from(height));
    let y = m::TITLE_HEIGHT + 6;
    let b = |x: i32| Rect {
        x,
        y,
        width: TOOL as u32,
        height: m::CONTROL_HEIGHT as u32,
    };
    let status_y = h - 24;
    let side = sidebar.clamp(SIDEBAR_MIN, SIDEBAR_MAX.min(w / 2));
    let top = m::HEADER_BOTTOM + 1;
    Geometry {
        w,
        h,
        back: b(m::CONTENT_INSET),
        forward: b(m::CONTENT_INSET + TOOL + 2),
        up: b(m::CONTENT_INSET + 2 * (TOOL + 2) + 4),
        crumbs: Rect {
            x: m::CONTENT_INSET + 3 * (TOOL + 2) + 12,
            y,
            width: (w
                - (m::CONTENT_INSET + 3 * (TOOL + 2) + 12)
                - 2 * (TOOL + 2)
                - m::CONTENT_INSET
                - 8)
            .max(40) as u32,
            height: m::CONTROL_HEIGHT as u32,
        },
        as_list: b(w - m::CONTENT_INSET - 2 * TOOL - 2),
        as_grid: b(w - m::CONTENT_INSET - TOOL),
        sidebar: Rect {
            x: 1,
            y: top,
            width: (side - 1) as u32,
            height: (status_y - top) as u32,
        },
        content: Rect {
            x: side + 1,
            y: top,
            width: (w - side - 2).max(0) as u32,
            height: (status_y - top) as u32,
        },
        status_y,
    }
}

/// Columns that fit `width` (the name column takes what is left).
pub fn columns(width: i32) -> ([Column; 4], usize) {
    let mut cols = [
        Column {
            title: "Name",
            width: 0,
            right: false,
        },
        Column {
            title: "Size",
            width: 64,
            right: true,
        },
        Column {
            title: "Modified",
            width: 112,
            right: true,
        },
        Column {
            title: "Kind",
            width: 96,
            right: false,
        },
    ];
    let n = if width >= 420 {
        4
    } else if width >= 300 {
        3
    } else {
        2
    };
    let fixed: i32 = cols[1..n].iter().map(|c| c.width).sum();
    cols[0].width = (width - w::BAR - fixed).max(60);
    (cols, n)
}

pub const SORTS: [Sort; 4] = [Sort::Name, Sort::Size, Sort::Modified, Sort::Kind];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit {
    Back,
    Forward,
    Up,
    Crumb(usize),
    AsList,
    AsGrid,
    Place(usize),
    Header(usize),
    Item(usize),
    Scrollbar(ScrollHit),
    Splitter,
    Blank,
}

fn inside(r: Rect, x: i32, y: i32) -> bool {
    x >= r.x && y >= r.y && x < r.x + r.width as i32 && y < r.y + r.height as i32
}

/// Breadcrumb segments of the current path: "Home" then each folder.
pub fn crumbs<'a>(e: &'a Explorer, out: &mut [&'a str; 16]) -> usize {
    out[0] = if e.in_trash() { "Trash" } else { "Home" };
    if e.in_trash() {
        return 1;
    }
    let mut n = 1;
    for part in e
        .path
        .bytes()
        .split(|b| *b == b'/')
        .filter(|p| !p.is_empty())
    {
        if n == out.len() {
            break;
        }
        out[n] = core::str::from_utf8(part).unwrap_or("?");
        n += 1;
    }
    n
}

pub fn hit(g: &Geometry, e: &Explorer, x: i32, y: i32) -> Hit {
    if inside(g.back, x, y) {
        return Hit::Back;
    }
    if inside(g.forward, x, y) {
        return Hit::Forward;
    }
    if inside(g.up, x, y) {
        return Hit::Up;
    }
    if inside(g.as_list, x, y) {
        return Hit::AsList;
    }
    if inside(g.as_grid, x, y) {
        return Hit::AsGrid;
    }
    if inside(g.crumbs, x, y) {
        let mut segs = [""; 16];
        let n = crumbs(e, &mut segs);
        return w::breadcrumb_hit(&segs[..n], g.crumbs.width as i32, x - g.crumbs.x)
            .map_or(Hit::Blank, Hit::Crumb);
    }
    if (x - g.content.x).abs() <= 2 && y >= g.content.y && y < g.status_y {
        return Hit::Splitter;
    }
    if inside(g.sidebar, x, y) {
        let i = ((y - g.sidebar.y - 8) / PLACE_H) as usize;
        return if y >= g.sidebar.y + 8 && i < PLACES.len() {
            Hit::Place(i)
        } else {
            Hit::Blank
        };
    }
    if inside(g.content, x, y) {
        return match e.view {
            View::List => {
                let (cols, n) = columns(g.content.width as i32);
                match e.list.hit(g.content, &cols[..n], x, y) {
                    ListHit::Header(i) => Hit::Header(i),
                    ListHit::Row(i) => Hit::Item(i),
                    ListHit::Scrollbar(h) => Hit::Scrollbar(h),
                    ListHit::Blank => Hit::Blank,
                }
            }
            View::Grid => {
                let track = grid_track(g);
                if x >= track.x
                    && let Some(h) = e.grid.scroll.hit(track, y)
                {
                    return Hit::Scrollbar(h);
                }
                e.grid.hit(g.content, x, y).map_or(Hit::Blank, Hit::Item)
            }
        };
    }
    Hit::Blank
}

fn grid_track(g: &Geometry) -> Rect {
    Rect {
        x: g.content.x + g.content.width as i32 - w::BAR,
        y: g.content.y,
        width: w::BAR as u32,
        height: g.content.height,
    }
}

/// Fit the list and grid scroll ranges to the window (call after a
/// resize or a re-list).
pub fn layout(e: &mut Explorer, g: &Geometry) {
    let n = e.count;
    e.list.layout(g.content, n);
    e.grid.layout(g.content, n);
}

/// "512 B", "4.0 KB", "1.2 MB" (decimal units).
pub fn size_text(v: u64, out: &mut [u8; 16]) -> &str {
    let (whole, frac, unit): (u64, Option<u64>, &[u8]) = if v < 1000 {
        (v, None, b" B")
    } else if v < 1_000_000 {
        (v / 1000, Some(v % 1000 / 100), b" KB")
    } else {
        (v / 1_000_000, Some(v % 1_000_000 / 100_000), b" MB")
    };
    let mut k = 0;
    let mut digits = [0u8; 20];
    let mut d = 0;
    let mut x = whole;
    loop {
        digits[d] = b'0' + (x % 10) as u8;
        d += 1;
        x /= 10;
        if x == 0 {
            break;
        }
    }
    while d > 0 {
        d -= 1;
        out[k] = digits[d];
        k += 1;
    }
    if let Some(f) = frac {
        out[k] = b'.';
        out[k + 1] = b'0' + f as u8;
        k += 2;
    }
    out[k..k + unit.len()].copy_from_slice(unit);
    k += unit.len();
    core::str::from_utf8(&out[..k]).unwrap_or("?")
}

/// Wall microseconds as "YYYY-MM-DD HH:MM" (UTC); 0 = "unknown".
pub fn time_text(us: u64, out: &mut [u8; 16]) -> &str {
    if us == 0 {
        return "unknown";
    }
    let secs = us / 1_000_000;
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    // Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    let mut put = |at: usize, v: u64, width: usize| {
        let mut x = v;
        for i in (0..width).rev() {
            out[at + i] = b'0' + (x % 10) as u8;
            x /= 10;
        }
    };
    put(0, year.clamp(0, 9999) as u64, 4);
    put(5, month as u64, 2);
    put(8, day as u64, 2);
    put(11, rem / 3600, 2);
    put(14, rem / 60 % 60, 2);
    out[4] = b'-';
    out[7] = b'-';
    out[10] = b' ';
    out[13] = b':';
    core::str::from_utf8(&out[..16]).unwrap_or("?")
}

/// Pointer and drag state the view shows (owned by the application).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Ui {
    pub hover: Option<Hit>,
    /// Dragging the selection: pointer position and the drop target.
    pub drag: Option<(i32, i32, Option<usize>)>,
    pub sidebar: i32,
    /// Properties sheet of a display item.
    pub sheet: Option<usize>,
    pub focused: bool,
}

#[allow(clippy::too_many_arguments)]
pub fn draw(
    canvas: &mut Canvas<'_>,
    e: &Explorer,
    ui: &Ui,
    sheet_info: Option<(u64, u64, u32)>,
    t: Theme,
) {
    let (width, height) = canvas.size();
    let g = geometry(width as u16, height as u16, ui.sidebar);
    // Toolbar.
    tool(
        canvas,
        g.back,
        Glyph::Chevron,
        e.can_back(),
        true,
        ui.hover == Some(Hit::Back),
        t,
    );
    tool(
        canvas,
        g.forward,
        Glyph::Chevron,
        e.can_forward(),
        false,
        ui.hover == Some(Hit::Forward),
        t,
    );
    tool(
        canvas,
        g.up,
        Glyph::Up,
        e.path.n > 0 && !e.in_trash(),
        false,
        ui.hover == Some(Hit::Up),
        t,
    );
    let mut segs = [""; 16];
    let n = crumbs(e, &mut segs);
    let hover = match ui.hover {
        Some(Hit::Crumb(i)) => Some(i),
        _ => None,
    };
    c::outlined(canvas, g.crumbs, t.field, t.field_edge, m::RADIUS);
    w::breadcrumb(
        canvas,
        Rect {
            x: g.crumbs.x + 4,
            y: g.crumbs.y,
            width: g.crumbs.width.saturating_sub(8),
            height: g.crumbs.height,
        },
        &segs[..n],
        hover,
        t,
    );
    toggle(canvas, g.as_list, e.view == View::List, false, t);
    toggle(canvas, g.as_grid, e.view == View::Grid, true, t);
    // Sidebar.
    c::rect(
        canvas,
        g.sidebar.x,
        g.sidebar.y,
        g.sidebar.width as i32,
        g.sidebar.height as i32,
        t.sidebar,
    );
    for (i, (label, path, glyph)) in PLACES.iter().enumerate() {
        let y = g.sidebar.y + 8 + i as i32 * PLACE_H;
        let here = e.path.bytes() == *path;
        let target = matches!(ui.drag, Some((_, _, _))) && ui.hover == Some(Hit::Place(i));
        if here || target {
            c::outlined(
                canvas,
                Rect {
                    x: g.sidebar.x + 4,
                    y,
                    width: g.sidebar.width.saturating_sub(8),
                    height: PLACE_H as u32,
                },
                if target { t.accent_soft } else { t.header },
                if target { t.accent } else { t.header },
                m::RADIUS,
            );
        } else if ui.hover == Some(Hit::Place(i)) {
            canvas.blend_rect(
                Rect {
                    x: g.sidebar.x + 4,
                    y,
                    width: g.sidebar.width.saturating_sub(8),
                    height: PLACE_H as u32,
                },
                t.accent,
                20,
            );
        }
        c::glyph(
            canvas,
            g.sidebar.x + 10,
            y + (PLACE_H - 9) / 2,
            *glyph,
            if here { t.accent } else { t.secondary },
        );
        w::text_in(
            canvas,
            g.sidebar.x + 26,
            y,
            PLACE_H,
            g.sidebar.width as i32 - 32,
            label,
            if here { t.text } else { t.secondary },
            here,
        );
    }
    w::Splitter {
        pos: g.content.x - 1,
        min: SIDEBAR_MIN,
        max: SIDEBAR_MAX,
    }
    .draw(
        canvas,
        g.content.y,
        g.content.height as i32,
        ui.hover == Some(Hit::Splitter),
        t,
    );
    // Content.
    match e.view {
        View::List => draw_list(canvas, e, ui, &g, t),
        View::Grid => draw_grid(canvas, e, ui, &g, t),
    }
    if e.count == 0 {
        let msg = if e.in_trash() {
            "The Trash is empty"
        } else {
            "This folder is empty"
        };
        let tw = measure13(msg, false);
        w::text_in(
            canvas,
            g.content.x + (g.content.width as i32 - tw) / 2,
            g.content.y + 40,
            20,
            tw + 2,
            msg,
            t.muted,
            false,
        );
    }
    // Drag badge.
    if let Some((x, y, _)) = ui.drag {
        let count = e.selection.count();
        let mut digits = [0u8; 32];
        let label = c::decimal(&mut digits, count as u64, false);
        let text_w = measure13(label, true) + measure13(" moving", false) + 6;
        let r = Rect {
            x: x + 12,
            y: y + 8,
            width: (text_w + 22) as u32,
            height: 20,
        };
        c::outlined(canvas, r, t.accent, t.accent, m::RADIUS);
        c::glyph(canvas, r.x + 5, r.y + 5, Glyph::Document, t.on_accent);
        let lw = w::text_in(canvas, r.x + 18, r.y, 20, 40, label, t.on_accent, true);
        w::text_in(
            canvas,
            r.x + 18 + lw + 3,
            r.y,
            20,
            60,
            "moving",
            t.on_accent,
            false,
        );
    }
    if let (Some(i), Some(info)) = (ui.sheet, sheet_info) {
        draw_sheet(canvas, e, i, info, &g, t);
    }
}

fn tool(c: &mut Canvas<'_>, r: Rect, g: Glyph, enabled: bool, mirror: bool, hover: bool, t: Theme) {
    let fill = if hover && enabled {
        t.control_edge
    } else {
        t.control
    };
    c::outlined(c, r, fill, t.control_edge, m::RADIUS);
    let color = if enabled { t.text } else { t.disabled };
    let (x, y) = (
        r.x + (r.width as i32 - 9) / 2,
        r.y + (r.height as i32 - 9) / 2,
    );
    if mirror {
        // Back: the chevron mirrored.
        let rows = c::glyph_mask(g);
        let mut flipped = [0u16; 9];
        for (i, row) in rows.iter().enumerate() {
            flipped[i] = (0..9).fold(0, |acc, b| acc | ((row >> b & 1) << (8 - b)));
        }
        c::draw_mask(c, x, y, &flipped, 9, color);
    } else {
        c::glyph(c, x, y, g, color);
    }
}

fn toggle(c: &mut Canvas<'_>, r: Rect, on: bool, grid: bool, t: Theme) {
    c::outlined(
        c,
        r,
        if on { t.accent_soft } else { t.control },
        if on { t.accent } else { t.control_edge },
        m::RADIUS,
    );
    let ink = if on { t.accent } else { t.secondary };
    let (x, y) = (r.x + 8, r.y + 7);
    if grid {
        for (dx, dy) in [(0, 0), (6, 0), (0, 6), (6, 6)] {
            c::rect(c, x + dx, y + dy, 4, 4, ink);
        }
    } else {
        for dy in [0, 4, 8] {
            c::rect(c, x, y + dy, 10, 2, ink);
        }
    }
}

fn draw_list(canvas: &mut Canvas<'_>, e: &Explorer, ui: &Ui, g: &Geometry, t: Theme) {
    let (cols, n) = columns(g.content.width as i32);
    let mut list = e.list;
    list.sort = Some((
        SORTS.iter().position(|s| *s == e.sort).unwrap_or(0),
        e.descending,
    ));
    list.hover = match ui.hover {
        Some(Hit::Item(i)) => Some(i),
        _ => None,
    };
    // Size and date text of the visible rows, formatted up front.
    let (first, end) = list.visible();
    let mut sizes = [([0u8; 16], 0usize); 64];
    let mut times = [([0u8; 16], 0usize); 64];
    for (k, row) in (first..end).enumerate().take(64) {
        let it = e.at(row);
        if !it.dir {
            sizes[k].1 = size_text(it.size, &mut sizes[k].0).len();
        }
        times[k].1 = time_text(it.mtime, &mut times[k].0).len();
    }
    let cell = |row: usize, col: usize| -> &str {
        let it = e.at(row);
        let k = row.wrapping_sub(first);
        match col {
            0 => core::str::from_utf8(it.name()).unwrap_or("?"),
            1 => slot(&sizes, k),
            2 => slot(&times, k),
            _ => kind_label(kind_of(it.name(), it.dir)),
        }
    };
    list.draw(
        canvas,
        g.content,
        &cols[..n],
        &e.selection,
        ui.focused,
        cell,
        |row| Some(w::kind_glyph(e.at(row).dir)),
        t,
    );
    // Drop target highlight and inline rename.
    if let Some((_, _, Some(target))) = ui.drag
        && target >= first
        && target < end
    {
        let y = g.content.y + w::HEADER + target as i32 * w::ROW - list.scroll.offset;
        c::border(
            canvas,
            Rect {
                x: g.content.x + 1,
                y,
                width: g.content.width.saturating_sub(w::BAR as u32 + 2),
                height: w::ROW as u32,
            },
            t.accent,
        );
    }
    if let Some((i, input)) = &e.rename
        && *i >= first
        && *i < end
    {
        let y = g.content.y + w::HEADER + *i as i32 * w::ROW - list.scroll.offset;
        let r = Rect {
            x: g.content.x + 22,
            y: y + 1,
            width: (cols[0].width - 26).max(40) as u32,
            height: (w::ROW - 2) as u32,
        };
        input.draw(canvas, r, true, t);
    }
}

fn slot(b: &[([u8; 16], usize); 64], k: usize) -> &str {
    b.get(k).map_or("", |(buf, n)| {
        core::str::from_utf8(&buf[..*n]).unwrap_or("")
    })
}

fn draw_grid(canvas: &mut Canvas<'_>, e: &Explorer, ui: &Ui, g: &Geometry, t: Theme) {
    c::rect(
        canvas,
        g.content.x,
        g.content.y,
        g.content.width as i32,
        g.content.height as i32,
        t.field,
    );
    let saved = canvas.clip();
    canvas.set_clip(Rect {
        x: g.content.x.max(saved.x),
        y: g.content.y.max(saved.y),
        width: g.content.width.min(saved.width),
        height: g.content.height.min(saved.height),
    });
    for i in 0..e.count {
        let r = e.grid.cell(g.content, i);
        if r.y + r.height as i32 <= g.content.y || r.y >= g.content.y + g.content.height as i32 {
            continue;
        }
        let it = e.at(i);
        let on = e.selection.contains(i);
        let target = matches!(ui.drag, Some((_, _, Some(x))) if x == i);
        if on || target {
            c::outlined(
                canvas,
                Rect {
                    x: r.x + 4,
                    y: r.y + 2,
                    width: r.width - 8,
                    height: r.height - 4,
                },
                t.accent_soft,
                if target { t.accent } else { t.accent_soft },
                m::RADIUS,
            );
        } else if ui.hover == Some(Hit::Item(i)) {
            canvas.blend_rect(
                Rect {
                    x: r.x + 4,
                    y: r.y + 2,
                    width: r.width - 8,
                    height: r.height - 4,
                },
                t.accent,
                20,
            );
        }
        big_glyph(
            canvas,
            r.x + (r.width as i32 - 27) / 2,
            r.y + 8,
            w::kind_glyph(it.dir),
            if it.dir { t.accent } else { t.secondary },
        );
        let name = core::str::from_utf8(it.name()).unwrap_or("?");
        let tw = measure13(name, false).min(r.width as i32 - 10);
        w::text_in(
            canvas,
            r.x + (r.width as i32 - tw) / 2,
            r.y + 42,
            20,
            r.width as i32 - 10,
            name,
            t.text,
            false,
        );
    }
    canvas.set_clip(saved);
    w::scrollbar(canvas, grid_track(g), &e.grid.scroll, ui.focused, t);
}

/// A 9x9 glyph at 3x (27 px): grid icons.
fn big_glyph(canvas: &mut Canvas<'_>, x: i32, y: i32, g: Glyph, color: u32) {
    for (r, row) in c::glyph_mask(g).iter().enumerate() {
        for b in 0..9 {
            if row >> (8 - b) & 1 == 1 {
                c::rect(canvas, x + b * 3, y + r as i32 * 3, 3, 3, color);
            }
        }
    }
}

fn draw_sheet(
    canvas: &mut Canvas<'_>,
    e: &Explorer,
    i: usize,
    (size, mtime, entries): (u64, u64, u32),
    g: &Geometry,
    t: Theme,
) {
    let it = e.at(i);
    let r = Rect {
        x: g.w / 2 - 130,
        y: g.h / 2 - 70,
        width: 260,
        height: 140,
    };
    canvas.darken_rect(
        Rect {
            x: 1,
            y: m::TITLE_HEIGHT,
            width: (g.w - 2) as u32,
            height: (g.h - m::TITLE_HEIGHT - 1) as u32,
        },
        60,
    );
    c::outlined(canvas, r, t.elevated, t.frame_focus, m::RADIUS_PANEL);
    big_glyph(canvas, r.x + 14, r.y + 14, w::kind_glyph(it.dir), t.accent);
    let name = core::str::from_utf8(it.name()).unwrap_or("?");
    w::text_in(canvas, r.x + 52, r.y + 12, 20, 196, name, t.text, true);
    let kind = kind_label(kind_of(it.name(), it.dir));
    w::text_in(
        canvas,
        r.x + 52,
        r.y + 30,
        18,
        196,
        kind,
        t.secondary,
        false,
    );
    let mut sb = [0u8; 16];
    let mut tb = [0u8; 16];
    let mut digits = [0u8; 32];
    let rows: [(&str, &str); 2] = [
        (
            if it.dir { "Items" } else { "Size" },
            if it.dir {
                c::decimal(&mut digits, u64::from(entries), true)
            } else {
                size_text(size, &mut sb)
            },
        ),
        ("Modified", time_text(mtime, &mut tb)),
    ];
    for (k, (label, value)) in rows.iter().enumerate() {
        let y = r.y + 60 + k as i32 * 20;
        w::text_in(canvas, r.x + 16, y, 20, 80, label, t.muted, false);
        w::text_in(canvas, r.x + 100, y, 20, 150, value, t.text, false);
    }
    w::text_in(
        canvas,
        r.x + 16,
        r.y + 112,
        20,
        230,
        "Press Esc to close",
        t.muted,
        false,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sizes_times_and_columns() {
        let mut b = [0u8; 16];
        assert_eq!(size_text(0, &mut b), "0 B");
        assert_eq!(size_text(999, &mut b), "999 B");
        assert_eq!(size_text(4096, &mut b), "4.0 KB");
        assert_eq!(size_text(16_777_216, &mut b), "16.7 MB");
        assert_eq!(time_text(0, &mut b), "unknown");
        assert_eq!(
            time_text(1_791_158_400_000_000 + 3_723_000_000, &mut b),
            "2026-10-05 01:02"
        );
        assert_eq!(time_text(951_782_400_000_000, &mut b), "2000-02-29 00:00");
        assert_eq!(columns(500).1, 4);
        assert_eq!(columns(320).1, 3);
        assert_eq!(columns(200).1, 2);
        let (cols, n) = columns(500);
        assert_eq!(cols[..n].iter().map(|c| c.width).sum::<i32>(), 500 - w::BAR);
    }
    #[test]
    fn geometry_hits_every_part() {
        let e = Explorer::new();
        let g = geometry(448, 288, 118);
        assert_eq!(hit(&g, &e, g.back.x + 2, g.back.y + 2), Hit::Back);
        assert_eq!(hit(&g, &e, g.up.x + 2, g.up.y + 2), Hit::Up);
        assert_eq!(hit(&g, &e, g.as_grid.x + 2, g.as_grid.y + 2), Hit::AsGrid);
        assert_eq!(
            hit(&g, &e, 20, g.sidebar.y + 8 + 3 * PLACE_H + 2),
            Hit::Place(3)
        );
        assert_eq!(hit(&g, &e, g.content.x, g.content.y + 30), Hit::Splitter);
        assert_eq!(
            hit(&g, &e, g.content.x + 20, g.content.y + 4),
            Hit::Header(0)
        );
        assert_eq!(hit(&g, &e, g.crumbs.x + 6, g.crumbs.y + 5), Hit::Crumb(0));
        assert!(g.crumbs.x + g.crumbs.width as i32 <= g.as_list.x);
        let wide = geometry(800, 518, 500);
        assert_eq!(wide.content.x, SIDEBAR_MAX + 1);
    }
}
