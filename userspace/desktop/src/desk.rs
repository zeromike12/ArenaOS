//! The desktop surface (Phase 11.8): `/Users/user/Desktop` as icons on the
//! shell background, at grid cells the user arranges, persisted in the
//! folder itself (`.positions`). Pure logic over a `Store` plus a
//! comparable view the compositor draws; the broker owns the instance and
//! resolves paths through its own capability for /Users/user.
use crate::apps::explorer::{Explorer, Kind, Path, Status, Store, TRASH, kind_of, sniff_text};
use crate::apps::explorer_view::icon32;
use arena_gfxkit::{Canvas, Rect, measure13};
use arena_ui::{components as c, metrics as m, theme::Theme, widgets as w};

/// Icons the desktop shows (a fuller folder says so).
pub const DESK_MAX: usize = 24;
pub const CELL_W: i32 = 84;
pub const CELL_H: i32 = 74;
const LEFT: i32 = 10;
const TOP: i32 = m::SYSTEM_BAR_HEIGHT + 10;
const NAME: usize = 28;
pub const POSITIONS: &[u8] = b".positions";
const FOLDER: &[u8] = b"Desktop";
const DOUBLE_US: u64 = 450_000;
const SLOP: i32 = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DeskIcon {
    pub name: [u8; NAME],
    pub len: u8,
    /// The name was longer than shown.
    pub cut: bool,
    pub dir: bool,
    pub col: u8,
    pub row: u8,
    pub selected: bool,
    /// The folder a drag would drop into.
    pub target: bool,
}
impl DeskIcon {
    const EMPTY: DeskIcon = DeskIcon {
        name: [0; NAME],
        len: 0,
        cut: false,
        dir: false,
        col: 0,
        row: 0,
        selected: false,
        target: false,
    };
    pub fn label(&self) -> &str {
        core::str::from_utf8(&self.name[..usize::from(self.len)]).unwrap_or("?")
    }
}

/// The desktop context menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MenuView {
    pub x: i32,
    pub y: i32,
    pub items: [&'static str; 4],
    pub n: u8,
    pub hover: Option<u8>,
}
pub const MENU_W: i32 = 168;
pub const MENU_ITEM: i32 = 24;
impl MenuView {
    pub fn rect(&self) -> Rect {
        Rect {
            x: self.x,
            y: self.y,
            width: MENU_W as u32,
            height: (i32::from(self.n) * MENU_ITEM + 8) as u32,
        }
    }
    pub fn item(&self, x: i32, y: i32) -> Option<u8> {
        let r = self.rect();
        let i = (y - r.y - 4).div_euclid(MENU_ITEM);
        (x >= r.x && x < r.x + MENU_W && y >= r.y + 4 && i >= 0 && i < i32::from(self.n))
            .then_some(i as u8)
    }
}

/// Everything the compositor draws for the desktop surface (comparable:
/// the compositor repaints exactly the cells that changed).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DeskView {
    pub icons: [DeskIcon; DESK_MAX],
    pub count: u8,
    /// Selected icons follow the pointer by this offset while dragged.
    pub ghost: Option<(i32, i32)>,
    pub menu: Option<MenuView>,
}
impl DeskView {
    pub const EMPTY: DeskView = DeskView {
        icons: [DeskIcon::EMPTY; DESK_MAX],
        count: 0,
        ghost: None,
        menu: None,
    };
}

pub fn cell_rect(col: u8, row: u8) -> Rect {
    Rect {
        x: LEFT + i32::from(col) * CELL_W,
        y: TOP + i32::from(row) * CELL_H,
        width: CELL_W as u32,
        height: CELL_H as u32,
    }
}
/// Grid size for a screen: rows above the dock, columns across.
pub fn grid(w: i32, h: i32) -> (u8, u8) {
    let rows = ((h - m::DOCK_HEIGHT - TOP - 4) / CELL_H).clamp(1, 255);
    let cols = ((w - LEFT) / CELL_W).clamp(1, 255);
    (cols as u8, rows as u8)
}
fn cell_at(w: i32, h: i32, x: i32, y: i32) -> (u8, u8) {
    let (cols, rows) = grid(w, h);
    let col = ((x - LEFT).div_euclid(CELL_W)).clamp(0, i32::from(cols) - 1);
    let row = ((y - TOP).div_euclid(CELL_H)).clamp(0, i32::from(rows) - 1);
    (col as u8, row as u8)
}
/// The icon (display index) under the point: its glyph or its label.
pub fn hit(v: &DeskView, x: i32, y: i32) -> Option<usize> {
    (0..usize::from(v.count)).find(|i| {
        let r = cell_rect(v.icons[*i].col, v.icons[*i].row);
        x >= r.x + 6
            && x < r.x + r.width as i32 - 6
            && y >= r.y + 2
            && y < r.y + r.height as i32 - 2
    })
}

