//! The Files application's interaction (Phase 11.8): pointer, keys, menus,
//! drag-and-drop, rubber band and the properties sheet over an `Explorer`
//! and its view. Pure over a `Store`: the guest application passes a
//! `CapStore` over filesd, host tests an in-memory store.
use super::explorer::{
    Explorer, Info, Kind, Outcome, Path, Status, Store, TRASH, View, kind_of, sniff_text,
};
use super::explorer_view::{self as v, Hit, PLACES, SORTS, Ui};
use super::scene::Fnv;
use crate::files::describe_status;
use crate::model::{MOD_ALT, MOD_CTRL, MOD_SHIFT, POINTER_CTRL, POINTER_SHIFT};
use arena_gfxkit::{Canvas, Rect};
use arena_ui::theme::Theme;
use arena_ui::widgets::{self as w, Grid, List, ScrollHit};

/// What the application must do after an interaction.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Effect {
    None,
    /// Offer this file's capability to a new Editor.
    OpenInEditor(Path),
    /// Open the context menu (its items: `Controller::menu_items`).
    Menu {
        x: i32,
        y: i32,
    },
}

/// Context menu commands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cmd {
    Open,
    OpenWithEditor,
    Rename,
    Copy,
    Cut,
    Paste,
    Trash,
    Restore,
    DeleteForever,
    EmptyTrash,
    NewFolder,
    NewDocument,
    Properties,
    Refresh,
    ToggleView,
}

const ITEM_MENU: [(&str, Cmd); 7] = [
    ("Open", Cmd::Open),
    ("Open With Editor", Cmd::OpenWithEditor),
    ("Rename", Cmd::Rename),
    ("Copy", Cmd::Copy),
    ("Cut", Cmd::Cut),
    ("Move to Trash", Cmd::Trash),
    ("Properties", Cmd::Properties),
];
const FOLDER_MENU: [(&str, Cmd); 5] = [
    ("New Folder", Cmd::NewFolder),
    ("New Document", Cmd::NewDocument),
    ("Paste", Cmd::Paste),
    ("Refresh", Cmd::Refresh),
    ("Toggle List/Grid", Cmd::ToggleView),
];
const TRASH_ITEM_MENU: [(&str, Cmd); 3] = [
    ("Restore", Cmd::Restore),
    ("Delete Permanently", Cmd::DeleteForever),
    ("Properties", Cmd::Properties),
];
const TRASH_MENU: [(&str, Cmd); 2] = [("Empty Trash", Cmd::EmptyTrash), ("Refresh", Cmd::Refresh)];

/// A press in progress (left button held).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Press {
    /// On item `i`; `collapse`: a plain click on an already selected item
    /// selects only it on release (unless it became a drag).
    Item {
        x: i32,
        y: i32,
        i: usize,
        dragging: bool,
        collapse: bool,
    },
    Band {
        x: i32,
        y: i32,
    },
    Splitter,
    Thumb {
        y: i32,
        offset: i32,
    },
    Other,
}

const DOUBLE_US: u64 = 450_000;
const DRAG_SLOP: i32 = 4;

pub struct Controller {
    pub ex: Explorer,
    pub ui: Ui,
    /// The properties sheet's facts (`ui.sheet` holds its item).
    pub sheet: Option<Info>,
    pub size: (u16, u16),
    press: Option<Press>,
    buttons: u8,
    last_click: Option<(u64, usize)>,
    /// Which menu the context menu shows (chosen at the right click).
    menu: &'static [(&'static str, Cmd)],
    line: [u8; 64],
    line_n: usize,
}

impl Default for Controller {
    fn default() -> Self {
        Self::new()
    }
}

impl Controller {
    pub const fn new() -> Self {
        Controller {
            ex: Explorer::new(),
            ui: Ui {
                hover: None,
                drag: None,
                sidebar: 116,
                sheet: None,
                focused: true,
                band: None,
            },
            sheet: None,
            size: (448, 288),
            press: None,
            buttons: 0,
            last_click: None,
            menu: &FOLDER_MENU,
            line: [0; 64],
            line_n: 0,
        }
    }

    pub fn geometry(&self) -> v::Geometry {
        v::geometry(self.size.0, self.size.1, self.ui.sidebar)
    }
    fn layout(&mut self) {
        let g = self.geometry();
        v::layout(&mut self.ex, &g);
    }
    /// Re-lay out for a new window size.
    pub fn resize(&mut self, size: (u16, u16)) {
        self.size = size;
        self.layout();
    }

    /// Run `f`; a failure the explorer did not explain is described from
    /// its filesd status. Re-lays out afterwards.
    fn run<T>(&mut self, f: impl FnOnce(&mut Self) -> Result<T, Status>) -> Option<T> {
        self.ex.status = "";
        let r = f(self);
        self.layout();
        match r {
            Ok(v) => Some(v),
            Err(e) => {
                if self.ex.status.is_empty() {
                    self.ex.status = describe_status(e);
                }
                None
            }
        }
    }

    /// Show `to` first (the window's start folder).
    pub fn start(&mut self, s: &mut impl Store, to: Path) {
        self.run(|c| c.ex.go(s, to));
    }

