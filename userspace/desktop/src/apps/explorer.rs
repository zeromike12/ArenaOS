//! The file explorer's logic (Phase 11.8): navigation, history, sorting,
//! selection, rename, Trash with restore, copy/cut/paste, drag-to-folder
//! moves, type-ahead and open-by-type. No pixels and no IPC here: every
//! filesystem effect goes through `Store`, implemented over filesd
//! capabilities in the application and in memory in the host tests.
//!
//! Paths are relative to the explorer's root capability (the user's home)
//! and are presentation only; the store resolves them from its capability.
use crate::files::wire;
use arena_ui::widgets::{Grid, Input, List, Selection};

pub type Status = u64;
pub const NAME_MAX: usize = 255;
pub const PATH_MAX: usize = 255;
/// Items one folder view holds; a larger folder says so honestly.
pub const ITEMS: usize = 256;
const HISTORY: usize = 16;
const CLIP: usize = 32;

/// One listed entry.
#[derive(Clone, Copy)]
pub struct Item {
    pub name: [u8; NAME_MAX],
    pub len: u8,
    pub dir: bool,
    pub size: u64,
    /// Wall microseconds; 0 = unknown.
    pub mtime: u64,
}
impl Item {
    pub const EMPTY: Item = Item {
        name: [0; NAME_MAX],
        len: 0,
        dir: false,
        size: 0,
        mtime: 0,
    };
    pub fn name(&self) -> &[u8] {
        &self.name[..usize::from(self.len)]
    }
    pub fn set(&mut self, name: &[u8], dir: bool, size: u64, mtime: u64) {
        let n = name.len().min(NAME_MAX);
        self.name[..n].copy_from_slice(&name[..n]);
        self.len = n as u8;
        self.dir = dir;
        self.size = size;
        self.mtime = mtime;
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Info {
    pub dir: bool,
    pub size: u64,
    pub mtime: u64,
    pub entries: u32,
    pub version: u64,
}

/// What the explorer needs from storage, by path below its root.
pub trait Store {
    /// Entries of `dir` after name `after`, in name order: (count, more).
    fn list(&mut self, dir: &[u8], after: &[u8], out: &mut [Item])
    -> Result<(usize, bool), Status>;
    fn stat(&mut self, path: &[u8]) -> Result<Info, Status>;
    fn mkdir(&mut self, path: &[u8]) -> Result<(), Status>;
    fn create(&mut self, path: &[u8]) -> Result<(), Status>;
    fn rename(&mut self, from: &[u8], to: &[u8]) -> Result<(), Status>;
    fn unlink(&mut self, path: &[u8]) -> Result<(), Status>;
    fn rmdir(&mut self, path: &[u8]) -> Result<(), Status>;
    fn read(&mut self, path: &[u8], offset: u64, out: &mut [u8]) -> Result<usize, Status>;
    fn write(&mut self, path: &[u8], offset: u64, data: &[u8]) -> Result<usize, Status>;
}

/// A path below the root, normalized (no leading `/`, no `.`/`..`).
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Path {
    pub b: [u8; PATH_MAX],
    pub n: usize,
}
impl core::fmt::Debug for Path {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "Path({:?})",
            core::str::from_utf8(self.bytes()).unwrap_or("?")
        )
    }
}
impl Path {
    pub const ROOT: Path = Path {
        b: [0; PATH_MAX],
        n: 0,
    };
    pub fn of(s: &[u8]) -> Path {
        let mut p = Path::ROOT;
        let _ = p.push_all(s);
        p
    }
    pub fn bytes(&self) -> &[u8] {
        &self.b[..self.n]
    }
    fn push_all(&mut self, s: &[u8]) -> Result<(), Status> {
        for part in s.split(|b| *b == b'/').filter(|p| !p.is_empty()) {
            self.push(part)?;
        }
        Ok(())
    }
    pub fn push(&mut self, name: &[u8]) -> Result<(), Status> {
        let extra = name.len() + usize::from(self.n != 0);
        if self.n + extra > PATH_MAX || !valid_name(name) {
            return Err(wire::S_INVAL);
        }
        if self.n != 0 {
            self.b[self.n] = b'/';
            self.n += 1;
        }
        self.b[self.n..self.n + name.len()].copy_from_slice(name);
        self.n += name.len();
        Ok(())
    }
    pub fn child(&self, name: &[u8]) -> Result<Path, Status> {
        let mut p = *self;
        p.push(name)?;
        Ok(p)
    }
    pub fn parent(&self) -> Path {
        let mut p = *self;
        p.n = self.b[..self.n]
            .iter()
            .rposition(|b| *b == b'/')
            .unwrap_or(0);
        p
    }
    pub fn name(&self) -> &[u8] {
        let start = self.b[..self.n]
            .iter()
            .rposition(|b| *b == b'/')
            .map_or(0, |i| i + 1);
        &self.b[start..self.n]
    }
    pub fn starts_with(&self, other: &Path) -> bool {
        other.n == 0
            || (self.n >= other.n
                && self.b[..other.n] == other.b[..other.n]
                && (self.n == other.n || self.b[other.n] == b'/'))
    }
}

/// A name the user may give: not empty, not `.`/`..`, no `/` or control.
pub fn valid_name(name: &[u8]) -> bool {
    !name.is_empty()
        && name.len() <= NAME_MAX
        && name != b"."
        && name != b".."
        && name.iter().all(|b| *b >= 0x20 && *b != 0x7f && *b != b'/')
}

pub const TRASH: &[u8] = b".Trash";
const RESTORE: &[u8] = b".restore";

/// What a file is, by extension and content: only kinds something can open.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Folder,
    Text,
    Other,
}

pub fn kind_of(name: &[u8], dir: bool) -> Kind {
    if dir {
        return Kind::Folder;
    }
    let ext = name
        .iter()
        .rposition(|b| *b == b'.')
        .map(|i| &name[i + 1..]);
    match ext {
        Some(e)
            if e.eq_ignore_ascii_case(b"txt")
                || e.eq_ignore_ascii_case(b"md")
                || e.eq_ignore_ascii_case(b"log") =>
        {
            Kind::Text
        }
        // No extension: text when the content sniff says so (the caller checks).
        None => Kind::Text,
        _ => Kind::Other,
    }
}

pub fn kind_label(k: Kind) -> &'static str {
    match k {
        Kind::Folder => "Folder",
        Kind::Text => "Text document",
        Kind::Other => "Document",
    }
}