/// The area a change to `a` -> `b` can touch (for damage).
pub fn changed(a: &DeskView, b: &DeskView, mut add: impl FnMut(Rect)) {
    let n = a.count.max(b.count) as usize;
    for i in 0..n {
        let (x, y) = (a.icons[i], b.icons[i]);
        let differ = i >= usize::from(a.count) || i >= usize::from(b.count) || x != y;
        let ghosted = |v: &DeskView, ic: &DeskIcon| v.ghost.is_some() && ic.selected;
        if differ || a.ghost != b.ghost && (ghosted(a, &x) || ghosted(b, &y)) {
            for (v, ic) in [(a, &x), (b, &y)] {
                let r = cell_rect(ic.col, ic.row);
                add(r);
                if let Some((dx, dy)) = v.ghost.filter(|_| ic.selected) {
                    add(Rect {
                        x: r.x + dx,
                        y: r.y + dy,
                        ..r
                    });
                }
            }
        }
    }
    if a.menu != b.menu {
        for menu in [a.menu, b.menu].into_iter().flatten() {
            let r = menu.rect();
            add(Rect {
                x: r.x - 2,
                y: r.y - 2,
                width: r.width + 8,
                height: r.height + 8,
            });
        }
    }
}

/// Icons on the background (windows cover them).
pub fn draw(canvas: &mut Canvas<'_>, v: &DeskView, t: Theme) {
    for ic in &v.icons[..usize::from(v.count)] {
        let r = cell_rect(ic.col, ic.row);
        icon(canvas, r, ic, ic.selected && v.ghost.is_none(), t);
        if let Some((dx, dy)) = v.ghost.filter(|_| ic.selected) {
            let g = Rect {
                x: r.x + dx,
                y: r.y + dy,
                ..r
            };
            icon(canvas, g, ic, true, t);
        }
    }
}
fn icon(canvas: &mut Canvas<'_>, r: Rect, ic: &DeskIcon, on: bool, t: Theme) {
    if on || ic.target {
        let well = Rect {
            x: r.x + 18,
            y: r.y + 2,
            width: 48,
            height: 40,
        };
        canvas.blend_rect(well, t.on_accent, 48);
        if ic.target {
            c::border(canvas, well, t.on_accent);
        }
    }
    icon32(canvas, r.x + (CELL_W - 32) / 2, r.y + 6, ic.dir, t);
    let mut label = [0u8; NAME + 3];
    let n = usize::from(ic.len);
    label[..n].copy_from_slice(&ic.name[..n]);
    let mut k = n;
    if ic.cut {
        label[k..k + 3].copy_from_slice("\u{2026}".as_bytes());
        k += 3;
    }
    let text = core::str::from_utf8(&label[..k]).unwrap_or("?");
    let tw = measure13(text, false).min(CELL_W - 6);
    let lx = r.x + (CELL_W - tw) / 2;
    if on {
        c::outlined(
            canvas,
            Rect {
                x: lx - 4,
                y: r.y + 45,
                width: (tw + 8) as u32,
                height: 18,
            },
            t.accent,
            t.accent,
            m::RADIUS,
        );
    }
    w::text_in(
        canvas,
        lx,
        r.y + 45,
        18,
        CELL_W - 6,
        text,
        if on { t.on_accent } else { t.desktop_text },
        false,
    );
}
/// The desktop context menu (above windows).
pub fn draw_menu(canvas: &mut Canvas<'_>, menu: &MenuView, t: Theme) {
    let r = menu.rect();
    c::popup_frame(canvas, r.x, r.y, r.width as i32, r.height as i32, t);
    c::rect(
        canvas,
        r.x,
        r.y,
        r.width as i32,
        r.height as i32,
        t.elevated,
    );
    for i in 0..menu.n {
        let y = r.y + 4 + i32::from(i) * MENU_ITEM;
        let on = menu.hover == Some(i);
        if on {
            c::rect(canvas, r.x + 3, y, MENU_W - 6, MENU_ITEM, t.accent);
        }
        w::text_in(
            canvas,
            r.x + 12,
            y,
            MENU_ITEM,
            MENU_W - 20,
            menu.items[usize::from(i)],
            if on { t.on_accent } else { t.text },
            false,
        );
    }
}