    /// Poll: re-list when the folder changed underneath (another window,
    /// the terminal). True when something was re-listed.
    pub fn poll(&mut self, s: &mut impl Store) -> bool {
        if self.ex.rename.is_some() || self.press.is_some() {
            return false;
        }
        let changed = self.ex.changed(s);
        if changed {
            self.layout();
        }
        changed
    }

    // ---- pointer ----------------------------------------------------------------

    pub fn pointer(&mut self, s: &mut impl Store, x: i32, y: i32, buttons: u8, now: u64) -> Effect {
        let shift = buttons & POINTER_SHIFT != 0;
        let ctrl = buttons & POINTER_CTRL != 0;
        let (left, was) = (buttons & 1 != 0, self.buttons & 1 != 0);
        let context = buttons & 2 != 0 && self.buttons & 2 == 0;
        self.buttons = buttons;
        let g = self.geometry();
        let h = v::hit(&g, &self.ex, x, y);
        if context {
            return self.context(&g, h, x, y);
        }
        if left && !was {
            return self.press(s, &g, h, x, y, shift, ctrl, now);
        }
        if left {
            self.drag(&g, h, x, y);
            return Effect::None;
        }
        if was {
            return self.release(s, h);
        }
        self.ui.hover = (h != Hit::Blank).then_some(h);
        Effect::None
    }

    fn context(&mut self, g: &v::Geometry, h: Hit, x: i32, y: i32) -> Effect {
        if self.ui.sheet.take().is_some() {
            return Effect::None;
        }
        if self.ex.rename.is_some() {
            self.ex.cancel_rename();
        }
        let on_item = match h {
            Hit::Item(i) => {
                if !self.ex.selection.contains(i) {
                    self.ex.selection.click(i, false, false);
                }
                true
            }
            _ => {
                if inside(g.content, x, y) {
                    self.ex.selection.clear();
                }
                false
            }
        };
        self.menu = match (self.ex.in_trash(), on_item) {
            (false, true) => &ITEM_MENU,
            (false, false) => &FOLDER_MENU,
            (true, true) => &TRASH_ITEM_MENU,
            (true, false) => &TRASH_MENU,
        };
        Effect::Menu { x, y }
    }

    #[allow(clippy::too_many_arguments)]
    fn press(
        &mut self,
        s: &mut impl Store,
        g: &v::Geometry,
        h: Hit,
        x: i32,
        y: i32,
        shift: bool,
        ctrl: bool,
        now: u64,
    ) -> Effect {
        if self.ui.sheet.take().is_some() {
            self.sheet = None;
            self.press = Some(Press::Other);
            return Effect::None;
        }
        if let Some((r, _)) = self.ex.rename {
            if h == Hit::Item(r) {
                self.press = Some(Press::Other);
                return Effect::None;
            }
            // A click elsewhere commits the name (a refused name keeps
            // the field open with the reason in the status bar).
            if self.run(|c| c.ex.commit_rename(s)).is_none() {
                self.press = Some(Press::Other);
                return Effect::None;
            }
        }
        self.press = Some(Press::Other);
        match h {
            Hit::Back => {
                self.run(|c| c.ex.back(s));
            }
            Hit::Forward => {
                self.run(|c| c.ex.forward(s));
            }
            Hit::Up => {
                self.run(|c| c.ex.up(s));
            }
            Hit::Crumb(i) => {
                if !self.ex.in_trash() {
                    let to = prefix(&self.ex.path, i);
                    self.run(|c| c.ex.go(s, to));
                }
            }
            Hit::AsList => self.ex.view = View::List,
            Hit::AsGrid => self.ex.view = View::Grid,
            Hit::Place(p) => {
                let to = Path::of(PLACES[p].1);
                self.run(|c| c.ex.go(s, to));
            }
            Hit::Header(col) => self.ex.sort_by(SORTS[col]),
            Hit::Item(i) => {
                let double = !shift
                    && !ctrl
                    && self
                        .last_click
                        .is_some_and(|(t, j)| j == i && now.saturating_sub(t) <= DOUBLE_US);
                if double {
                    self.last_click = None;
                    return self.open(s, i, false);
                }
                let was_selected = self.ex.selection.contains(i);
                if shift || ctrl || !was_selected {
                    self.ex.selection.click(i, shift, ctrl);
                } else {
                    self.ex.selection.cursor = Some(i);
                }
                self.last_click = Some((now, i));
                self.press = Some(Press::Item {
                    x,
                    y,
                    i,
                    dragging: false,
                    collapse: was_selected && !shift && !ctrl,
                });
            }
            Hit::Scrollbar(ScrollHit::Thumb) => {
                self.press = Some(Press::Thumb {
                    y,
                    offset: self.scroll().offset,
                })
            }
            Hit::Scrollbar(ScrollHit::PageUp) => {
                let page = self.scroll().viewport;
                self.scroll_mut().by(-page);
            }
            Hit::Scrollbar(ScrollHit::PageDown) => {
                let page = self.scroll().viewport;
                self.scroll_mut().by(page);
            }
            Hit::Splitter => self.press = Some(Press::Splitter),
            Hit::Blank => {
                if inside(g.content, x, y) {
                    if !shift && !ctrl {
                        self.ex.selection.clear();
                    }
                    self.press = Some(Press::Band { x, y });
                }
            }
        }
        self.layout();
        Effect::None
    }