/// Bytes look like editable text (the Editor's own acceptance rule).
pub fn sniff_text(data: &[u8]) -> bool {
    data.iter()
        .all(|b| b.is_ascii_graphic() || matches!(b, b' ' | b'\n' | b'\t'))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sort {
    Name,
    Size,
    Modified,
    Kind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum View {
    List,
    Grid,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Clip {
    Copy,
    Cut,
}

/// What the application must do after an explorer action (a `Path` is a
/// fixed array: there is no heap to box it).
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    None,
    /// Open this file in the Editor (the app offers its capability).
    OpenInEditor(Path),
    /// Nothing can open this kind; say so.
    NoApplication,
}

pub struct Explorer {
    pub path: Path,
    back: [Path; HISTORY],
    back_n: usize,
    fwd: [Path; HISTORY],
    fwd_n: usize,
    pub items: [Item; ITEMS],
    pub count: usize,
    /// The folder held more items than one view shows.
    pub truncated: bool,
    /// Display order: indices into `items`.
    pub order: [u16; ITEMS],
    pub sort: Sort,
    pub descending: bool,
    pub selection: Selection,
    pub view: View,
    pub list: List,
    pub grid: Grid,
    clip: [Path; CLIP],
    clip_n: usize,
    clip_mode: Clip,
    /// Inline rename of the item at display index.
    pub rename: Option<(usize, Input)>,
    typeahead: [u8; 32],
    typeahead_n: usize,
    typeahead_at: u64,
    pub status: &'static str,
    /// The listed folder's version (a change re-lists it).
    version: u64,
    pub show_hidden: bool,
}

impl Default for Explorer {
    fn default() -> Self {
        Self::new()
    }
}

fn cmp_names(a: &[u8], b: &[u8]) -> core::cmp::Ordering {
    let la = a.iter().map(u8::to_ascii_lowercase);
    let lb = b.iter().map(u8::to_ascii_lowercase);
    la.cmp(lb).then(a.cmp(b))
}

impl Explorer {
    pub const fn new() -> Self {
        Explorer {
            path: Path::ROOT,
            back: [Path::ROOT; HISTORY],
            back_n: 0,
            fwd: [Path::ROOT; HISTORY],
            fwd_n: 0,
            items: [Item::EMPTY; ITEMS],
            count: 0,
            truncated: false,
            order: [0; ITEMS],
            sort: Sort::Name,
            descending: false,
            selection: Selection::EMPTY,
            view: View::List,
            list: List::new(),
            grid: Grid {
                items: 0,
                scroll: arena_ui::widgets::Scroll {
                    offset: 0,
                    content: 0,
                    viewport: 0,
                },
            },
            clip: [Path::ROOT; CLIP],
            clip_n: 0,
            clip_mode: Clip::Copy,
            rename: None,
            typeahead: [0; 32],
            typeahead_n: 0,
            typeahead_at: 0,
            status: "",
            version: 0,
            show_hidden: false,
        }
    }

    /// The item shown at display position `i`.
    pub fn at(&self, i: usize) -> &Item {
        &self.items[usize::from(self.order[i])]
    }
    pub fn in_trash(&self) -> bool {
        self.path.bytes() == TRASH
    }

    // ---- listing ---------------------------------------------------------------

    /// List the current folder again (selection is kept by name).
    pub fn refresh(&mut self, s: &mut impl Store) -> Result<(), Status> {
        let keep = self.selected_names();
        let mut buf = [Item::EMPTY; 16];
        let mut after = [0u8; NAME_MAX];
        let mut after_n = 0usize;
        self.count = 0;
        self.truncated = false;
        loop {
            let (n, more) = s.list(self.path.bytes(), &after[..after_n], &mut buf)?;
            for it in &buf[..n] {
                let hidden = it.name().first() == Some(&b'.');
                if hidden && !self.show_hidden {
                    continue;
                }
                if self.count == ITEMS {
                    self.truncated = true;
                    break;
                }
                self.items[self.count] = *it;
                self.count += 1;
            }
            if n == 0 || !more || self.truncated {
                break;
            }
            let last = buf[n - 1];
            after[..last.name().len()].copy_from_slice(last.name());
            after_n = last.name().len();
        }
        self.version = s.stat(self.path.bytes()).map(|i| i.version).unwrap_or(0);
        self.resort();
        self.selection.clear();
        self.reselect(&keep);
        Ok(())
    }

    fn selected_names(&self) -> [[u8; NAME_MAX + 1]; 8] {
        let mut out = [[0u8; NAME_MAX + 1]; 8];
        let mut k = 0;
        for i in 0..self.count {
            if self.selection.contains(i) && k < 8 {
                let it = self.at(i);
                out[k][0] = it.len;
                out[k][1..=it.name().len()].copy_from_slice(it.name());
                k += 1;
            }
        }
        out
    }
    fn reselect(&mut self, keep: &[[u8; NAME_MAX + 1]; 8]) {
        for name in keep.iter().take(keep_n(keep)) {
            let n = &name[1..=usize::from(name[0])];
            if let Some(i) = (0..self.count).find(|i| self.at(*i).name() == n) {
                self.selection.click(i, false, true);
            }
        }
    }

    /// Re-list when the folder changed since it was listed (polled while the
    /// window is open: filesd bumps a folder's version on every change).
    pub fn changed(&mut self, s: &mut impl Store) -> bool {
        match s.stat(self.path.bytes()) {
            Ok(i) if i.version != self.version => self.refresh(s).is_ok(),
            _ => false,
        }
    }

    pub fn resort(&mut self) {
        for i in 0..self.count {
            self.order[i] = i as u16;
        }
        let (sort, desc) = (self.sort, self.descending);
        let items = &self.items;
        let order = &mut self.order[..self.count];
        // Folders first, then the chosen key; ties by name.
        order.sort_unstable_by(|&a, &b| {
            let (x, y) = (&items[usize::from(a)], &items[usize::from(b)]);
            let by = match sort {
                Sort::Name => cmp_names(x.name(), y.name()),
                Sort::Size => x.size.cmp(&y.size).then(cmp_names(x.name(), y.name())),
                Sort::Modified => x.mtime.cmp(&y.mtime).then(cmp_names(x.name(), y.name())),
                Sort::Kind => (kind_of(x.name(), x.dir) as u8)
                    .cmp(&(kind_of(y.name(), y.dir) as u8))
                    .then(cmp_names(x.name(), y.name())),
            };
            y.dir.cmp(&x.dir).then(if desc { by.reverse() } else { by })
        });
    }

    /// A header click: sort by `sort`, toggling direction on the same key.
    pub fn sort_by(&mut self, sort: Sort) {
        if self.sort == sort {
            self.descending = !self.descending;
        } else {
            self.sort = sort;
            self.descending = false;
        }
        let keep = self.selected_names();
        self.resort();
        self.selection.clear();
        self.reselect(&keep);
    }

    // ---- navigation ----------------------------------------------------------------

    pub fn go(&mut self, s: &mut impl Store, to: Path) -> Result<(), Status> {
        if !s.stat(to.bytes())?.dir {
            return Err(wire::S_NOTDIR);
        }
        if to != self.path {
            if self.back_n == HISTORY {
                self.back.copy_within(1.., 0);
                self.back_n -= 1;
            }
            self.back[self.back_n] = self.path;
            self.back_n += 1;
            self.fwd_n = 0;
        }
        self.enter(s, to)
    }
    fn enter(&mut self, s: &mut impl Store, to: Path) -> Result<(), Status> {
        self.path = to;
        self.rename = None;
        self.selection.clear();
        self.list.scroll.offset = 0;
        self.grid.scroll.offset = 0;
        self.refresh(s)
    }
    pub fn can_back(&self) -> bool {
        self.back_n > 0
    }
    pub fn can_forward(&self) -> bool {
        self.fwd_n > 0
    }
    pub fn back(&mut self, s: &mut impl Store) -> Result<(), Status> {
        if self.back_n == 0 {
            return Ok(());
        }
        self.back_n -= 1;
        let to = self.back[self.back_n];
        if self.fwd_n < HISTORY {
            self.fwd[self.fwd_n] = self.path;
            self.fwd_n += 1;
        }
        self.enter(s, to)
    }
    pub fn forward(&mut self, s: &mut impl Store) -> Result<(), Status> {
        if self.fwd_n == 0 {
            return Ok(());
        }
        self.fwd_n -= 1;
        let to = self.fwd[self.fwd_n];
        if self.back_n < HISTORY {
            self.back[self.back_n] = self.path;
            self.back_n += 1;
        }
        self.enter(s, to)
    }
    pub fn up(&mut self, s: &mut impl Store) -> Result<(), Status> {
        if self.path.n == 0 {
            return Ok(());
        }
        let from = self.path;
        self.go(s, self.path.parent())?;
        // Land on the folder we came from.
        if let Some(i) = (0..self.count).find(|i| self.at(*i).name() == from.name()) {
            self.selection.click(i, false, false);
        }
        Ok(())
    }

    /// Enter or double-click on display item `i`.
    pub fn open(&mut self, s: &mut impl Store, i: usize) -> Result<Outcome, Status> {
        if i >= self.count {
            return Ok(Outcome::None);
        }
        let it = *self.at(i);
        let path = self.path.child(it.name())?;
        if it.dir {
            self.go(s, path)?;
            return Ok(Outcome::None);
        }
        match kind_of(it.name(), false) {
            Kind::Text => {
                // Sniff the first block: only text opens in the Editor.
                let mut head = [0u8; 512];
                let n = s.read(path.bytes(), 0, &mut head)?;
                if sniff_text(&head[..n]) && it.size <= 4096 {
                    Ok(Outcome::OpenInEditor(path))
                } else {
                    self.status = "NO APPLICATION CAN OPEN THIS FILE";
                    Ok(Outcome::NoApplication)
                }
            }
            _ => {
                self.status = "NO APPLICATION CAN OPEN THIS FILE";
                Ok(Outcome::NoApplication)
            }
        }
    }

    // ---- creating and renaming -----------------------------------------------------------

    /// A name in `dir` not yet taken: `base`, `base 2`, `base 3`, ...
    fn unique(
        &self,
        s: &mut impl Store,
        dir: &Path,
        base: &[u8],
        ext: &[u8],
    ) -> Result<Path, Status> {
        for n in 1..1000u32 {
            let mut name = [0u8; NAME_MAX];
            let mut k = 0;
            for part in [base, if n > 1 { b" " } else { b"" }] {
                name[k..k + part.len()].copy_from_slice(part);
                k += part.len();
            }
            if n > 1 {
                let mut digits = [0u8; 10];
                let mut v = n;
                let mut d = 0;
                while v > 0 {
                    digits[d] = b'0' + (v % 10) as u8;
                    v /= 10;
                    d += 1;
                }
                for j in (0..d).rev() {
                    name[k] = digits[j];
                    k += 1;
                }
            }
            if k + ext.len() > NAME_MAX {
                return Err(wire::S_INVAL);
            }
            name[k..k + ext.len()].copy_from_slice(ext);
            k += ext.len();
            let p = dir.child(&name[..k])?;
            match s.stat(p.bytes()) {
                Err(wire::S_NOENT) => return Ok(p),
                Err(e) => return Err(e),
                Ok(_) => {}
            }
        }
        Err(wire::S_EXIST)
    }

    /// New Folder: created at once and left in rename mode.
    pub fn new_folder(&mut self, s: &mut impl Store) -> Result<(), Status> {
        let p = self.unique(s, &self.path.clone(), b"New Folder", b"")?;
        s.mkdir(p.bytes())?;
        self.refresh(s)?;
        self.begin_rename_named(p.name());
        Ok(())
    }
    /// New Document (an empty text file), left in rename mode.
    pub fn new_document(&mut self, s: &mut impl Store) -> Result<(), Status> {
        let p = self.unique(s, &self.path.clone(), b"Untitled", b".txt")?;
        s.create(p.bytes())?;
        self.refresh(s)?;
        self.begin_rename_named(p.name());
        Ok(())
    }
    fn begin_rename_named(&mut self, name: &[u8]) {
        if let Some(i) = (0..self.count).find(|i| self.at(*i).name() == name) {
            self.selection.click(i, false, false);
            self.list.reveal(i);
            self.begin_rename(i);
        }
    }
    /// F2: rename item `i` inline, its name selected without the extension.
    pub fn begin_rename(&mut self, i: usize) {
        if i >= self.count {
            return;
        }
        let it = self.at(i);
        let mut input = Input::EMPTY;
        input.set(it.name());
        let stem = if it.dir {
            it.name().len()
        } else {
            it.name()
                .iter()
                .rposition(|b| *b == b'.')
                .filter(|p| *p > 0)
                .unwrap_or(it.name().len())
        };
        input.select(0, stem);
        self.rename = Some((i, input));
    }
    pub fn cancel_rename(&mut self) {
        self.rename = None;
    }
    pub fn commit_rename(&mut self, s: &mut impl Store) -> Result<(), Status> {
        let Some((i, input)) = self.rename else {
            return Ok(());
        };
        let new = &input.buf[..input.len];
        let old = *self.at(i);
        if new == old.name() {
            self.rename = None;
            return Ok(());
        }
        if !valid_name(new) {
            self.status = "A NAME CANNOT BE EMPTY OR CONTAIN /";
            return Err(wire::S_INVAL);
        }
        let to = self.path.child(new)?;
        match s.rename(self.path.child(old.name())?.bytes(), to.bytes()) {
            Ok(()) => {
                self.rename = None;
                self.refresh(s)?;
                self.begin_rename_named(new);
                self.rename = None;
                self.status = "RENAMED";
                Ok(())
            }
            Err(wire::S_EXIST) => {
                self.status = "AN ITEM WITH THAT NAME ALREADY EXISTS";
                Err(wire::S_EXIST)
            }
            Err(e) => Err(e),
        }
    }

    // ---- Trash ---------------------------------------------------------------------------

    fn selected_paths(&self, out: &mut [Path; CLIP]) -> Result<usize, Status> {
        let mut k = 0;
        for i in 0..self.count {
            if self.selection.contains(i) {
                if k == CLIP {
                    break;
                }
                out[k] = self.path.child(self.at(i).name())?;
                k += 1;
            }
        }
        Ok(k)
    }

    /// Delete: move the selection to the Trash with a restore record of its
    /// folder (in the Trash itself, delete permanently).
    pub fn delete(&mut self, s: &mut impl Store) -> Result<usize, Status> {
        let mut paths = [Path::ROOT; CLIP];
        let n = self.selected_paths(&mut paths)?;
        let trash = Path::of(TRASH);
        let records = trash.child(RESTORE)?;
        if self.in_trash() {
            for p in &paths[..n] {
                remove_tree(s, p, 0)?;
                let _ = s.unlink(records.child(p.name())?.bytes());
            }
            self.status = "DELETED PERMANENTLY";
        } else {
            if s.stat(records.bytes()).is_err() {
                s.mkdir(records.bytes())?;
            }
            for p in &paths[..n] {
                let name = p.name();
                let (stem, ext) = split_ext(name);
                let to = self.unique(s, &trash, stem, ext)?;
                s.rename(p.bytes(), to.bytes())?;
                // The record holds where it came from: folder and original
                // name (a presentation path, resolved from the root again).
                let rec = records.child(to.name())?;
                let _ = s.unlink(rec.bytes());
                s.create(rec.bytes())?;
                s.write(rec.bytes(), 0, p.bytes())?;
            }
            self.status = "MOVED TO TRASH";
        }
        self.refresh(s)?;
        Ok(n)
    }

    /// Restore the selected Trash items under their original names in their
    /// folders (a taken name gets a free one; a vanished folder restores to
    /// the home folder).
    pub fn restore(&mut self, s: &mut impl Store) -> Result<usize, Status> {
        if !self.in_trash() {
            return Ok(0);
        }
        let mut paths = [Path::ROOT; CLIP];
        let n = self.selected_paths(&mut paths)?;
        let records = Path::of(TRASH).child(RESTORE)?;
        for p in &paths[..n] {
            let rec = records.child(p.name())?;
            let mut buf = [0u8; PATH_MAX];
            let len = s.read(rec.bytes(), 0, &mut buf).unwrap_or(0);
            // No record (put in the Trash some other way): home, own name.
            let origin = Path::of(&buf[..len]);
            let (mut dir, name) = if origin.n == 0 {
                (Path::ROOT, p.name())
            } else {
                (origin.parent(), origin.name())
            };
            if !s.stat(dir.bytes()).is_ok_and(|i| i.dir) || dir.starts_with(&Path::of(TRASH)) {
                dir = Path::ROOT;
            }
            let (stem, ext) = split_ext(name);
            let to = self.unique(s, &dir, stem, ext)?;
            s.rename(p.bytes(), to.bytes())?;
            let _ = s.unlink(rec.bytes());
        }
        self.status = "RESTORED";
        self.refresh(s)?;
        Ok(n)
    }

    /// Empty the Trash: everything in it, permanently.
    pub fn empty_trash(&mut self, s: &mut impl Store) -> Result<(), Status> {
        let trash = Path::of(TRASH);
        let mut buf = [Item::EMPTY; 16];
        loop {
            let (n, _) = s.list(trash.bytes(), b"", &mut buf)?;
            let victims = buf[..n].iter().filter(|it| it.name() != RESTORE).count();
            if victims == 0 {
                break;
            }
            for it in buf[..n].iter().filter(|it| it.name() != RESTORE) {
                remove_tree(s, &trash.child(it.name())?, 0)?;
            }
        }
        let records = trash.child(RESTORE)?;
        if s.stat(records.bytes()).is_ok() {
            remove_tree(s, &records, 0)?;
        }
        self.status = "TRASH EMPTIED";
        if self.in_trash() {
            self.refresh(s)?;
        }
        Ok(())
    }

    // ---- clipboard and moves ----------------------------------------------------------------

    pub fn copy(&mut self, cut: bool) -> Result<usize, Status> {
        let mut paths = [Path::ROOT; CLIP];
        let n = self.selected_paths(&mut paths)?;
        self.clip = paths;
        self.clip_n = n;
        self.clip_mode = if cut { Clip::Cut } else { Clip::Copy };
        self.status = if cut { "CUT" } else { "COPIED" };
        Ok(n)
    }
    pub fn has_clip(&self) -> bool {
        self.clip_n > 0
    }

    /// Paste into the current folder: copies get free names ("x 2"); a cut
    /// moves (and empties the clipboard).
    pub fn paste(&mut self, s: &mut impl Store) -> Result<usize, Status> {
        let dir = self.path;
        let n = self.clip_n;
        for i in 0..n {
            let from = self.clip[i];
            if dir.starts_with(&from) {
                self.status = "CANNOT PUT A FOLDER INSIDE ITSELF";
                return Err(wire::S_LOOP);
            }
            let (stem, ext) = split_ext(from.name());
            match self.clip_mode {
                Clip::Cut if from.parent() == dir => {}
                Clip::Cut => {
                    let to = self.unique(s, &dir, stem, ext)?;
                    s.rename(from.bytes(), to.bytes())?;
                }
                Clip::Copy => {
                    let to = self.unique(s, &dir, stem, ext)?;
                    copy_tree(s, &from, &to, 0)?;
                }
            }
        }
        if self.clip_mode == Clip::Cut {
            self.clip_n = 0;
        }
        self.status = "PASTED";
        self.refresh(s)?;
        Ok(n)
    }

    /// Drop the selection onto folder `target` (a display index or a path).
    pub fn move_selection_to(&mut self, s: &mut impl Store, target: Path) -> Result<usize, Status> {
        let mut paths = [Path::ROOT; CLIP];
        let n = self.selected_paths(&mut paths)?;
        let mut moved = 0;
        for p in &paths[..n] {
            if target.starts_with(p) {
                self.status = "CANNOT PUT A FOLDER INSIDE ITSELF";
                return Err(wire::S_LOOP);
            }
            if p.parent() == target {
                continue;
            }
            let (stem, ext) = split_ext(p.name());
            let to = self.unique(s, &target, stem, ext)?;
            s.rename(p.bytes(), to.bytes())?;
            moved += 1;
        }
        self.status = "MOVED";
        self.refresh(s)?;
        Ok(moved)
    }

    // ---- keyboard conveniences ---------------------------------------------------------------

    /// Type-ahead: letters typed within a second select the first item whose
    /// name starts with them (case-insensitive).
    pub fn type_ahead(&mut self, ch: u8, now_us: u64) {
        if now_us.saturating_sub(self.typeahead_at) > 1_000_000 {
            self.typeahead_n = 0;
        }
        self.typeahead_at = now_us;
        if self.typeahead_n < self.typeahead.len() {
            self.typeahead[self.typeahead_n] = ch.to_ascii_lowercase();
            self.typeahead_n += 1;
        }
        let prefix = &self.typeahead[..self.typeahead_n];
        if let Some(i) = (0..self.count).find(|i| {
            let n = self.at(*i).name();
            n.len() >= prefix.len() && n[..prefix.len()].eq_ignore_ascii_case(prefix)
        }) {
            self.selection.click(i, false, false);
            self.list.reveal(i);
        }
    }

    /// Properties of display item `i`.
    pub fn properties(&self, s: &mut impl Store, i: usize) -> Result<Info, Status> {
        s.stat(self.path.child(self.at(i).name())?.bytes())
    }
}

/// `Store` over filesd: paths are walked from the root capability on every
/// call (client-side, `..` never climbs out); intermediate capabilities are
/// released at once.
pub struct CapStore {
    pub fs: crate::files::Files,
    pub root: u64,
}

impl CapStore {
    fn with<T>(
        &mut self,
        path: &[u8],
        rights: u8,
        f: impl FnOnce(&crate::files::Files, u64) -> Result<T, Status>,
    ) -> Result<T, Status> {
        let (cap, _) = self.fs.walk(self.root, path, rights)?;
        let r = f(&self.fs, cap);
        if cap != self.root {
            self.fs.release(cap);
        }
        r
    }
    fn in_parent<T>(
        &mut self,
        path: &[u8],
        rights: u8,
        f: impl FnOnce(&crate::files::Files, u64, &[u8]) -> Result<T, Status>,
    ) -> Result<T, Status> {
        let (dir, name) = crate::files::split_last(path);
        if name.is_empty() {
            return Err(wire::S_INVAL);
        }
        self.with(dir, rights, |fs, cap| f(fs, cap, name))
    }
}

impl Store for CapStore {
    fn list(
        &mut self,
        dir: &[u8],
        after: &[u8],
        out: &mut [Item],
    ) -> Result<(usize, bool), Status> {
        self.with(dir, wire::R_LIST | wire::R_READ, |fs, cap| {
            let mut entries = [crate::files::Entry::EMPTY; 16];
            let k = out.len().min(entries.len());
            let (n, more) = fs.list(cap, after, &mut entries[..k])?;
            for (o, e) in out.iter_mut().zip(&entries[..n]) {
                o.set(e.name(), e.is_dir(), e.size, e.mtime);
            }
            Ok((n, more))
        })
    }
    fn stat(&mut self, path: &[u8]) -> Result<Info, Status> {
        self.with(path, wire::R_LIST, |fs, cap| {
            let st = fs.stat(cap, None)?;
            Ok(Info {
                dir: st.typ == 2,
                size: st.size,
                mtime: st.mtime,
                entries: st.entries,
                version: st.version,
            })
        })
    }
    fn mkdir(&mut self, path: &[u8]) -> Result<(), Status> {
        self.in_parent(path, wire::R_CREATE | wire::R_LIST, |fs, d, n| {
            fs.mkdir(d, n)
        })
    }
    fn create(&mut self, path: &[u8]) -> Result<(), Status> {
        self.in_parent(path, wire::R_CREATE | wire::R_LIST, |fs, d, n| {
            fs.create(d, n)
        })
    }
    fn rename(&mut self, from: &[u8], to: &[u8]) -> Result<(), Status> {
        let (fd, fname) = crate::files::split_last(from);
        let (td, tname) = crate::files::split_last(to);
        let (src, _) = self.fs.walk(self.root, fd, wire::R_RENAME | wire::R_LIST)?;
        let dst = match self.fs.walk(self.root, td, wire::R_CREATE | wire::R_LIST) {
            Ok((d, _)) => d,
            Err(e) => {
                if src != self.root {
                    self.fs.release(src);
                }
                return Err(e);
            }
        };
        let r = self.fs.rename(src, fname, Some(dst), tname);
        for c in [src, dst] {
            if c != self.root {
                self.fs.release(c);
            }
        }
        r
    }
    fn unlink(&mut self, path: &[u8]) -> Result<(), Status> {
        self.in_parent(path, wire::R_DELETE | wire::R_LIST, |fs, d, n| {
            fs.unlink(d, n)
        })
    }
    fn rmdir(&mut self, path: &[u8]) -> Result<(), Status> {
        self.in_parent(path, wire::R_DELETE | wire::R_LIST, |fs, d, n| {
            fs.rmdir(d, n)
        })
    }
    fn read(&mut self, path: &[u8], offset: u64, out: &mut [u8]) -> Result<usize, Status> {
        self.with(path, wire::R_READ, |fs, cap| fs.read(cap, offset, out))
    }
    fn write(&mut self, path: &[u8], offset: u64, data: &[u8]) -> Result<usize, Status> {
        self.with(path, wire::R_WRITE, |fs, cap| fs.write(cap, offset, data))
    }
}

fn keep_n(keep: &[[u8; NAME_MAX + 1]; 8]) -> usize {
    keep.iter().take_while(|n| n[0] != 0).count()
}

/// `name` split at its last dot (dotfiles and dot-less names keep it all).
pub fn split_ext(name: &[u8]) -> (&[u8], &[u8]) {
    match name.iter().rposition(|b| *b == b'.') {
        Some(i) if i > 0 => (&name[..i], &name[i..]),
        _ => (name, b""),
    }
}

const DEPTH: usize = 16;

/// Remove `p` and everything below it (bounded depth).
pub fn remove_tree(s: &mut impl Store, p: &Path, depth: usize) -> Result<(), Status> {
    let info = s.stat(p.bytes())?;
    if !info.dir {
        return s.unlink(p.bytes());
    }
    if depth >= DEPTH {
        return Err(wire::S_LOOP);
    }
    let mut buf = [Item::EMPTY; 16];
    loop {
        let (n, _) = s.list(p.bytes(), b"", &mut buf)?;
        if n == 0 {
            break;
        }
        for it in &buf[..n] {
            remove_tree(s, &p.child(it.name())?, depth + 1)?;
        }
    }
    s.rmdir(p.bytes())
}

/// Copy `from` to the new path `to` (folders recursively, bounded depth).
pub fn copy_tree(s: &mut impl Store, from: &Path, to: &Path, depth: usize) -> Result<(), Status> {
    let info = s.stat(from.bytes())?;
    if !info.dir {
        s.create(to.bytes())?;
        let mut buf = [0u8; 4096];
        let mut at = 0u64;
        loop {
            let n = s.read(from.bytes(), at, &mut buf)?;
            if n == 0 {
                break;
            }
            let mut w = 0;
            while w < n {
                w += s.write(to.bytes(), at + w as u64, &buf[w..n])?;
            }
            at += n as u64;
        }
        return Ok(());
    }
    if depth >= DEPTH {
        return Err(wire::S_LOOP);
    }
    s.mkdir(to.bytes())?;
    let mut buf = [Item::EMPTY; 16];
    let mut after = [0u8; NAME_MAX];
    let mut after_n = 0;
    loop {
        let (n, more) = s.list(from.bytes(), &after[..after_n], &mut buf)?;
        for it in &buf[..n] {
            copy_tree(s, &from.child(it.name())?, &to.child(it.name())?, depth + 1)?;
        }
        if n == 0 || !more {
            break;
        }
        let last = buf[n - 1];
        after[..last.name().len()].copy_from_slice(last.name());
        after_n = last.name().len();
    }
    Ok(())
}

#[cfg(test)]
pub mod tests {
    extern crate std;
    use super::*;
    use std::collections::BTreeMap;
    use std::vec::Vec;

    /// An in-memory store with filesd's semantics (refusals included).
    #[derive(Default)]
    pub struct Mem {
        pub nodes: BTreeMap<Vec<u8>, Option<Vec<u8>>>,
        pub versions: BTreeMap<Vec<u8>, u64>,
    }
    impl Mem {
        pub fn new() -> Self {
            let mut m = Mem::default();
            m.nodes.insert(Vec::new(), None);
            m
        }
        fn parent(p: &[u8]) -> Vec<u8> {
            p.iter()
                .rposition(|b| *b == b'/')
                .map_or(Vec::new(), |i| p[..i].to_vec())
        }
        fn bump(&mut self, p: &[u8]) {
            *self.versions.entry(Self::parent(p)).or_default() += 1;
        }
        fn need_dir(&self, p: &[u8]) -> Result<(), Status> {
            match self.nodes.get(p) {
                Some(None) => Ok(()),
                Some(Some(_)) => Err(wire::S_NOTDIR),
                None => Err(wire::S_NOENT),
            }
        }
        pub fn file(&mut self, p: &str, data: &[u8]) {
            self.nodes
                .insert(p.as_bytes().to_vec(), Some(data.to_vec()));
        }
        pub fn dir(&mut self, p: &str) {
            self.nodes.insert(p.as_bytes().to_vec(), None);
        }
        pub fn get(&self, p: &str) -> Option<&Option<Vec<u8>>> {
            self.nodes.get(p.as_bytes())
        }
        fn children(&self, dir: &[u8]) -> Vec<Vec<u8>> {
            self.nodes
                .keys()
                .filter(|k| !k.is_empty() && Self::parent(k) == dir)
                .map(|k| k[k.iter().rposition(|b| *b == b'/').map_or(0, |i| i + 1)..].to_vec())
                .collect()
        }
    }
    impl Store for Mem {
        fn list(
            &mut self,
            dir: &[u8],
            after: &[u8],
            out: &mut [Item],
        ) -> Result<(usize, bool), Status> {
            self.need_dir(dir)?;
            let mut names: Vec<Vec<u8>> = self
                .children(dir)
                .into_iter()
                .filter(|n| n.as_slice() > after)
                .collect();
            names.sort();
            let more = names.len() > out.len();
            for (o, n) in out.iter_mut().zip(names.iter()) {
                let mut full = dir.to_vec();
                if !full.is_empty() {
                    full.push(b'/');
                }
                full.extend_from_slice(n);
                let node = &self.nodes[&full];
                o.set(
                    n,
                    node.is_none(),
                    node.as_ref().map_or(0, |d| d.len() as u64),
                    7,
                );
            }
            Ok((names.len().min(out.len()), more))
        }
        fn stat(&mut self, p: &[u8]) -> Result<Info, Status> {
            match self.nodes.get(p) {
                Some(None) => Ok(Info {
                    dir: true,
                    entries: self.children(p).len() as u32,
                    version: *self.versions.get(p).unwrap_or(&0),
                    ..Info::default()
                }),
                Some(Some(d)) => Ok(Info {
                    size: d.len() as u64,
                    ..Info::default()
                }),
                None => Err(wire::S_NOENT),
            }
        }
        fn mkdir(&mut self, p: &[u8]) -> Result<(), Status> {
            self.need_dir(&Self::parent(p))?;
            if self.nodes.contains_key(p) {
                return Err(wire::S_EXIST);
            }
            self.nodes.insert(p.to_vec(), None);
            self.bump(p);
            Ok(())
        }
        fn create(&mut self, p: &[u8]) -> Result<(), Status> {
            self.need_dir(&Self::parent(p))?;
            if self.nodes.contains_key(p) {
                return Err(wire::S_EXIST);
            }
            self.nodes.insert(p.to_vec(), Some(Vec::new()));
            self.bump(p);
            Ok(())
        }
        fn rename(&mut self, from: &[u8], to: &[u8]) -> Result<(), Status> {
            if !self.nodes.contains_key(from) {
                return Err(wire::S_NOENT);
            }
            self.need_dir(&Self::parent(to))?;
            if self.nodes.contains_key(to) {
                return Err(wire::S_EXIST);
            }
            if to.starts_with(from) && to.get(from.len()) == Some(&b'/') {
                return Err(wire::S_LOOP);
            }
            let moved: Vec<Vec<u8>> = self
                .nodes
                .keys()
                .filter(|k| {
                    k.as_slice() == from
                        || (k.starts_with(from) && k.get(from.len()) == Some(&b'/'))
                })
                .cloned()
                .collect();
            for k in moved {
                let v = self.nodes.remove(&k).unwrap();
                let mut nk = to.to_vec();
                nk.extend_from_slice(&k[from.len()..]);
                self.nodes.insert(nk, v);
            }
            self.bump(from);
            self.bump(to);
            Ok(())
        }
        fn unlink(&mut self, p: &[u8]) -> Result<(), Status> {
            match self.nodes.get(p) {
                Some(Some(_)) => {
                    self.nodes.remove(p);
                    self.bump(p);
                    Ok(())
                }
                Some(None) => Err(wire::S_ISDIR),
                None => Err(wire::S_NOENT),
            }
        }
        fn rmdir(&mut self, p: &[u8]) -> Result<(), Status> {
            self.need_dir(p)?;
            if !self.children(p).is_empty() {
                return Err(wire::S_NOTEMPTY);
            }
            self.nodes.remove(p);
            self.bump(p);
            Ok(())
        }
        fn read(&mut self, p: &[u8], offset: u64, out: &mut [u8]) -> Result<usize, Status> {
            match self.nodes.get(p) {
                Some(Some(d)) => {
                    let start = (offset as usize).min(d.len());
                    let n = (d.len() - start).min(out.len());
                    out[..n].copy_from_slice(&d[start..start + n]);
                    Ok(n)
                }
                Some(None) => Err(wire::S_ISDIR),
                None => Err(wire::S_NOENT),
            }
        }
        fn write(&mut self, p: &[u8], offset: u64, data: &[u8]) -> Result<usize, Status> {
            match self.nodes.get_mut(p) {
                Some(Some(d)) => {
                    let end = offset as usize + data.len();
                    if d.len() < end {
                        d.resize(end, 0);
                    }
                    d[offset as usize..end].copy_from_slice(data);
                    self.bump(p);
                    Ok(data.len())
                }
                Some(None) => Err(wire::S_ISDIR),
                None => Err(wire::S_NOENT),
            }
        }
    }

    fn home() -> Mem {
        let mut m = Mem::new();
        for d in ["Desktop", "Documents", ".Trash", "Documents/Projects"] {
            m.dir(d);
        }
        m.file("Documents/notes.txt", b"hello");
        m.file("Documents/b.bin", &[0x80, 1, 2]);
        m.file("Documents/Projects/plan.md", b"# plan");
        m
    }

    fn names(e: &Explorer) -> Vec<std::string::String> {
        (0..e.count)
            .map(|i| std::string::String::from_utf8(e.at(i).name().to_vec()).unwrap())
            .collect()
    }

    #[test]
    fn listing_sorts_folders_first_and_hides_dotfiles() {
        let mut m = home();
        let mut e = Explorer::new();
        e.refresh(&mut m).unwrap();
        assert_eq!(names(&e), ["Desktop", "Documents"]);
        e.go(&mut m, Path::of(b"Documents")).unwrap();
        assert_eq!(names(&e), ["Projects", "b.bin", "notes.txt"]);
        e.sort_by(Sort::Size);
        assert_eq!(names(&e), ["Projects", "b.bin", "notes.txt"]);
        e.sort_by(Sort::Size);
        assert_eq!(names(&e), ["Projects", "notes.txt", "b.bin"]);
        e.sort_by(Sort::Kind);
        assert_eq!(names(&e), ["Projects", "notes.txt", "b.bin"]);
    }

    #[test]
    fn history_back_forward_up_and_open_by_type() {
        let mut m = home();
        let mut e = Explorer::new();
        e.refresh(&mut m).unwrap();
        assert_eq!(e.open(&mut m, 1).unwrap(), Outcome::None);
        assert_eq!(e.path.bytes(), b"Documents");
        assert_eq!(e.open(&mut m, 0).unwrap(), Outcome::None);
        assert_eq!(e.path.bytes(), b"Documents/Projects");
        e.back(&mut m).unwrap();
        assert_eq!(e.path.bytes(), b"Documents");
        e.forward(&mut m).unwrap();
        assert_eq!(e.path.bytes(), b"Documents/Projects");
        assert!(!e.can_forward());
        e.up(&mut m).unwrap();
        assert_eq!(e.path.bytes(), b"Documents");
        assert_eq!(
            e.selection.first().map(|i| e.at(i).name().to_vec()),
            Some(b"Projects".to_vec())
        );
        assert_eq!(
            e.open(&mut m, 2).unwrap(),
            Outcome::OpenInEditor(Path::of(b"Documents/notes.txt"))
        );
        assert_eq!(e.open(&mut m, 1).unwrap(), Outcome::NoApplication);
        assert!(e.go(&mut m, Path::of(b"Documents/notes.txt")).is_err());
    }

    #[test]
    fn new_folder_rename_and_refusals() {
        let mut m = home();
        let mut e = Explorer::new();
        e.go(&mut m, Path::of(b"Documents")).unwrap();
        e.new_folder(&mut m).unwrap();
        e.new_folder(&mut m).unwrap();
        assert!(
            m.get("Documents/New Folder").is_some() && m.get("Documents/New Folder 2").is_some()
        );
        let (i, mut input) = e.rename.unwrap();
        assert_eq!(e.at(i).name(), b"New Folder 2");
        assert_eq!(input.selection(), Some((0, 12)));
        input.set(b"Archive");
        e.rename = Some((i, input));
        e.commit_rename(&mut m).unwrap();
        assert!(m.get("Documents/Archive").is_some() && e.rename.is_none());
        // Rename onto an existing name is refused and stays in rename mode.
        let idx = names(&e).iter().position(|n| n == "notes.txt").unwrap();
        e.begin_rename(idx);
        assert_eq!(e.rename.unwrap().1.selection(), Some((0, 5)));
        let (i, mut input) = e.rename.unwrap();
        input.set(b"b.bin");
        e.rename = Some((i, input));
        assert_eq!(e.commit_rename(&mut m), Err(wire::S_EXIST));
        assert!(e.rename.is_some() && m.get("Documents/notes.txt").is_some());
        input.set(b"bad/name");
        e.rename = Some((i, input));
        assert_eq!(e.commit_rename(&mut m), Err(wire::S_INVAL));
        e.cancel_rename();
        e.new_document(&mut m).unwrap();
        assert_eq!(m.get("Documents/Untitled.txt"), Some(&Some(Vec::new())));
        assert_eq!(e.rename.unwrap().1.selection(), Some((0, 8)));
    }

    #[test]
    fn trash_restore_and_empty() {
        let mut m = home();
        let mut e = Explorer::new();
        e.go(&mut m, Path::of(b"Documents")).unwrap();
        m.file(".Trash/notes.txt", b"older");
        let i = names(&e).iter().position(|n| n == "notes.txt").unwrap();
        e.selection.click(i, false, false);
        assert_eq!(e.delete(&mut m).unwrap(), 1);
        assert!(m.get("Documents/notes.txt").is_none());
        assert_eq!(m.get(".Trash/notes 2.txt"), Some(&Some(b"hello".to_vec())));
        assert_eq!(
            m.get(".Trash/.restore/notes 2.txt"),
            Some(&Some(b"Documents/notes.txt".to_vec()))
        );
        e.go(&mut m, Path::of(TRASH)).unwrap();
        assert_eq!(names(&e), ["notes 2.txt", "notes.txt"]);
        e.selection.click(0, false, false);
        e.restore(&mut m).unwrap();
        assert_eq!(m.get("Documents/notes.txt"), Some(&Some(b"hello".to_vec())));
        assert!(m.get(".Trash/.restore/notes 2.txt").is_none());
        // An item whose folder vanished restores home; Delete in the Trash
        // is permanent; Empty Trash removes everything, folders included.
        m.dir(".Trash/old dir");
        m.file(".Trash/old dir/x", b"1");
        e.refresh(&mut m).unwrap();
        assert_eq!(names(&e), ["old dir", "notes.txt"]);
        e.selection.click(0, false, false);
        e.restore(&mut m).unwrap();
        assert_eq!(m.get("old dir/x"), Some(&Some(b"1".to_vec())));
        e.refresh(&mut m).unwrap();
        e.empty_trash(&mut m).unwrap();
        assert!(
            m.nodes.keys().all(|k| !k.starts_with(b".Trash/")),
            "{:?}",
            m.nodes.keys().collect::<Vec<_>>()
        );
    }

    #[test]
    fn copy_cut_paste_and_drag_moves() {
        let mut m = home();
        let mut e = Explorer::new();
        e.go(&mut m, Path::of(b"Documents")).unwrap();
        e.selection.click(0, false, false); // Projects
        e.copy(false).unwrap();
        e.paste(&mut m).unwrap();
        assert_eq!(
            m.get("Documents/Projects 2/plan.md"),
            Some(&Some(b"# plan".to_vec()))
        );
        // Paste a folder into itself is refused.
        e.go(&mut m, Path::of(b"Documents/Projects")).unwrap();
        assert_eq!(e.paste(&mut m), Err(wire::S_LOOP));
        e.back(&mut m).unwrap();
        let i = names(&e).iter().position(|n| n == "notes.txt").unwrap();
        e.selection.click(i, false, false);
        e.copy(true).unwrap();
        e.go(&mut m, Path::of(b"Desktop")).unwrap();
        e.paste(&mut m).unwrap();
        assert!(m.get("Documents/notes.txt").is_none() && m.get("Desktop/notes.txt").is_some());
        assert!(!e.has_clip());
        // Drag b.bin onto Projects; dragging Projects into itself refuses.
        e.go(&mut m, Path::of(b"Documents")).unwrap();
        let b = names(&e).iter().position(|n| n == "b.bin").unwrap();
        e.selection.click(b, false, false);
        assert_eq!(
            e.move_selection_to(&mut m, Path::of(b"Documents/Projects"))
                .unwrap(),
            1
        );
        assert!(m.get("Documents/Projects/b.bin").is_some());
        let p = names(&e).iter().position(|n| n == "Projects").unwrap();
        e.selection.click(p, false, false);
        assert_eq!(
            e.move_selection_to(&mut m, Path::of(b"Documents/Projects")),
            Err(wire::S_LOOP)
        );
    }

    #[test]
    fn type_ahead_change_detection_and_paths() {
        let mut m = home();
        let mut e = Explorer::new();
        e.go(&mut m, Path::of(b"Documents")).unwrap();
        e.type_ahead(b'N', 10);
        assert_eq!(e.at(e.selection.first().unwrap()).name(), b"notes.txt");
        e.type_ahead(b'p', 2_000_000);
        assert_eq!(e.at(e.selection.first().unwrap()).name(), b"Projects");
        assert!(!e.changed(&mut m));
        m.create(b"Documents/new.txt").unwrap();
        assert!(e.changed(&mut m));
        assert!(names(&e).contains(&"new.txt".into()));
        assert!(Path::of(b"a/b").starts_with(&Path::of(b"a")));
        assert!(!Path::of(b"ab").starts_with(&Path::of(b"a")));
        assert!(Path::ROOT.child(b"..").is_err() && Path::ROOT.child(b"a/b").is_err());
        assert_eq!(split_ext(b".bashrc"), (&b".bashrc"[..], &b""[..]));
        assert_eq!(kind_of(b"x.TXT", false), Kind::Text);
    }
}