// ---- positions -----------------------------------------------------------------

/// Parse `.positions`: lines `col row name`. Unknown or malformed lines are
/// ignored (the file is the user's own data, not authority).
pub fn parse<'a>(data: &'a [u8], mut each: impl FnMut(u8, u8, &'a [u8])) {
    for line in data.split(|b| *b == b'\n') {
        let mut parts = line.splitn(3, |b| *b == b' ');
        let (Some(a), Some(b), Some(name)) = (parts.next(), parts.next(), parts.next()) else {
            continue;
        };
        let num = |s: &[u8]| -> Option<u8> {
            (!s.is_empty() && s.len() <= 3 && s.iter().all(u8::is_ascii_digit))
                .then(|| s.iter().fold(0u32, |v, d| v * 10 + u32::from(d - b'0')))
                .and_then(|v| u8::try_from(v).ok())
        };
        if let (Some(col), Some(row)) = (num(a), num(b))
            && !name.is_empty()
        {
            each(col, row, name);
        }
    }
}

/// What a desktop interaction asks of the broker.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Effect {
    None,
    /// Open this folder (relative to home) in Files.
    OpenFolder(Path),
    /// Open this text file in an Editor (the broker grants it).
    OpenDocument(Path),
    /// Nothing opens this kind.
    NoApplication,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Press {
    Icon {
        x: i32,
        y: i32,
        i: usize,
        dragging: bool,
        collapse: bool,
    },
    Blank,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Cmd {
    Open,
    Trash,
    NewFolder,
    NewDocument,
    Arrange,
}
const ICON_MENU: [(&str, Cmd); 2] = [("Open", Cmd::Open), ("Move to Trash", Cmd::Trash)];
const DESK_MENU: [(&str, Cmd); 3] = [
    ("New Folder", Cmd::NewFolder),
    ("New Document", Cmd::NewDocument),
    ("Arrange Icons", Cmd::Arrange),
];

pub struct Desk {
    pub ex: Explorer,
    /// Cell of each display item (index into the explorer's order).
    cells: [(u8, u8); DESK_MAX],
    pub screen: (i32, i32),
    press: Option<Press>,
    buttons: u8,
    last: Option<(u64, usize)>,
    pub view: DeskView,
    menu: &'static [(&'static str, Cmd)],
    /// The folder held more items than the desktop shows.
    pub truncated: bool,
}

impl Desk {
    pub const fn new() -> Self {
        Desk {
            ex: Explorer::new(),
            cells: [(0, 0); DESK_MAX],
            screen: (800, 600),
            press: None,
            buttons: 0,
            last: None,
            view: DeskView::EMPTY,
            menu: &DESK_MENU,
            truncated: false,
        }
    }

    /// List the Desktop folder and place its icons (saved cells first,
    /// then the first free cells, column by column).
    pub fn load(&mut self, s: &mut impl Store) -> Result<(), Status> {
        if self.ex.path.bytes() != FOLDER {
            self.ex.path = Path::of(FOLDER);
        }
        self.ex.refresh(s)?;
        let n = self.ex.count.min(DESK_MAX);
        self.truncated = self.ex.count > DESK_MAX;
        let mut buf = [0u8; 4096];
        let len = s
            .read(Path::of(FOLDER).child(POSITIONS)?.bytes(), 0, &mut buf)
            .unwrap_or(0);
        let (cols, rows) = grid(self.screen.0, self.screen.1);
        let mut taken = [[false; 64]; 32];
        let mut placed = [false; DESK_MAX];
        parse(&buf[..len], |col, row, name| {
            if col >= cols
                || row >= rows
                || col >= 32
                || row >= 64
                || taken[col as usize][row as usize]
            {
                return;
            }
            if let Some(i) = (0..n).find(|i| !placed[*i] && self.ex.at(*i).name() == name) {
                self.cells[i] = (col, row);
                placed[i] = true;
                taken[col as usize][row as usize] = true;
            }
        });
        for i in 0..n {
            if !placed[i] {
                self.cells[i] = free(&mut taken, cols, rows);
            }
        }
        self.view_update();
        Ok(())
    }