    fn drag(&mut self, g: &v::Geometry, h: Hit, x: i32, y: i32) {
        match self.press {
            Some(Press::Item {
                x: x0,
                y: y0,
                i,
                dragging,
                collapse,
            }) => {
                let moved = (x - x0).abs() > DRAG_SLOP || (y - y0).abs() > DRAG_SLOP;
                if !dragging && !moved {
                    return;
                }
                if !dragging && self.ex.in_trash() {
                    // Items leave the Trash by Restore, not by dragging.
                    return;
                }
                self.press = Some(Press::Item {
                    x: x0,
                    y: y0,
                    i,
                    dragging: true,
                    collapse,
                });
                let target = match h {
                    Hit::Item(j) if self.ex.at(j).dir && !self.ex.selection.contains(j) => Some(j),
                    _ => None,
                };
                self.ui.drag = Some((x, y, target));
                self.ui.hover = Some(h);
            }
            Some(Press::Band { x: x0, y: y0 }) => {
                let (bx, by) = (x0.min(x), y0.min(y));
                let (bw, bh) = ((x - x0).abs(), (y - y0).abs());
                self.ui.band = Some((bx, by, bw, bh));
                self.band_select(
                    g,
                    Rect {
                        x: bx,
                        y: by,
                        width: bw as u32,
                        height: bh as u32,
                    },
                );
            }
            Some(Press::Splitter) => {
                self.ui.sidebar = x.clamp(v::SIDEBAR_MIN, v::SIDEBAR_MAX);
                self.layout();
            }
            Some(Press::Thumb { y: y0, offset }) => {
                let track = match self.ex.view {
                    View::List => List::track(g.content),
                    View::Grid => Rect {
                        x: g.content.x + g.content.width as i32 - w::BAR,
                        y: g.content.y,
                        width: w::BAR as u32,
                        height: g.content.height,
                    },
                };
                let to = self.scroll().drag(track, offset, y - y0);
                self.scroll_mut().set(to);
            }
            _ => {}
        }
    }

    /// Select exactly the items the rubber band touches.
    fn band_select(&mut self, g: &v::Geometry, band: Rect) {
        self.ex.selection.clear();
        for i in 0..self.ex.count {
            let r = match self.ex.view {
                View::List => {
                    let body = List::body(g.content);
                    Rect {
                        x: body.x,
                        y: body.y + i as i32 * w::ROW - self.ex.list.scroll.offset,
                        width: body.width,
                        height: w::ROW as u32,
                    }
                }
                View::Grid => self.ex.grid.cell(g.content, i),
            };
            if touches(r, band) {
                self.ex.selection.click(i, false, true);
            }
        }
    }

    fn release(&mut self, s: &mut impl Store, h: Hit) -> Effect {
        let press = self.press.take();
        self.ui.band = None;
        match press {
            Some(Press::Item { dragging: true, .. }) => {
                self.ui.drag = None;
                match h {
                    Hit::Item(j) if self.ex.at(j).dir && !self.ex.selection.contains(j) => {
                        let name = *self.ex.at(j);
                        self.run(|c| {
                            let to = c.ex.path.child(name.name())?;
                            c.ex.move_selection_to(s, to)
                        });
                    }
                    Hit::Place(p) if PLACES[p].1 == TRASH => {
                        self.run(|c| c.ex.delete(s));
                    }
                    Hit::Place(p) => {
                        let to = Path::of(PLACES[p].1);
                        self.run(|c| c.ex.move_selection_to(s, to));
                    }
                    _ => {}
                }
            }
            Some(Press::Item {
                collapse: true, i, ..
            }) => self.ex.selection.click(i, false, false),
            _ => {}
        }
        self.layout();
        Effect::None
    }

    fn scroll(&self) -> w::Scroll {
        match self.ex.view {
            View::List => self.ex.list.scroll,
            View::Grid => self.ex.grid.scroll,
        }
    }
    fn scroll_mut(&mut self) -> &mut w::Scroll {
        match self.ex.view {
            View::List => &mut self.ex.list.scroll,
            View::Grid => &mut self.ex.grid.scroll,
        }
    }
    /// Wheel notches (positive: away from the user, scrolls back).
    pub fn wheel(&mut self, delta: i8) {
        let step = i32::from(delta) * 3 * w::ROW;
        self.scroll_mut().by(-step);
    }
    fn reveal_cursor(&mut self) {
        if let Some(i) = self.ex.selection.cursor {
            let g = self.geometry();
            self.ex.list.reveal(i);
            self.ex.grid.reveal(g.content, i);
        }
    }

    // ---- keyboard ----------------------------------------------------------------------

    pub fn key(&mut self, s: &mut impl Store, code: u16, shift: bool, now: u64) -> Effect {
        if self.ui.sheet.is_some() {
            if matches!(code, 13 | 27) {
                self.ui.sheet = None;
                self.sheet = None;
            }
            return Effect::None;
        }
        if let Some((_, input)) = self.ex.rename.as_mut() {
            match code {
                13 => {
                    self.run(|c| c.ex.commit_rename(s));
                }
                27 => self.ex.cancel_rename(),
                _ => {
                    input.key(code, shift);
                }
            }
            return Effect::None;
        }
        let n = self.ex.count;
        let cols = match self.ex.view {
            View::List => 1,
            View::Grid => Grid::columns(self.geometry().content) as isize,
        };
        match code {
            258 => self.ex.selection.step(-cols, n, shift),
            259 => self.ex.selection.step(cols, n, shift),
            256 if self.ex.view == View::Grid => self.ex.selection.step(-1, n, shift),
            257 if self.ex.view == View::Grid => self.ex.selection.step(1, n, shift),
            260 => self.ex.selection.step(-(n as isize), n, shift),
            261 => self.ex.selection.step(n as isize, n, shift),
            13 => {
                if let Some(i) = self.ex.selection.first() {
                    return self.open(s, i, false);
                }
            }
            8 => {
                self.run(|c| c.ex.up(s));
            }
            262 => {
                self.run(|c| c.ex.delete(s));
            }
            27 => self.ex.selection.clear(),
            32..=126 => self.ex.type_ahead(code as u8, now),
            _ => return Effect::None,
        }
        self.reveal_cursor();
        Effect::None
    }

    /// Ctrl/Alt chords and shifted navigation.
    pub fn chord(&mut self, s: &mut impl Store, code: u16, mods: u8, now: u64) -> Effect {
        if mods & (MOD_CTRL | MOD_ALT) == 0 {
            return self.key(s, code, mods & MOD_SHIFT != 0, now);
        }
        if self.ex.rename.is_some() || self.ui.sheet.is_some() {
            return Effect::None;
        }
        if mods & MOD_ALT != 0 {
            match code {
                256 => {
                    self.run(|c| c.ex.back(s));
                }
                257 => {
                    self.run(|c| c.ex.forward(s));
                }
                258 => {
                    self.run(|c| c.ex.up(s));
                }
                _ => {}
            }
            return Effect::None;
        }
        let shift = mods & MOD_SHIFT != 0 || (code as u8).is_ascii_uppercase();
        match (code as u8).to_ascii_lowercase() {
            b'a' => self.ex.selection.select_all(self.ex.count),
            b'c' => return self.command(s, Cmd::Copy),
            b'x' => return self.command(s, Cmd::Cut),
            b'v' => return self.command(s, Cmd::Paste),
            b'n' if shift => return self.command(s, Cmd::NewFolder),
            b'n' => return self.command(s, Cmd::NewDocument),
            b'r' => return self.command(s, Cmd::Rename),
            b'o' => return self.command(s, Cmd::Open),
            b'i' => return self.command(s, Cmd::Properties),
            b'1' => self.ex.view = View::List,
            b'2' => self.ex.view = View::Grid,
            _ => {}
        }
        self.layout();
        Effect::None
    }

    // ---- commands ----------------------------------------------------------------------