    /// Re-list when the Desktop folder changed (another app, the terminal).
    pub fn poll(&mut self, s: &mut impl Store) -> bool {
        if self.press.is_some() {
            return false;
        }
        match s.stat(FOLDER) {
            Ok(_) if self.ex.changed(s) => self.load(s).is_ok(),
            _ => false,
        }
    }

    fn save(&mut self, s: &mut impl Store) -> Result<(), Status> {
        let mut out = [0u8; 4096];
        let mut k = 0;
        for i in 0..self.ex.count.min(DESK_MAX) {
            let name = self.ex.at(i).name();
            let (col, row) = self.cells[i];
            let line_len = 8 + name.len() + 1;
            if k + line_len > out.len() {
                break;
            }
            for v in [col, row] {
                let d = [b'0' + v / 100, b'0' + v / 10 % 10, b'0' + v % 10];
                let skip = if v >= 100 {
                    0
                } else if v >= 10 {
                    1
                } else {
                    2
                };
                out[k..k + 3 - skip].copy_from_slice(&d[skip..]);
                k += 3 - skip;
                out[k] = b' ';
                k += 1;
            }
            out[k..k + name.len()].copy_from_slice(name);
            k += name.len();
            out[k] = b'\n';
            k += 1;
        }
        let path = Path::of(FOLDER).child(POSITIONS)?;
        if s.stat(path.bytes()).is_err() {
            s.create(path.bytes())?;
        }
        s.write(path.bytes(), 0, &out[..k])?;
        Ok(())
    }

    fn view_update(&mut self) {
        let n = self.ex.count.min(DESK_MAX);
        let ghost = self.view.ghost;
        let menu = self.view.menu;
        let mut v = DeskView::EMPTY;
        for i in 0..n {
            let it = self.ex.at(i);
            let name = it.name();
            let shown = name.len().min(NAME);
            let mut icon = DeskIcon::EMPTY;
            // Cut at a character boundary.
            let mut end = shown;
            while end > 0 && core::str::from_utf8(&name[..end]).is_err() {
                end -= 1;
            }
            icon.name[..end].copy_from_slice(&name[..end]);
            icon.len = end as u8;
            icon.cut = end < name.len();
            icon.dir = it.dir;
            (icon.col, icon.row) = self.cells[i];
            icon.selected = self.ex.selection.contains(i);
            v.icons[i] = icon;
        }
        v.count = n as u8;
        v.ghost = ghost;
        v.menu = menu;
        if let (Some((dx, dy)), Some(Press::Icon { .. })) = (ghost, self.press) {
            // Mark the folder under the dragged icons' pointer as target.
            let _ = (dx, dy);
        }
        self.view = v;
    }

    /// Whether the desktop wants this pointer event: a press on the bare
    /// desktop (or on its menu), or anything while it holds a press.
    pub fn owns(&self, bare: bool) -> bool {
        bare || self.press.is_some() || self.view.menu.is_some()
    }