    pub fn menu_items(&self, out: &mut [&'static str; 8]) -> usize {
        for (o, (label, _)) in out.iter_mut().zip(self.menu) {
            *o = label;
        }
        self.menu.len()
    }
    /// Run context menu entry `item`.
    pub fn act(&mut self, s: &mut impl Store, item: usize) -> Effect {
        match self.menu.get(item) {
            Some(&(_, cmd)) => self.command(s, cmd),
            None => Effect::None,
        }
    }

    pub fn command(&mut self, s: &mut impl Store, cmd: Cmd) -> Effect {
        let first = self.ex.selection.first();
        match cmd {
            Cmd::Open => {
                if let Some(i) = first {
                    return self.open(s, i, false);
                }
            }
            Cmd::OpenWithEditor => {
                if let Some(i) = first {
                    return self.open(s, i, true);
                }
            }
            Cmd::Rename => {
                if let Some(i) = first {
                    self.ex.begin_rename(i);
                    self.ex.list.reveal(i);
                }
            }
            Cmd::Copy | Cmd::Cut => {
                self.run(|c| c.ex.copy(cmd == Cmd::Cut));
            }
            Cmd::Paste => {
                if self.ex.has_clip() && !self.ex.in_trash() {
                    self.run(|c| c.ex.paste(s));
                }
            }
            Cmd::Trash | Cmd::DeleteForever => {
                self.run(|c| c.ex.delete(s));
            }
            Cmd::Restore => {
                self.run(|c| c.ex.restore(s));
            }
            Cmd::EmptyTrash => {
                self.run(|c| c.ex.empty_trash(s));
            }
            Cmd::NewFolder => {
                if !self.ex.in_trash() {
                    self.run(|c| c.ex.new_folder(s));
                }
            }
            Cmd::NewDocument => {
                if !self.ex.in_trash() {
                    self.run(|c| c.ex.new_document(s));
                }
            }
            Cmd::Properties => {
                if let Some(i) = first
                    && let Some(info) = self.run(|c| c.ex.properties(s, i))
                {
                    self.sheet = Some(info);
                    self.ui.sheet = Some(i);
                }
            }
            Cmd::Refresh => {
                self.run(|c| c.ex.refresh(s));
            }
            Cmd::ToggleView => {
                self.ex.view = match self.ex.view {
                    View::List => View::Grid,
                    View::Grid => View::List,
                }
            }
        }
        self.layout();
        Effect::None
    }

    /// Open display item `i`: folders in place, documents by type (or in
    /// the Editor when asked and the content is text it can hold).
    fn open(&mut self, s: &mut impl Store, i: usize, with_editor: bool) -> Effect {
        if with_editor && !self.ex.at(i).dir {
            let it = *self.ex.at(i);
            let opened = self.run(|c| {
                let path = c.ex.path.child(it.name())?;
                let mut head = [0u8; 512];
                let n = s.read(path.bytes(), 0, &mut head)?;
                if sniff_text(&head[..n]) && it.size <= 4096 {
                    Ok(Some(path))
                } else {
                    c.ex.status = "THE EDITOR OPENS TEXT UP TO 4096 BYTES";
                    Ok(None)
                }
            });
            return match opened.flatten() {
                Some(p) => Effect::OpenInEditor(p),
                None => Effect::None,
            };
        }
        match self.run(|c| c.ex.open(s, i)) {
            Some(Outcome::OpenInEditor(p)) => Effect::OpenInEditor(p),
            _ => Effect::None,
        }
    }

    // ---- presentation ------------------------------------------------------------------

    /// The status bar: the last operation's outcome, else a summary.
    pub fn status_line(&mut self) -> &str {
        let mut out = Line::default();
        if !self.ex.status.is_empty() {
            out.push(self.ex.status.as_bytes());
        } else {
            let selected = (0..self.ex.count)
                .filter(|i| self.ex.selection.contains(*i))
                .count();
            out.number(self.ex.count as u64);
            out.push(if self.ex.count == 1 {
                b" ITEM"
            } else {
                b" ITEMS"
            });
            if self.ex.truncated {
                out.push(b" SHOWN (FOLDER HOLDS MORE)");
            }
            if selected > 0 {
                out.push(b" / ");
                out.number(selected as u64);
                out.push(b" SELECTED");
            }
            if self.ex.has_clip() {
                out.push(b" / CLIPBOARD READY");
            }
        }
        self.line = out.b;
        self.line_n = out.n;
        core::str::from_utf8(&self.line[..self.line_n]).unwrap_or("")
    }
    pub fn status(&self) -> &str {
        core::str::from_utf8(&self.line[..self.line_n]).unwrap_or("")
    }

    pub fn draw(&self, canvas: &mut Canvas<'_>, t: Theme) {
        let sheet = self.sheet.map(|i| (i.size, i.mtime, i.entries));
        v::draw(canvas, &self.ex, &self.ui, sheet, t);
    }

    /// Hash of everything `draw` shows (the content band's repaint key).
    pub fn fingerprint(&self, h: &mut Fnv) {
        let e = &self.ex;
        h.bytes(e.path.bytes())
            .u64(e.count as u64)
            .u64(u64::from(e.truncated))
            .u64(e.sort as u64)
            .u64(u64::from(e.descending))
            .u64(e.view as u64)
            .u64(u64::from(e.can_back()))
            .u64(u64::from(e.can_forward()))
            .u64(e.list.scroll.offset as u64)
            .u64(e.list.scroll.content as u64)
            .u64(e.list.scroll.viewport as u64)
            .u64(e.grid.scroll.offset as u64)
            .u64(e.grid.scroll.content as u64)
            .u64(e.grid.scroll.viewport as u64);
        for i in 0..e.count {
            let it = e.at(i);
            h.bytes(it.name())
                .u64(it.size)
                .u64(it.mtime)
                .u64(u64::from(it.dir))
                .u64(u64::from(e.selection.contains(i)));
        }
        if let Some((i, input)) = &e.rename {
            h.u64(*i as u64)
                .bytes(&input.buf[..input.len])
                .u64(input.cursor as u64)
                .u64(input.anchor.map_or(u64::MAX, |a| a as u64));
        }
        let u = &self.ui;
        let hit = |h: Option<Hit>| match h {
            None => 0,
            Some(x) => {
                let (a, b) = match x {
                    Hit::Back => (1, 0),
                    Hit::Forward => (2, 0),
                    Hit::Up => (3, 0),
                    Hit::Crumb(i) => (4, i),
                    Hit::AsList => (5, 0),
                    Hit::AsGrid => (6, 0),
                    Hit::Place(i) => (7, i),
                    Hit::Header(i) => (8, i),
                    Hit::Item(i) => (9, i),
                    Hit::Scrollbar(_) => (10, 0),
                    Hit::Splitter => (11, 0),
                    Hit::Blank => (12, 0),
                };
                a << 32 | b as u64
            }
        };
        h.u64(hit(u.hover))
            .u64(u64::from(u.focused))
            .u64(u.sidebar as u64)
            .u64(u.sheet.map_or(u64::MAX, |i| i as u64));
        if let Some((x, y, t)) = u.drag {
            h.u64(x as u64)
                .u64(y as u64)
                .u64(t.map_or(u64::MAX, |i| i as u64));
        }
        if let Some((x, y, bw, bh)) = u.band {
            h.u64(x as u64).u64(y as u64).u64(bw as u64).u64(bh as u64);
        }
        if let Some(i) = self.sheet {
            h.u64(i.size).u64(i.mtime).u64(u64::from(i.entries));
        }
        h.u64(self.selection_count() as u64);
    }
    fn selection_count(&self) -> usize {
        self.ex.selection.count()
    }

    /// Whether display item `i` may open in the Editor by type.
    pub fn is_text(&self, i: usize) -> bool {
        let it = self.ex.at(i);
        kind_of(it.name(), it.dir) == Kind::Text
    }
}

/// The first `i` components of `p` (0: home).
fn prefix(p: &Path, i: usize) -> Path {
    let mut out = Path::ROOT;
    for part in p
        .bytes()
        .split(|b| *b == b'/')
        .filter(|x| !x.is_empty())
        .take(i)
    {
        let _ = out.push(part);
    }
    out
}

fn inside(r: Rect, x: i32, y: i32) -> bool {
    x >= r.x && y >= r.y && x < r.x + r.width as i32 && y < r.y + r.height as i32
}
fn touches(a: Rect, b: Rect) -> bool {
    a.x < b.x + b.width as i32 + 1
        && b.x < a.x + a.width as i32
        && a.y < b.y + b.height as i32 + 1
        && b.y < a.y + a.height as i32
}

#[derive(Clone, Copy)]
struct Line {
    b: [u8; 64],
    n: usize,
}
impl Default for Line {
    fn default() -> Self {
        Line { b: [0; 64], n: 0 }
    }
}
impl Line {
    fn push(&mut self, s: &[u8]) {
        let k = s.len().min(64 - self.n);
        self.b[self.n..self.n + k].copy_from_slice(&s[..k]);
        self.n += k;
    }
    fn number(&mut self, mut v: u64) {
        let mut d = [0u8; 20];
        let mut k = 0;
        loop {
            d[k] = b'0' + (v % 10) as u8;
            k += 1;
            v /= 10;
            if v == 0 {
                break;
            }
        }
        for i in (0..k).rev() {
            self.push(&d[i..=i]);
        }
    }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::super::explorer::tests::Mem;
    use super::*;
    use std::boxed::Box;