    /// Pointer at screen (`x`, `y`) with `ctrl` held. Only called while
    /// `owns` (the window policy sees no buttons meanwhile).
    pub fn pointer(
        &mut self,
        s: &mut impl Store,
        x: i32,
        y: i32,
        buttons: u8,
        ctrl: bool,
        now: u64,
    ) -> Effect {
        let (left, was) = (buttons & 1 != 0, self.buttons & 1 != 0);
        let context = buttons & 2 != 0 && self.buttons & 2 == 0;
        self.buttons = buttons;
        if let Some(menu) = self.view.menu {
            let item = menu.item(x, y);
            if (left && !was) || context {
                self.view.menu = None;
                let e = match item {
                    Some(i) if left => self.command(s, self.menu[usize::from(i)].1),
                    _ => Effect::None,
                };
                self.view_update();
                return e;
            }
            if item != menu.hover {
                self.view.menu = Some(MenuView {
                    hover: item,
                    ..menu
                });
            }
            return Effect::None;
        }
        let at = hit(&self.view, x, y);
        if context {
            match at {
                Some(i) => {
                    if !self.ex.selection.contains(i) {
                        self.ex.selection.click(i, false, false);
                    }
                    self.menu = &ICON_MENU;
                }
                None => {
                    self.ex.selection.clear();
                    self.menu = &DESK_MENU;
                }
            }
            let mut items = [""; 4];
            for (o, (l, _)) in items.iter_mut().zip(self.menu) {
                *o = l;
            }
            let h = self.menu.len() as i32 * MENU_ITEM + 8;
            self.view.menu = Some(MenuView {
                x: x.min(self.screen.0 - MENU_W - 4),
                y: y.min(self.screen.1 - h - 4),
                items,
                n: self.menu.len() as u8,
                hover: None,
            });
            self.view_update();
            return Effect::None;
        }
        if left && !was {
            match at {
                Some(i) => {
                    let double = !ctrl
                        && self
                            .last
                            .is_some_and(|(t, j)| j == i && now.saturating_sub(t) <= DOUBLE_US);
                    if double {
                        self.last = None;
                        self.press = None;
                        let e = self.open(s, i);
                        self.view_update();
                        return e;
                    }
                    let was_selected = self.ex.selection.contains(i);
                    if ctrl || !was_selected {
                        self.ex.selection.click(i, false, ctrl);
                    }
                    self.last = Some((now, i));
                    self.press = Some(Press::Icon {
                        x,
                        y,
                        i,
                        dragging: false,
                        collapse: was_selected && !ctrl,
                    });
                }
                None => {
                    if !ctrl {
                        self.ex.selection.clear();
                    }
                    self.press = Some(Press::Blank);
                }
            }
            self.view_update();
            return Effect::None;
        }
        if left {
            if let Some(Press::Icon {
                x: x0,
                y: y0,
                i,
                dragging,
                collapse,
            }) = self.press
                && (dragging || (x - x0).abs() > SLOP || (y - y0).abs() > SLOP)
            {
                self.press = Some(Press::Icon {
                    x: x0,
                    y: y0,
                    i,
                    dragging: true,
                    collapse,
                });
                self.view.ghost = Some((x - x0, y - y0));
                let target = at.filter(|j| self.ex.at(*j).dir && !self.ex.selection.contains(*j));
                self.view_update();
                for k in 0..usize::from(self.view.count) {
                    self.view.icons[k].target = Some(k) == target;
                }
            }
            return Effect::None;
        }
        if was {
            let press = self.press.take();
            let ghost = self.view.ghost.take();
            match (press, ghost) {
                (Some(Press::Icon { dragging: true, .. }), Some((dx, dy))) => {
                    let into = at.filter(|j| self.ex.at(*j).dir && !self.ex.selection.contains(*j));
                    match into {
                        Some(j) => {
                            let name = *self.ex.at(j);
                            let _ = Path::of(FOLDER)
                                .child(name.name())
                                .and_then(|to| self.ex.move_selection_to(s, to));
                            let _ = self.load(s);
                        }
                        None => {
                            self.move_cells(dx, dy);
                            let _ = self.save(s);
                        }
                    }
                }
                (
                    Some(Press::Icon {
                        collapse: true, i, ..
                    }),
                    _,
                ) => self.ex.selection.click(i, false, false),
                _ => {}
            }
            self.view_update();
        }
        Effect::None
    }

    /// Move the selected icons by a pixel offset, snapped to free cells.
    fn move_cells(&mut self, dx: i32, dy: i32) {
        let n = self.ex.count.min(DESK_MAX);
        let (cols, rows) = grid(self.screen.0, self.screen.1);
        let mut taken = [[false; 64]; 32];
        for i in (0..n).filter(|i| !self.ex.selection.contains(*i)) {
            let (c, r) = self.cells[i];
            taken[usize::from(c).min(31)][usize::from(r).min(63)] = true;
        }
        for i in (0..n).filter(|i| self.ex.selection.contains(*i)) {
            let r = cell_rect(self.cells[i].0, self.cells[i].1);
            let (c, row) = cell_at(
                self.screen.0,
                self.screen.1,
                r.x + CELL_W / 2 + dx,
                r.y + CELL_H / 2 + dy,
            );
            self.cells[i] = if taken[usize::from(c)][usize::from(row)] {
                nearest_free(&taken, cols, rows, c, row)
            } else {
                (c, row)
            };
            taken[usize::from(self.cells[i].0)][usize::from(self.cells[i].1)] = true;
        }
    }

    /// A key while no window has focus.
    pub fn key(&mut self, s: &mut impl Store, code: u16) -> Effect {
        let e = match code {
            13 => match self.ex.selection.first() {
                Some(i) => self.open(s, i),
                None => Effect::None,
            },
            262 => self.command(s, Cmd::Trash),
            27 => {
                self.view.menu = None;
                self.ex.selection.clear();
                Effect::None
            }
            _ => Effect::None,
        };
        self.view_update();
        e
    }

    fn command(&mut self, s: &mut impl Store, cmd: Cmd) -> Effect {
        match cmd {
            Cmd::Open => {
                if let Some(i) = self.ex.selection.first() {
                    return self.open(s, i);
                }
            }
            Cmd::Trash => {
                if self.ex.selection.count() > 0 {
                    let _ = self.ex.delete(s);
                    let _ = self.load(s);
                    let _ = self.save(s);
                }
            }
            Cmd::NewFolder | Cmd::NewDocument => {
                let r = if cmd == Cmd::NewFolder {
                    self.ex.new_folder(s)
                } else {
                    self.ex.new_document(s)
                };
                // The new item is named in Files (or by its default name).
                self.ex.cancel_rename();
                if r.is_ok() {
                    let _ = self.load(s);
                    let _ = self.save(s);
                }
            }
            Cmd::Arrange => {
                let n = self.ex.count.min(DESK_MAX);
                let (cols, rows) = grid(self.screen.0, self.screen.1);
                let mut taken = [[false; 64]; 32];
                for i in 0..n {
                    self.cells[i] = free(&mut taken, cols, rows);
                }
                let _ = self.save(s);
            }
        }
        Effect::None
    }

    fn open(&mut self, s: &mut impl Store, i: usize) -> Effect {
        let it = *self.ex.at(i);
        let Ok(path) = Path::of(FOLDER).child(it.name()) else {
            return Effect::None;
        };
        if it.dir {
            return Effect::OpenFolder(path);
        }
        if kind_of(it.name(), false) != Kind::Text || it.size > 4096 {
            return Effect::NoApplication;
        }
        let mut head = [0u8; 512];
        match s.read(path.bytes(), 0, &mut head) {
            Ok(n) if sniff_text(&head[..n]) => Effect::OpenDocument(path),
            _ => Effect::NoApplication,
        }
    }

    /// The Trash path, for callers that drop onto it (unused by the
    /// desktop itself: icons go to the Trash by menu or Delete).
    pub fn trash() -> Path {
        Path::of(TRASH)
    }
}

impl Default for Desk {
    fn default() -> Self {
        Self::new()
    }
}