    fn setup() -> (Box<Controller>, Mem) {
        let mut m = Mem::new();
        for d in ["Desktop", "Documents", "Documents/Projects", ".Trash"] {
            m.mkdir(d.as_bytes()).unwrap();
        }
        for (f, data) in [
            ("Documents/notes.txt", &b"hello"[..]),
            ("Documents/todo.txt", b"milk"),
            ("Documents/photo.png", b"\x89PNG"),
        ] {
            m.create(f.as_bytes()).unwrap();
            m.write(f.as_bytes(), 0, data).unwrap();
        }
        let mut c = Box::new(Controller::new());
        c.resize((640, 400));
        c.start(&mut m, Path::of(b"Documents"));
        (c, m)
    }
    fn index(c: &Controller, name: &str) -> usize {
        (0..c.ex.count)
            .find(|i| c.ex.at(*i).name() == name.as_bytes())
            .unwrap()
    }
    fn row_point(c: &Controller, i: usize) -> (i32, i32) {
        let g = c.geometry();
        (
            g.content.x + 30,
            g.content.y + w::HEADER + i as i32 * w::ROW + 5 - c.ex.list.scroll.offset,
        )
    }

    #[test]
    fn double_click_opens_folder_and_text_and_refuses_binary() {
        let (mut c, mut m) = setup();
        assert_eq!(c.ex.at(0).name(), b"Projects");
        let (x, y) = row_point(&c, 0);
        c.pointer(&mut m, x, y, 1, 1_000);
        c.pointer(&mut m, x, y, 0, 1_100);
        assert_eq!(c.pointer(&mut m, x, y, 1, 200_000), Effect::None);
        c.pointer(&mut m, x, y, 0, 200_100);
        assert_eq!(c.ex.path.bytes(), b"Documents/Projects");
        c.pointer(&mut m, 5, 5, 0, 0);
        c.chord(&mut m, 256, MOD_ALT, 0);
        assert_eq!(c.ex.path.bytes(), b"Documents");
        let i = index(&c, "notes.txt");
        let (x, y) = row_point(&c, i);
        c.pointer(&mut m, x, y, 1, 5_000_000);
        c.pointer(&mut m, x, y, 0, 5_000_100);
        assert_eq!(
            c.pointer(&mut m, x, y, 1, 5_300_000),
            Effect::OpenInEditor(Path::of(b"Documents/notes.txt"))
        );
        c.pointer(&mut m, x, y, 0, 5_300_100);
        let p = index(&c, "photo.png");
        c.ex.selection.click(p, false, false);
        assert_eq!(c.key(&mut m, 13, false, 0), Effect::None);
        assert_eq!(c.ex.status, "NO APPLICATION CAN OPEN THIS FILE");
        assert_eq!(c.command(&mut m, Cmd::OpenWithEditor), Effect::None);
        assert_eq!(c.ex.status, "THE EDITOR OPENS TEXT UP TO 4096 BYTES");
    }

    #[test]
    fn shift_ctrl_band_and_drag_into_folder_and_trash() {
        let (mut c, mut m) = setup();
        let a = index(&c, "notes.txt");
        let b = index(&c, "todo.txt");
        let (x, y) = row_point(&c, a);
        c.pointer(&mut m, x, y, 1, 0);
        c.pointer(&mut m, x, y, 0, 0);
        let (x2, y2) = row_point(&c, b);
        c.pointer(&mut m, x2, y2, 1 | POINTER_SHIFT, 10_000_000);
        c.pointer(&mut m, x2, y2, POINTER_SHIFT, 10_000_000);
        assert_eq!(c.ex.selection.count(), b - a + 1);
        // Ctrl toggles one out.
        let (xp, yp) = row_point(&c, index(&c, "photo.png"));
        c.pointer(&mut m, xp, yp, 1 | POINTER_CTRL, 20_000_000);
        c.pointer(&mut m, xp, yp, POINTER_CTRL, 20_000_000);
        assert!(!c.ex.selection.contains(index(&c, "photo.png")));
        assert_eq!(c.ex.selection.count(), 2);
        // Drag the two onto the Projects folder (row 0).
        let (fx, fy) = row_point(&c, 0);
        c.pointer(&mut m, x, y, 1, 30_000_000);
        c.pointer(&mut m, x + 10, y - 10, 1, 30_000_000);
        c.pointer(&mut m, fx, fy, 1, 30_000_000);
        assert_eq!(c.ui.drag, Some((fx, fy, Some(0))));
        c.pointer(&mut m, fx, fy, 0, 30_000_000);
        assert!(m.nodes.contains_key(&b"Documents/Projects/notes.txt"[..]));
        assert!(m.nodes.contains_key(&b"Documents/Projects/todo.txt"[..]));
        assert_eq!(c.ui.drag, None);
        // Rubber band from blank space below the rows selects what it touches.
        let g = c.geometry();
        let below = g.content.y + w::HEADER + c.ex.count as i32 * w::ROW + 30;
        c.pointer(&mut m, g.content.x + 200, below, 1, 40_000_000);
        c.pointer(
            &mut m,
            g.content.x + 210,
            g.content.y + w::HEADER + 2,
            1,
            40_000_000,
        );
        assert_eq!(c.ex.selection.count(), c.ex.count);
        c.pointer(
            &mut m,
            g.content.x + 210,
            g.content.y + w::HEADER + 2,
            0,
            40_000_000,
        );
        assert_eq!(c.ui.band, None);
        // Drag photo.png onto the Trash place.
        let p = index(&c, "photo.png");
        let (px, py) = row_point(&c, p);
        c.pointer(&mut m, px, py, 1, 50_000_000);
        c.pointer(&mut m, px, py, 0, 50_000_000);
        c.pointer(&mut m, px, py, 1, 60_000_000);
        let trash_y = g.sidebar.y + 8 + 3 * 22 + 5;
        c.pointer(&mut m, 30, trash_y, 1, 60_000_000);
        c.pointer(&mut m, 30, trash_y, 0, 60_000_000);
        assert!(m.nodes.contains_key(&b".Trash/photo.png"[..]));
        assert_eq!(c.ex.status, "MOVED TO TRASH");
    }

    #[test]
    fn keyboard_rename_clipboard_menu_and_sheet() {
        let (mut c, mut m) = setup();
        let i = index(&c, "notes.txt");
        c.ex.selection.click(i, false, false);
        c.chord(&mut m, u16::from(b'r'), MOD_CTRL, 0);
        assert!(c.ex.rename.is_some());
        for k in b"memo" {
            c.key(&mut m, u16::from(*k), false, 0);
        }
        c.key(&mut m, 13, false, 0);
        assert!(m.nodes.contains_key(&b"Documents/memo.txt"[..]));
        assert_eq!(c.ex.status, "RENAMED");
        // Copy, go home, paste.
        c.chord(&mut m, u16::from(b'c'), MOD_CTRL, 0);
        c.chord(&mut m, 258, MOD_ALT, 0);
        assert_eq!(c.ex.path.bytes(), b"");
        c.chord(&mut m, u16::from(b'v'), MOD_CTRL, 0);
        assert_eq!(
            m.nodes.get(&b"memo.txt"[..]),
            Some(&Some(b"hello".to_vec()))
        );
        // Ctrl+Shift+N: a new folder, named inline.
        c.chord(&mut m, u16::from(b'N'), MOD_CTRL | MOD_SHIFT, 0);
        assert!(m.nodes.contains_key(&b"New Folder"[..]));
        assert!(c.ex.rename.is_some());
        c.key(&mut m, 27, false, 0);
        // Right click on an item: the item menu; Properties opens the sheet.
        let k = index(&c, "memo.txt");
        let (x, y) = row_point(&c, k);
        assert_eq!(c.pointer(&mut m, x, y, 2, 0), Effect::Menu { x, y });
        let mut labels = [""; 8];
        let n = c.menu_items(&mut labels);
        assert_eq!(
            &labels[..n],
            [
                "Open",
                "Open With Editor",
                "Rename",
                "Copy",
                "Cut",
                "Move to Trash",
                "Properties"
            ]
        );
        c.act(&mut m, 6);
        assert_eq!(c.sheet.map(|s| s.size), Some(5));
        assert_eq!(c.ui.sheet, Some(k));
        c.key(&mut m, 27, false, 0);
        assert_eq!(c.ui.sheet, None);
        // Delete key: to the Trash; there, the Trash menu restores.
        c.ex.selection.click(index(&c, "memo.txt"), false, false);
        c.key(&mut m, 262, false, 0);
        assert!(m.nodes.contains_key(&b".Trash/memo.txt"[..]));
        let g = c.geometry();
        c.pointer(&mut m, 30, g.sidebar.y + 8 + 3 * 22 + 5, 1, 0);
        c.pointer(&mut m, 30, g.sidebar.y + 8 + 3 * 22 + 5, 0, 0);
        assert!(c.ex.in_trash());
        let t = index(&c, "memo.txt");
        let (x, y) = row_point(&c, t);
        c.pointer(&mut m, x, y, 2, 0);
        let n = c.menu_items(&mut labels);
        assert_eq!(
            &labels[..n],
            ["Restore", "Delete Permanently", "Properties"]
        );
        c.act(&mut m, 0);
        assert!(m.nodes.contains_key(&b"memo.txt"[..]));
        assert!(!m.nodes.contains_key(&b".Trash/memo.txt"[..]));
    }