fn free(taken: &mut [[bool; 64]; 32], cols: u8, rows: u8) -> (u8, u8) {
    for c in 0..cols.min(32) {
        for r in 0..rows.min(64) {
            if !taken[usize::from(c)][usize::from(r)] {
                taken[usize::from(c)][usize::from(r)] = true;
                return (c, r);
            }
        }
    }
    (0, 0)
}
fn nearest_free(taken: &[[bool; 64]; 32], cols: u8, rows: u8, c: u8, r: u8) -> (u8, u8) {
    let mut best = (c, r);
    let mut dist = i32::MAX;
    for cc in 0..cols.min(32) {
        for rr in 0..rows.min(64) {
            if !taken[usize::from(cc)][usize::from(rr)] {
                let d =
                    (i32::from(cc) - i32::from(c)).pow(2) + (i32::from(rr) - i32::from(r)).pow(2);
                if d < dist {
                    dist = d;
                    best = (cc, rr);
                }
            }
        }
    }
    best
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use crate::apps::explorer::tests::Mem;
    use std::borrow::ToOwned;
    use std::boxed::Box;

    fn setup() -> (Box<Desk>, Mem) {
        let mut m = Mem::new();
        for d in ["Desktop", "Desktop/Photos", "Documents", ".Trash"] {
            m.mkdir(d.as_bytes()).unwrap();
        }
        for (f, data) in [
            ("Desktop/a.txt", &b"alpha"[..]),
            ("Desktop/b.bin", b"\x80\x81"),
            ("Desktop/readme", b"hi"),
        ] {
            m.create(f.as_bytes()).unwrap();
            m.write(f.as_bytes(), 0, data).unwrap();
        }
        let mut d = Box::new(Desk::new());
        d.load(&mut m).unwrap();
        (d, m)
    }
    fn center(d: &Desk, name: &str) -> (i32, i32, usize) {
        let i = (0..usize::from(d.view.count))
            .find(|i| d.view.icons[*i].label() == name)
            .unwrap();
        let r = cell_rect(d.view.icons[i].col, d.view.icons[i].row);
        (r.x + CELL_W / 2, r.y + 20, i)
    }

    #[test]
    fn positions_parse_and_persist_and_survive_reload() {
        let mut got = std::vec::Vec::new();
        parse(
            b"0 0 a.txt\n2 3 my file.txt\nx 1 bad\n1 1 \n300 1 big\n4 5 b",
            |c, r, n| got.push((c, r, n.to_vec())),
        );
        assert_eq!(
            got,
            [
                (0, 0, b"a.txt".to_vec()),
                (2, 3, b"my file.txt".to_vec()),
                (4, 5, b"b".to_vec())
            ]
        );
        let (mut d, mut m) = setup();
        // Folders first, then names: Photos, a.txt, b.bin, readme in column 0.
        assert_eq!(d.view.icons[0].label(), "Photos");
        assert_eq!((d.view.icons[3].col, d.view.icons[3].row), (0, 3));
        // Drag readme two cells right and one down.
        let (x, y, _) = center(&d, "readme");
        d.pointer(&mut m, x, y, 1, false, 0);
        d.pointer(&mut m, x + 20, y, 1, false, 0);
        assert!(d.view.ghost.is_some());
        d.pointer(&mut m, x + 2 * CELL_W, y + CELL_H, 1, false, 0);
        d.pointer(&mut m, x + 2 * CELL_W, y + CELL_H, 0, false, 0);
        assert_eq!(d.view.ghost, None);
        let (_, _, i) = center(&d, "readme");
        assert_eq!((d.view.icons[i].col, d.view.icons[i].row), (2, 4));
        let saved = m.get("Desktop/.positions").cloned().unwrap().unwrap();
        assert!(
            saved.windows(10).any(|w| w == b"2 4 readme"),
            "{}",
            std::string::String::from_utf8_lossy(&saved)
        );
        // A fresh desk reads the same cells back.
        let mut again = Box::new(Desk::new());
        again.load(&mut m).unwrap();
        let cells = |v: &DeskView| {
            v.icons[..4]
                .iter()
                .map(|i| (i.label().to_owned(), i.col, i.row))
                .collect::<std::vec::Vec<_>>()
        };
        assert_eq!(cells(&again.view), cells(&d.view));
        // Dropping onto an occupied cell takes the nearest free one.
        let (x, y, _) = center(&d, "a.txt");
        let (tx, ty, _) = center(&d, "b.bin");
        d.pointer(&mut m, x, y, 1, false, 1_000_000);
        d.pointer(&mut m, x + 30, y + 30, 1, false, 1_000_000);
        d.pointer(&mut m, tx + 30, ty + 40, 1, false, 1_000_000);
        d.pointer(&mut m, tx + 30, ty + 40, 0, false, 1_000_000);
        let cells: std::collections::BTreeSet<_> =
            d.view.icons[..4].iter().map(|i| (i.col, i.row)).collect();
        assert_eq!(cells.len(), 4, "two icons share a cell");
    }

    #[test]
    fn open_drop_into_folder_menu_and_trash() {
        let (mut d, mut m) = setup();
        let (x, y, _) = center(&d, "a.txt");
        d.pointer(&mut m, x, y, 1, false, 10);
        d.pointer(&mut m, x, y, 0, false, 10);
        assert_eq!(
            d.pointer(&mut m, x, y, 1, false, 100_000),
            Effect::OpenDocument(Path::of(b"Desktop/a.txt"))
        );
        d.pointer(&mut m, x, y, 0, false, 100_000);
        let (bx, by, _) = center(&d, "b.bin");
        d.pointer(&mut m, bx, by, 1, false, 5_000_000);
        d.pointer(&mut m, bx, by, 0, false, 5_000_000);
        assert_eq!(
            d.pointer(&mut m, bx, by, 1, false, 5_100_000),
            Effect::NoApplication
        );
        d.pointer(&mut m, bx, by, 0, false, 5_100_000);
        let (px, py, _) = center(&d, "Photos");
        d.pointer(&mut m, px, py, 1, false, 9_000_000);
        d.pointer(&mut m, px, py, 0, false, 9_000_000);
        assert_eq!(
            d.pointer(&mut m, px, py, 1, false, 9_100_000),
            Effect::OpenFolder(Path::of(b"Desktop/Photos"))
        );
        d.pointer(&mut m, px, py, 0, false, 9_100_000);
        // Drag a.txt onto Photos: it moves into the folder.
        d.pointer(&mut m, x, y, 1, false, 20_000_000);
        d.pointer(&mut m, x + 10, y + 10, 1, false, 20_000_000);
        d.pointer(&mut m, px, py, 1, false, 20_000_000);
        let (_, _, pi) = center(&d, "Photos");
        assert!(d.view.icons[pi].target);
        d.pointer(&mut m, px, py, 0, false, 20_000_000);
        assert!(m.get("Desktop/Photos/a.txt").is_some());
        assert!(m.get("Desktop/a.txt").is_none());
        assert_eq!(d.view.count, 3);
        // Context menu on the bare desktop: New Folder.
        d.pointer(&mut m, 700, 400, 2, false, 30_000_000);
        let menu = d.view.menu.unwrap();
        assert_eq!(
            &menu.items[..3],
            ["New Folder", "New Document", "Arrange Icons"]
        );
        d.pointer(&mut m, 700, 400, 0, false, 30_000_000);
        let r = menu.rect();
        d.pointer(&mut m, r.x + 20, r.y + 4 + 5, 1, false, 30_000_000);
        d.pointer(&mut m, r.x + 20, r.y + 4 + 5, 0, false, 30_000_000);
        assert!(m.get("Desktop/New Folder").is_some());
        assert_eq!(d.view.menu, None);
        // Icon menu: Move to Trash, with a restore record.
        let (rx, ry, _) = center(&d, "readme");
        d.pointer(&mut m, rx, ry, 2, false, 40_000_000);
        d.pointer(&mut m, rx, ry, 0, false, 40_000_000);
        let r = d.view.menu.unwrap().rect();
        d.pointer(
            &mut m,
            r.x + 20,
            r.y + 4 + MENU_ITEM + 5,
            1,
            false,
            40_000_000,
        );
        d.pointer(
            &mut m,
            r.x + 20,
            r.y + 4 + MENU_ITEM + 5,
            0,
            false,
            40_000_000,
        );
        assert!(m.get(".Trash/readme").is_some());
        assert_eq!(
            m.get(".Trash/.restore/readme"),
            Some(&Some(b"Desktop/readme".to_vec()))
        );
        // The desktop re-lists a change made elsewhere.
        m.create(b"Desktop/new.txt").unwrap();
        assert!(d.poll(&mut m));
        assert!((0..usize::from(d.view.count)).any(|i| d.view.icons[i].label() == "new.txt"));
    }

    #[test]
    fn damage_covers_changed_cells_ghosts_and_menu() {
        let (mut d, mut m) = setup();
        let a = d.view;
        let (x, y, i) = center(&d, "readme");
        d.pointer(&mut m, x, y, 1, false, 0);
        let b = d.view;
        let mut rects = std::vec::Vec::new();
        changed(&a, &b, |r| rects.push(r));
        let cell = cell_rect(a.icons[i].col, a.icons[i].row);
        assert!(rects.contains(&cell));
        d.pointer(&mut m, x + 40, y + 40, 1, false, 0);
        let c2 = d.view;
        rects.clear();
        changed(&b, &c2, |r| rects.push(r));
        assert!(rects.contains(&Rect {
            x: cell.x + 40,
            y: cell.y + 40,
            ..cell
        }));
        assert_eq!(changed_count(&c2, &c2), 0);
    }
    fn changed_count(a: &DeskView, b: &DeskView) -> usize {
        let mut n = 0;
        changed(a, b, |_| n += 1);
        n
    }
}