    #[test]
    fn fingerprint_tracks_visible_state_only() {
        let (mut c, mut m) = setup();
        let key = |c: &Controller| {
            let mut h = Fnv::new();
            c.fingerprint(&mut h);
            h.0
        };
        let k0 = key(&c);
        c.pointer(&mut m, 1, 1, 0, 0); // over nothing: no visible change
        assert_eq!(key(&c), k0);
        let (x, y) = row_point(&c, 1);
        c.pointer(&mut m, x, y, 0, 0);
        assert_ne!(key(&c), k0, "row hover");
        let k1 = key(&c);
        c.key(&mut m, 259, false, 0);
        assert_ne!(key(&c), k1, "selection");
        let k2 = key(&c);
        c.status_line();
        assert_eq!(key(&c), k2);
        c.wheel(-1);
        let _ = key(&c);
    }

    /// The guest workflow's opening at the guest window size (448x288).
    #[test]
    fn guest_sequence_on_the_default_window() {
        let mut m = Mem::new();
        for d in ["Desktop", "Documents", ".Trash"] {
            m.mkdir(d.as_bytes()).unwrap();
        }
        for (f, data) in [
            ("Documents/user-note", &b"Aa file manager"[..]),
            ("Documents/user-full", b"ffff"),
            ("Documents/user-bin", b"\x80binary"),
        ] {
            m.create(f.as_bytes()).unwrap();
            m.write(f.as_bytes(), 0, data).unwrap();
        }
        let mut c = Box::new(Controller::new());
        c.resize((448, 288));
        c.start(&mut m, Path::of(b"Documents"));
        let g = c.geometry();
        let row = |i: i32| (g.content.x + 40, g.content.y + 22 + 20 * i + 10);
        let place = |i: i32| (30, g.sidebar.y + 8 + 22 * i + 11);
        let mut now = 0u64;
        let mut click = |c: &mut Controller, m: &mut Mem, (x, y): (i32, i32)| {
            now += 1_000_000;
            c.pointer(m, x, y, 1, now);
            c.pointer(m, x, y, 0, now);
        };
        c.chord(&mut m, u16::from(b'N'), MOD_CTRL | MOD_SHIFT, 0);
        for k in b"Projects" {
            c.key(&mut m, u16::from(*k), false, 0);
        }
        c.key(&mut m, 13, false, 0);
        assert!(m.nodes.contains_key(&b"Documents/Projects"[..]));
        click(&mut c, &mut m, row(3));
        c.chord(&mut m, u16::from(b'c'), MOD_CTRL, 0);
        let (x, y) = row(0);
        c.pointer(&mut m, x, y, 1, 50_000_000);
        c.pointer(&mut m, x, y, 0, 50_000_000);
        c.pointer(&mut m, x, y, 1, 50_100_000);
        c.pointer(&mut m, x, y, 0, 50_100_000);
        assert_eq!(c.ex.path.bytes(), b"Documents/Projects");
        c.chord(&mut m, u16::from(b'v'), MOD_CTRL, 0);
        assert!(m.nodes.contains_key(&b"Documents/Projects/user-note"[..]));
        c.chord(&mut m, 256, MOD_ALT, 0);
        click(&mut c, &mut m, row(2));
        c.chord(&mut m, u16::from(b'x'), MOD_CTRL, 0);
        click(&mut c, &mut m, place(1));
        c.chord(&mut m, u16::from(b'v'), MOD_CTRL, 0);
        assert!(m.nodes.contains_key(&b"Desktop/user-full"[..]));
        click(&mut c, &mut m, place(2));
        assert_eq!(c.ex.path.bytes(), b"Documents");
        click(&mut c, &mut m, row(1));
        assert_eq!(c.ex.at(1).name(), b"user-bin");
        assert!(c.ex.selection.contains(1), "{:?}", c.ex.selection);
        c.key(&mut m, 262, false, 0);
        assert!(
            m.nodes.contains_key(&b".Trash/user-bin"[..]),
            "{}",
            c.ex.status
        );
    }
}
