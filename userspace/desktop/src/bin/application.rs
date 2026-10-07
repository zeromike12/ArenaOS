//! Ordinary multicall ELF. Startup metadata selects a model; capabilities
//! independently determine which operations the broker accepts.
#![no_std]
#![no_main]
#![allow(clippy::deref_addrof)]
use arena_desktop::{
    app_client as service,
    apps::{
        self,
        explorer::{CapStore, Path as XPath},
        explorer_ctl::{Controller, Effect},
        layout::{self, Layout},
        model::{Editor, Line, Terminal},
        scene, view,
    },
    client::{self, Client, Transient},
    files::{self, Entry, Files},
    filesd_wire as fw,
    model::{Event, PopupKind},
    service_wire::Frame,
};
use arena_gfxkit::Canvas;
use arena_startup_abi::{
    manifest::FLAG_MULTI_INSTANCE,
    startup::{
        CAP_KIND_BADGED_ENDPOINT, CAP_KIND_MEMORY_POOL, CAP_KIND_NOTIFICATION,
        CAP_KIND_SHARED_REGION, CapabilityDescriptor, CapabilityRole, RIGHT_COPY, RIGHT_DESTROY,
        RIGHT_READ, RIGHT_WRITE, StartupView,
    },
};
use arena_ui::metrics as m;
#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    client::exit(99)
}
struct App {
    editor: Editor,
    terminal: Terminal,
    line: Line,
    names: [[u8; 32]; 32],
    count: usize,
    selected: usize,
    top: usize,
    preview: [u8; 4096],
    preview_len: usize,
    dialog: u8,
    status: &'static str,
    buttons: u8,
    kind: u8,
    display: (u16, u16),
    counts: [u64; 9],
    processes: [(u64, u64); 32],
    process_count: usize,
    next_sample: u64,
    closing: bool,
    gallery_theme: u8,
    /// Surface size the application is laid out for (Phase 11.3).
    size: (u16, u16),
    /// A size the window policy configured and the next frame adopts.
    pending: Option<(u16, u16)>,
    /// The open context menu (a real transient surface), if any.
    menu: Option<Menu>,
    /// filesd session (ADR-0077); None = no file capability or offline.
    afs: Option<Files>,
    /// /Users/user capability (terminal, Files) or CAP_NONE.
    home: u64,
    /// Terminal working directory, relative to home.
    cwd: Path,
    /// Files: the explorer (Phase 11.8) over the home capability.
    files: Controller,
    /// Files: the record watching the folder shown (ADR-0079) and its path.
    watch_cap: u64,
    watch_path: Option<XPath>,
    /// Editor: the document capability (granted at launch or by the
    /// chooser) or CAP_NONE.
    doc: u64,
}
/// A path relative to the home capability, normalized lexically (no `.`,
/// no `..`, no empty components, no leading `/`). Presentation only: the
/// authority is the capability it is walked from.
#[derive(Clone, Copy)]
struct Path {
    b: [u8; 256],
    n: usize,
}
impl Path {
    const EMPTY: Path = Path { b: [0; 256], n: 0 };
    fn bytes(&self) -> &[u8] {
        &self.b[..self.n]
    }
    /// `rel` resolved against `self` (a leading `/` starts from home).
    fn join(&self, rel: &[u8]) -> Result<Path, i64> {
        let mut out = if rel.first() == Some(&b'/') {
            Path::EMPTY
        } else {
            *self
        };
        for (_, part) in files::components(rel) {
            match part {
                b"." => {}
                b".." => {
                    out.n = out.b[..out.n].iter().rposition(|b| *b == b'/').unwrap_or(0);
                }
                _ => {
                    let extra = part.len() + usize::from(out.n != 0);
                    if out.n + extra > out.b.len() {
                        return Err(fserr(fw::S_INVAL));
                    }
                    if out.n != 0 {
                        out.b[out.n] = b'/';
                        out.n += 1;
                    }
                    out.b[out.n..out.n + part.len()].copy_from_slice(part);
                    out.n += part.len();
                }
            }
        }
        Ok(out)
    }
}
fn fserr(s: files::Status) -> i64 {
    -3000 - s as i64
}
/// Context menu state: its transient surface, hovered item and whether it
/// must be repainted and republished.
struct Menu {
    surface: Transient,
    hover: Option<usize>,
    buttons: u8,
    dirty: bool,
}
/// Context menu entries per application; each runs the same operation as
/// the equivalent toolbar control or command.
fn menu_items(kind: u8) -> &'static [&'static str] {
    match kind {
        apps::TERMINAL => &["Clear", "Help", "List Files"],
        apps::EDITOR => &["New", "Open...", "Save", "Save As...", "Open Read-Only..."],
        apps::SETTINGS => &["Toggle Appearance", "Toggle Motion"],
        apps::MONITOR => &["Sample Now"],
        _ => &["Toggle Theme"],
    }
}
static mut APP: App = App {
    editor: Editor::new(),
    terminal: Terminal::new(),
    line: Line::new(),
    names: [[0; 32]; 32],
    count: 0,
    selected: 0,
    top: 0,
    preview: [0; 4096],
    preview_len: 0,
    dialog: 0,
    status: "READY",
    buttons: 0,
    kind: 5,
    display: (0, 0),
    counts: [0; 9],
    processes: [(0, 0); 32],
    process_count: 0,
    next_sample: 0,
    closing: false,
    gallery_theme: 2,
    size: (m::WINDOW_WIDTH as u16, m::WINDOW_HEIGHT as u16),
    pending: None,
    menu: None,
    afs: None,
    home: CAP_NONE,
    cwd: Path::EMPTY,
    files: Controller::new(),
    watch_cap: CAP_NONE,
    watch_path: None,
    doc: CAP_NONE,
};
const CAP_NONE: u64 = u64::MAX;
/// Scratch for the dirty bands' previous pixels (Phase 11 latency): up to
/// 64K pixels a frame are compared; larger repaints publish their bands.
static mut BEFORE: [u32; 65536] = [0; 65536];
fn describe(slot: u64) -> Option<[u64; 3]> {
    let mut d = [0u64; 3];
    (unsafe { arena_desktop::app_client::describe_raw(slot, &mut d) } == 0).then_some(d)
}
fn length(n: &[u8; 32]) -> usize {
    n.iter().position(|b| *b == 0).unwrap_or(32)
}
/// Check the trusted launcher metadata against this static multicall Image's
/// identity, actual linked entry, and exact built-in cap profile. None of
/// these descriptive fields grant authority; the held caps do.
fn startup_values(view: &StartupView<'_>) -> Result<(u8, bool, bool, [u8; 32]), i64> {
    let kind = apps::APPLICATION_IDS
        .iter()
        .position(|id| id == view.application_id())
        .ok_or(-2)? as u8;
    let id = &apps::APPLICATION_IDS[kind as usize];
    let id_len = length(id);
    let actual_entry = _start as *const () as usize as u64;
    if view.flags() != FLAG_MULTI_INSTANCE
        || view.argument_count() == 0
        || view.argument_count() > 2
        || view.argument(0) != Some(&id[..id_len])
        || view.environment_count() != 2
        || view.capability_count() < 3
        || view.capability_count() > 4
        || view.cwd_descriptor().is_some()
        || view.stdin_descriptor().is_some()
        || view.stdout_descriptor().is_some()
        || view.stderr_descriptor().is_some()
        || view.page_size() != 4096
        || view.entry() != actual_entry
        || view.entry() != apps::APPLICATION_ENTRY
        || view.load_base() != apps::APPLICATION_LOAD_BASE
        || view.instance_slot() as usize >= apps::STARTUP_INSTANCE_SLOTS
        || view.instance_generation() == 0
        || view.clock_us() > service::now()
    {
        return Err(-2);
    }
    let theme_env = view.environment(0).ok_or(-2)?;
    let dark = if theme_env == b"ARENA_THEME=dark" {
        true
    } else if theme_env == b"ARENA_THEME=light" {
        false
    } else {
        return Err(-2);
    };
    let motion_env = view.environment(1).ok_or(-2)?;
    let motion = if motion_env == b"ARENA_MOTION=1" {
        true
    } else if motion_env == b"ARENA_MOTION=0" {
        false
    } else {
        return Err(-2);
    };
    let mut path = [0u8; 32];
    if view.argument_count() == 2 {
        let argument = view.argument(1).ok_or(-2)?;
        if argument.len() > path.len() {
            return Err(-2);
        }
        path[..argument.len()].copy_from_slice(argument);
    }
    let expected = [
        CapabilityDescriptor {
            slot: 1,
            role: CapabilityRole::Other,
            kind: CAP_KIND_BADGED_ENDPOINT,
            rights: RIGHT_WRITE | RIGHT_COPY | RIGHT_DESTROY,
        },
        CapabilityDescriptor {
            slot: 2,
            role: CapabilityRole::Other,
            kind: CAP_KIND_SHARED_REGION,
            rights: RIGHT_READ | RIGHT_WRITE | RIGHT_COPY,
        },
        CapabilityDescriptor {
            slot: 3,
            role: CapabilityRole::Other,
            kind: CAP_KIND_NOTIFICATION,
            rights: if kind == apps::FILES {
                RIGHT_READ | RIGHT_WRITE | RIGHT_COPY
            } else {
                RIGHT_READ | RIGHT_WRITE
            },
        },
    ];
    if expected
        .iter()
        .enumerate()
        .any(|(index, cap)| view.capability(index) != Some(*cap))
    {
        return Err(-2);
    }
    match (kind, view.capability_count(), view.capability(3)) {
        (apps::MONITOR, 4, Some(cap))
            if cap.slot == 4
                && cap.role == CapabilityRole::Other
                && cap.kind == CAP_KIND_MEMORY_POOL
                && cap.rights == RIGHT_READ => {}
        (apps::TERMINAL | apps::FILES | apps::EDITOR, 4, Some(cap))
            if cap.slot == 4
                && cap.role == CapabilityRole::Other
                && cap.kind == CAP_KIND_BADGED_ENDPOINT
                && cap.rights == (RIGHT_WRITE | RIGHT_COPY) => {}
        (apps::TERMINAL | apps::FILES | apps::EDITOR, 3, None) => {}
        (apps::SETTINGS | apps::GALLERY, 3, None) => {}
        _ => return Err(-2),
    }
    Ok((kind, dark, motion, path))
}
fn error(rc: i64) -> &'static str {
    match rc {
        -3099..=-3000 => files::describe_status((-rc - 3000) as u64),
        -2001 => "REFUSED: DOCUMENT FULL (4096 BYTES)",
        -1001 => "FILE NOT FOUND",
        -1002 => "FILE ALREADY EXISTS",
        -1004 => "REFUSED: FILE TABLE FULL",
        -1006 => "REFUSED: DISK SPACE",
        -1009 => "REFUSED: FILE SIZE OR RANGE",
        -1011 => "REFUSED: FILE IS OPEN / BUSY",
        -1005 | -1008 => "FILESYSTEM ERROR",
        -4 => "REFUSED: APPLICATION CAPACITY",
        -5 => "SERVICE UNAVAILABLE",
        -6 => "SERVICE CALL CANCELLED",
        _ => "REFUSED: CHECK NAME, RIGHTS AND FILE FORMAT",
    }
}
fn call(f: Frame) -> Result<(u64, Frame), i64> {
    service::exchange(f, service::FUNCTION)
}
fn launch(kind: u8, path: [u8; 32]) -> Result<(), i64> {
    call(Frame::Launch { kind, path }).map(|_| ())
}
impl App {
    // ---- AFS2 through capabilities (ADR-0077) -----------------------------
    fn fs(&self) -> Result<Files, i64> {
        self.afs.ok_or(fserr(fw::S_OFFLINE))
    }
    /// Register this client's filesd page for the lineage of `cap`.
    fn start_files(&mut self, client: &Client, cap: u64) {
        let (page, base) = client.file_page();
        match Files::session(cap, client.backing_slot(), base, page) {
            Ok(f) => self.afs = Some(f),
            Err(e) => self.status = files::describe_status(e),
        }
    }
    /// `rel` (relative to home) opened with `rights`: (slot, type).
    fn walk(&self, rel: &Path, rights: u8) -> Result<(u64, u8), i64> {
        if self.home == CAP_NONE {
            return Err(fserr(fw::S_DENIED));
        }
        self.fs()?
            .walk(self.home, rel.bytes(), rights)
            .map_err(fserr)
    }
    /// The directory holding `rel` (opened with `rights`) and the final
    /// name. The caller releases the directory capability.
    fn parent<'a>(&self, rel: &'a Path, rights: u8) -> Result<(u64, &'a [u8]), i64> {
        let (dir, name) = files::split_last(rel.bytes());
        if name.is_empty() {
            return Err(fserr(fw::S_INVAL));
        }
        let mut d = Path::EMPTY;
        d.b[..dir.len()].copy_from_slice(dir);
        d.n = dir.len();
        Ok((self.walk(&d, rights)?.0, name))
    }
    fn release(&self, cap: u64) {
        if let Ok(f) = self.fs() {
            f.release(cap);
        }
    }

    // ---- Files: the explorer over /Users/user (Phase 11.8) ---------------
    fn store(&self) -> Result<CapStore, i64> {
        if self.home == CAP_NONE {
            return Err(fserr(fw::S_DENIED));
        }
        Ok(CapStore {
            fs: self.fs()?,
            root: self.home,
        })
    }
    /// Keep a watch on the folder shown (ADR-0079): a record for exactly
    /// that folder, opened from home, with this client's clock lent to
    /// filesd. Navigation moves the watch; releasing the record ends it.
    fn files_rewatch(&mut self) {
        let Ok(fs) = self.fs() else {
            return;
        };
        let path = self.files.ex.path;
        if self.watch_path == Some(path) {
            return;
        }
        if self.watch_cap != CAP_NONE {
            fs.release(self.watch_cap);
            self.watch_cap = CAP_NONE;
        }
        self.watch_path = Some(path);
        if let Ok((cap, _)) = fs.walk(self.home, path.bytes(), fw::R_LIST) {
            if fs.watch(cap, service::CLOCK, 1).is_ok() {
                self.watch_cap = cap;
            } else {
                fs.release(cap);
            }
        }
    }
    /// The watch badge arrived: ask filesd through the watched record, and
    /// re-list only if the folder really changed.
    fn files_watched(&mut self) -> bool {
        let (Ok(fs), Ok(mut store)) = (self.fs(), self.store()) else {
            return false;
        };
        if self.watch_cap == CAP_NONE {
            return false;
        }
        match fs.watched(self.watch_cap) {
            Ok((0, false)) => false,
            Ok((_, false)) => self.files.poll(&mut store),
            _ => {
                // The folder is gone: show home, which is never removed.
                self.files.start(&mut store, XPath::ROOT);
                self.files.ex.status = "THE FOLDER WAS REMOVED";
                true
            }
        }
    }
    /// Carry out what an explorer interaction asks of the application.
    fn files_effect(&mut self, e: Effect, client: &Client) -> Result<(), i64> {
        match e {
            Effect::None => Ok(()),
            Effect::Menu { x, y } => self.open_menu(x, y, client),
            Effect::OpenInEditor(p) => {
                // A capability for exactly this file, offered to the broker
                // and re-granted in the new Editor's lineage; the name is
                // only its title.
                let fs = self.fs()?;
                let (f, _) = fs
                    .walk(self.home, p.bytes(), fw::R_READ | fw::R_WRITE)
                    .map_err(fserr)?;
                service::offer(f)?;
                let mut title = [0u8; 32];
                for (t, b) in title.iter_mut().zip(p.name()) {
                    *t = if b.is_ascii_graphic() || *b == b' ' {
                        *b
                    } else {
                        b'?'
                    };
                }
                launch(apps::EDITOR, title)?;
                self.files.ex.status = "OPENED IN EDITOR";
                Ok(())
            }
        }
    }

    // ---- Editor: one document capability ---------------------------------
    fn open_document(&mut self, title: [u8; 32]) -> Result<(), i64> {
        self.load_from(self.doc, title)
    }
    /// Load document `cap` into the editor; on refusal nothing changes.
    fn load_from(&mut self, cap: u64, title: [u8; 32]) -> Result<(), i64> {
        let fs = self.fs()?;
        let data = fs.read_all(cap, &mut self.preview).map_err(fserr)?;
        self.editor
            .load(data, view::string(&title[..length(&title)]))
            .map_err(|_| -2)?;
        self.top = 0;
        self.status = "OPENED";
        Ok(())
    }
    fn save(&mut self, _client: &Client) -> Result<(), i64> {
        if self.doc == CAP_NONE {
            return self.choose(true);
        }
        self.fs()?
            .write_all(self.doc, &self.editor.data[..self.editor.len])
            .map_err(fserr)?;
        self.editor.dirty = false;
        self.status = "SAVED: TRANSACTIONS COMMITTED";
        if self.closing {
            client::exit(42)
        }
        Ok(())
    }
    /// Ask the broker's trusted chooser for a document (Open or Save As).
    /// The answer arrives later as an event; nothing is granted here.
    fn choose(&mut self, save: bool) -> Result<(), i64> {
        self.choose_mode(save, false)
    }
    fn choose_mode(&mut self, save: bool, read_only: bool) -> Result<(), i64> {
        let suggestion = if save && self.editor.path[0] != 0 {
            self.editor.path
        } else {
            let mut n = [0u8; 32];
            n[..12].copy_from_slice(b"Untitled.txt");
            n
        };
        call(Frame::Choose {
            save,
            read_only,
            name: suggestion,
        })?;
        self.status = if save {
            "CHOOSE WHERE TO SAVE"
        } else {
            "CHOOSE A DOCUMENT"
        };
        Ok(())
    }
    /// The chooser finished: take the granted capability, if any.
    fn chosen(&mut self, client: &Client) -> Result<(), i64> {
        let Some(service::Grant {
            cap,
            title,
            save,
            read_only,
        }) = service::take_grant()?
        else {
            self.status = "CANCELLED";
            return Ok(());
        };
        if self.afs.is_none() {
            self.start_files(client, cap);
        }
        if !save {
            // The editor switches documents only once the new one loaded:
            // a refused file (binary, too large) leaves the open document
            // and its capability exactly as they were.
            if let Err(e) = self.load_from(cap, title) {
                self.release(cap);
                return Err(e);
            }
        }
        if self.doc != CAP_NONE {
            self.release(self.doc);
        }
        self.doc = cap;
        if save {
            self.editor.path = title;
            self.save(client)
        } else {
            if read_only {
                // The capability itself carries no write right: a save is
                // refused by filesd, not by this application.
                self.status = "OPENED READ-ONLY";
            }
            Ok(())
        }
    }

    /// A new, unsaved document (the next save asks where, via the chooser).
    fn new_document(&mut self) -> Result<(), i64> {
        if self.editor.dirty {
            self.status = "SAVE FIRST OR OPEN A NEW EDITOR";
            return Ok(());
        }
        self.editor.load(b"", "").map_err(|_| -2)?;
        if self.doc != CAP_NONE {
            self.release(self.doc);
            self.doc = CAP_NONE;
        }
        self.top = 0;
        Ok(())
    }
    fn accept(&mut self, _client: &Client) -> Result<(), i64> {
        // No application dialog takes typed input any more (names are
        // chosen in the trusted chooser or edited inline in Files).
        self.dialog = 0;
        Ok(())
    }

    // ---- Terminal ------------------------------------------------------------
    fn command(&mut self, client: &Client) -> Result<(), i64> {
        let (bytes, n) = self.terminal.consume();
        let text = &bytes[..n];
        self.terminal.write(text);
        let split = text.iter().position(|b| *b == b' ').unwrap_or(n);
        let cmd = &text[..split];
        let args = if split < n { &text[split + 1..] } else { b"" };
        let (first, rest) = match args.iter().position(|b| *b == b' ') {
            Some(i) => (&args[..i], &args[i + 1..]),
            None => (args, &b""[..]),
        };
        match cmd {
            b"" => {}
            b"help" => self.terminal.write(b"help echo ls cd pwd cat put mkdir rm rmdir mv ps launch clear\nPaths are inside your home folder; / is home.\nlaunch term|files|edit|settings|monitor|gallery"),
            b"echo" => self.terminal.write(args),
            b"pwd" => {
                let mut b = [0u8; 64];
                let mut k = 0;
                append(&mut b, &mut k, b"~/");
                append(&mut b, &mut k, self.cwd.bytes());
                self.terminal.write(&b[..k]);
            }
            b"cd" => {
                let to = self.cwd.join(args)?;
                let (cap, typ) = self.walk(&to, fw::R_LIST)?;
                self.release(cap);
                if typ != 2 {
                    return Err(fserr(fw::S_NOTDIR));
                }
                self.cwd = to;
            }
            b"ls" => {
                let at = self.cwd.join(args)?;
                let (cap, typ) = self.walk(&at, fw::R_LIST | fw::R_READ)?;
                let r = if typ == 2 { self.list_into_terminal(cap) } else { Ok(()) };
                if typ != 2 {
                    let name = files::split_last(at.bytes()).1;
                    self.terminal.write(name);
                }
                self.release(cap);
                r?;
            }
            b"cat" => {
                let at = self.cwd.join(args)?;
                let (cap, _) = self.walk(&at, fw::R_READ)?;
                let r = self.fs()?.read_all(cap, &mut self.preview).map(|d| d.len());
                self.release(cap);
                let len = r.map_err(fserr)?;
                let data = &self.preview[..len];
                if !data.iter().all(|b| b.is_ascii_graphic() || matches!(b, b' ' | b'\n' | b'\t')) {
                    return Err(fserr(fw::S_INVAL));
                }
                let mut copy = [0u8; 4096];
                copy[..len].copy_from_slice(data);
                self.terminal.write(&copy[..len]);
            }
            b"put" => {
                let at = self.cwd.join(first)?;
                let (dir, name) = self.parent(&at, fw::R_CREATE | fw::R_LIST | fw::R_WRITE)?;
                let fs = self.fs()?;
                let r = match fs.create(dir, name) {
                    Ok(()) | Err(fw::S_EXIST) => fs
                        .open(dir, Some(name), fw::R_WRITE)
                        .and_then(|(f, _)| {
                            let w = fs.write_all(f, rest);
                            fs.release(f);
                            w
                        }),
                    Err(e) => Err(e),
                };
                fs.release(dir);
                r.map_err(fserr)?;
                self.terminal.write(b"Saved");
            }
            b"mkdir" | b"rm" | b"rmdir" => {
                let at = self.cwd.join(args)?;
                let rights = if cmd == b"mkdir" { fw::R_CREATE } else { fw::R_DELETE };
                let (dir, name) = self.parent(&at, rights)?;
                let fs = self.fs()?;
                let r = match cmd {
                    b"mkdir" => fs.mkdir(dir, name),
                    b"rm" => fs.unlink(dir, name),
                    _ => fs.rmdir(dir, name),
                };
                fs.release(dir);
                r.map_err(fserr)?;
                self.terminal.write(b"Done");
            }
            b"mv" => {
                let from = self.cwd.join(first)?;
                let mut to = self.cwd.join(rest)?;
                // Moving onto an existing folder moves into it.
                if let Ok((cap, typ)) = self.walk(&to, fw::R_LIST) {
                    self.release(cap);
                    if typ == 2 {
                        to = to.join(files::split_last(from.bytes()).1)?;
                    }
                }
                let (src, a) = self.parent(&from, fw::R_RENAME)?;
                let dst = match self.parent(&to, fw::R_CREATE) {
                    Ok(d) => d,
                    Err(e) => {
                        self.release(src);
                        return Err(e);
                    }
                };
                let fs = self.fs()?;
                let r = fs.rename(src, a, Some(dst.0), dst.1);
                fs.release(src);
                fs.release(dst.0);
                r.map_err(fserr)?;
                self.terminal.write(b"Moved");
            }
            b"ps" => {self.process_count=service::processes(&mut self.processes)?; for i in 0..self.process_count {let mut b=[0;64]; let mut n=0; append(&mut b,&mut n,b"PID ");number(&mut b,&mut n,self.processes[i].0);append(&mut b,&mut n,b" THREADS ");number(&mut b,&mut n,self.processes[i].1);self.terminal.write(&b[..n]);}},
            b"launch" => {let kind=match args {b"term"=>0,b"files"=>1,b"edit"=>2,b"settings"=>3,b"monitor"=>4,b"gallery"=>5,_=>return Err(-2)};launch(kind,[0;32])?; self.terminal.write(b"Real application started");},
            b"clear" => {self.terminal.count=0;},
            _ => self.terminal.write(b"Unknown command; use help"),
        }
        let _ = client;
        Ok(())
    }
    /// Every entry of directory `cap`, one line each (folders end in `/`).
    fn list_into_terminal(&mut self, cap: u64) -> Result<(), i64> {
        let mut batch = [Entry::EMPTY; 8];
        let mut after = [0u8; files::NAME_MAX];
        let mut after_len = 0usize;
        loop {
            let (n, more) = self
                .fs()?
                .list(cap, &after[..after_len], &mut batch)
                .map_err(fserr)?;
            for e in &batch[..n] {
                let mut b = [0u8; 64];
                let mut k = 0;
                let name = e.name();
                append(&mut b, &mut k, &name[..name.len().min(40)]);
                if name.len() > 40 {
                    append(&mut b, &mut k, b"~");
                }
                if e.is_dir() {
                    append(&mut b, &mut k, b"/");
                } else {
                    append(&mut b, &mut k, b"  ");
                    number(&mut b, &mut k, e.size);
                    append(&mut b, &mut k, b" B");
                }
                self.terminal.write(&b[..k]);
            }
            if n == 0 || !more {
                return Ok(());
            }
            let last = batch[n - 1];
            after[..last.name().len()].copy_from_slice(last.name());
            after_len = last.name().len();
        }
    }
    fn layout(&self) -> Layout {
        Layout::new(self.size.0, self.size.1)
    }
    fn key(&mut self, key: u16, client: &Client) -> Result<(), i64> {
        let l = self.layout();
        if self.menu.is_some() {
            return self.menu_key(key, client);
        }
        if self.dialog != 0 {
            if self.dialog == 4 {
                if key == 27 {
                    self.dialog = 0;
                    self.closing = false;
                    client.cancel_close()?;
                }
                return Ok(());
            }
            if key == 27 {
                self.dialog = 0;
                if self.closing {
                    self.closing = false;
                    client.cancel_close()?;
                }
            } else if self.line.key(key) {
                self.accept(client)?;
            }
            return Ok(());
        }
        match self.kind {
            apps::TERMINAL => {
                if key == 258 {
                    self.top =
                        (self.top + 1).min(self.terminal.count.saturating_sub(l.TERMINAL_ROWS));
                    return Ok(());
                }
                if key == 259 {
                    self.top = self.top.saturating_sub(1);
                    return Ok(());
                }
                self.top = 0;
                if self.terminal.key(key) {
                    self.command(client)?;
                }
            }
            apps::EDITOR => {
                match key {
                    8 => self.editor.backspace(),
                    13 => self.editor.insert(b'\n').map_err(|_| -2001)?,
                    256 => self.editor.left(),
                    257 => self.editor.right(),
                    258 => self.editor.vertical(false),
                    259 => self.editor.vertical(true),
                    260 => self.editor.home(),
                    261 => self.editor.end(),
                    262 => self.editor.delete(),
                    9 => self.editor.insert(b'\t').map_err(|_| -2001)?,
                    32..=126 => self.editor.insert(key as u8).map_err(|_| -2001)?,
                    _ => {}
                }
                let row = l.visual_row(&self.editor);
                if row < self.top {
                    self.top = row;
                }
                if row >= self.top + l.EDIT_ROWS {
                    self.top = row.saturating_sub(l.EDIT_ROWS - 1);
                }
                self.status = if self.editor.dirty {
                    "MODIFIED / SAVE COMMITS COMPLETE FILE"
                } else {
                    "READY"
                };
            }
            apps::FILES => {
                let mut store = self.store()?;
                let e = self.files.key(&mut store, key, false, service::now());
                return self.files_effect(e, client);
            }
            apps::GALLERY if key == 116 => {
                self.gallery_theme = if self.gallery_theme == 2 {
                    u8::from(client.appearance.get() & 1 == 0)
                } else {
                    1 - self.gallery_theme
                };
            }
            apps::MONITOR => {
                if key == 258 {
                    self.top = self.top.saturating_sub(1)
                } else if key == 259 {
                    self.top = (self.top + 1).min(self.process_count.saturating_sub(l.MONITOR_ROWS))
                }
            }
            _ => {}
        }
        Ok(())
    }
    fn pointer(&mut self, x: i32, y: i32, buttons: u8, client: &Client) -> Result<(), i64> {
        let l = self.layout();
        if self.kind == apps::FILES && self.dialog == 0 {
            self.buttons = buttons;
            let mut store = self.store()?;
            let e = self
                .files
                .pointer(&mut store, x, y, buttons, service::now());
            return self.files_effect(e, client);
        }
        let pressed = buttons & 1 != 0 && self.buttons & 1 == 0;
        let context = buttons & 2 != 0 && self.buttons & 2 == 0;
        self.buttons = buttons;
        if context && self.dialog == 0 && y >= m::TITLE_HEIGHT {
            return self.open_menu(x, y, client);
        }
        if !pressed {
            return Ok(());
        }
        if self.dialog != 0 {
            if self.dialog == 4 {
                if layout::hit(l.NAME_FIELD, x, y) {
                    self.dialog = 0;
                    self.save(client)?;
                } else if layout::hit(l.PRIMARY, x, y) {
                    client::exit(42)
                } else if layout::hit(l.SECONDARY, x, y) {
                    self.dialog = 0;
                    self.closing = false;
                    client.cancel_close()?;
                }
                return Ok(());
            }
            if layout::hit(l.PRIMARY, x, y) {
                self.accept(client)?;
            } else if layout::hit(l.SECONDARY, x, y) {
                self.dialog = 0;
                if self.closing {
                    self.closing = false;
                    client.cancel_close()?;
                }
            }
            return Ok(());
        }
        match self.kind {
            apps::EDITOR => {
                if layout::hit(l.NEW, x, y) {
                    self.new_document()?;
                } else if layout::hit(l.SAVE, x, y) {
                    self.save(client)?;
                } else if layout::hit(l.SAVE_AS, x, y) {
                    self.choose(true)?;
                } else if layout::hit(l.OPEN, x, y) {
                    if self.editor.dirty {
                        self.status = "SAVE FIRST OR OPEN A NEW EDITOR";
                    } else {
                        self.choose(false)?;
                    }
                } else if layout::hit(l.EDIT_TEXT, x, y) {
                    self.editor.cursor = l.visual_cursor(
                        &self.editor,
                        self.top + ((y - l.EDIT_TEXT.y - 6).max(0) / m::LINE_HEIGHT) as usize,
                        ((x - l.EDIT_TEXT.x - 6).max(0) / m::FONT_ADVANCE) as usize,
                    );
                }
            }
            apps::SETTINGS if layout::hit(l.APPEARANCE, x, y) => {
                let dark = client.appearance.get() & 1 == 0;
                call(Frame::Configure {
                    theme: u8::from(dark),
                    motion: client.appearance.get() & 2 != 0,
                })?;
                self.status = "APPEARANCE COMMITTED TO AFS1";
            }
            apps::SETTINGS if layout::hit(l.MOTION, x, y) => {
                call(Frame::Configure {
                    theme: client.appearance.get() & 1,
                    motion: client.appearance.get() & 2 == 0,
                })?;
                self.status = "MOTION PREFERENCE COMMITTED TO AFS1";
            }
            _ => {}
        }
        Ok(())
    }
    /// The context menu's labels (Files: chosen by what was clicked).
    fn menu_labels(&self, out: &mut [&'static str; 8]) -> usize {
        if self.kind == apps::FILES {
            return self.files.menu_items(out);
        }
        let items = menu_items(self.kind);
        for (o, i) in out.iter_mut().zip(items) {
            *o = i;
        }
        items.len().min(8)
    }
    /// Open the context menu at window-local (`x`, `y`).
    fn open_menu(&mut self, x: i32, y: i32, client: &Client) -> Result<(), i64> {
        let mut labels = [""; 8];
        let n = self.menu_labels(&mut labels);
        let items = &labels[..n];
        let surface = client.open_transient(
            PopupKind::Menu,
            x,
            y,
            view::MENU_WIDTH,
            view::menu_height(items.len()),
        )?;
        // Opening replaced any previous menu surface (its handle is stale).
        self.menu = Some(Menu {
            surface,
            hover: None,
            buttons: 0,
            dirty: true,
        });
        Ok(())
    }
    fn close_menu(&mut self, client: &Client) -> Result<(), i64> {
        if let Some(menu) = self.menu.take() {
            client.close_transient(&menu.surface)?;
        }
        Ok(())
    }
    fn menu_pointer(&mut self, x: i32, y: i32, buttons: u8, client: &Client) -> Result<(), i64> {
        let n = self.menu_labels(&mut [""; 8]);
        let Some(menu) = self.menu.as_mut() else {
            return Ok(());
        };
        let hover = view::menu_item(n, x, y);
        let pressed = buttons & 1 != 0 && menu.buttons & 1 == 0;
        menu.buttons = buttons;
        if hover != menu.hover {
            menu.hover = hover;
            menu.dirty = true;
        }
        if pressed && let Some(item) = hover {
            self.close_menu(client)?;
            return self.act(item, client);
        }
        Ok(())
    }
    fn menu_key(&mut self, key: u16, client: &Client) -> Result<(), i64> {
        let n = self.menu_labels(&mut [""; 8]);
        let Some(menu) = self.menu.as_mut() else {
            return Ok(());
        };
        match key {
            27 => return self.close_menu(client),
            13 => {
                if let Some(item) = menu.hover {
                    self.close_menu(client)?;
                    return self.act(item, client);
                }
            }
            258 => {
                menu.hover = Some(menu.hover.map_or(n - 1, |h| (h + n - 1) % n));
                menu.dirty = true;
            }
            259 => {
                menu.hover = Some(menu.hover.map_or(0, |h| (h + 1) % n));
                menu.dirty = true;
            }
            _ => {}
        }
        Ok(())
    }
    /// Run context menu entry `item` (see `menu_items`).
    fn act(&mut self, item: usize, client: &Client) -> Result<(), i64> {
        match (self.kind, item) {
            (apps::TERMINAL, 0) => {
                self.terminal.count = 0;
                self.top = 0;
            }
            (apps::TERMINAL, 1 | 2) => {
                let command: &[u8] = if item == 1 { b"help" } else { b"ls" };
                for b in command {
                    self.terminal.key(u16::from(*b));
                }
                self.top = 0;
                self.command(client)?;
            }
            (apps::EDITOR, 0) => self.new_document()?,
            (apps::EDITOR, 1) => {
                if self.editor.dirty {
                    self.status = "SAVE FIRST OR OPEN A NEW EDITOR";
                } else {
                    self.choose(false)?
                }
            }
            (apps::EDITOR, 2) => self.save(client)?,
            (apps::EDITOR, 3) => self.choose(true)?,
            (apps::EDITOR, 4) => {
                if self.editor.dirty {
                    self.status = "SAVE FIRST OR OPEN A NEW EDITOR";
                } else {
                    self.choose_mode(false, true)?
                }
            }
            (apps::FILES, _) => {
                let mut store = self.store()?;
                let e = self.files.act(&mut store, item);
                self.files_effect(e, client)?;
            }
            (apps::SETTINGS, 0 | 1) => {
                let a = client.appearance.get();
                let (dark, motion) = if item == 0 {
                    (a & 1 == 0, a & 2 != 0)
                } else {
                    (a & 1 != 0, a & 2 == 0)
                };
                call(Frame::Configure {
                    theme: u8::from(dark),
                    motion,
                })?;
                self.status = "PREFERENCE COMMITTED TO AFS1";
            }
            (apps::MONITOR, _) => self.sample()?,
            (apps::GALLERY, _) => {
                self.gallery_theme = if self.gallery_theme == 2 {
                    u8::from(client.appearance.get() & 1 == 0)
                } else {
                    1 - self.gallery_theme
                };
            }
            _ => {}
        }
        Ok(())
    }
    /// Keyboard shortcuts: the same operations as the context menu.
    fn chord(&mut self, code: u16, mods: u8, client: &Client) -> Result<(), i64> {
        use arena_desktop::model::{MOD_ALT, MOD_CTRL, MOD_SHIFT, MOD_SUPER};
        if self.kind == apps::FILES && self.dialog == 0 && self.menu.is_none() {
            let mut store = self.store()?;
            let e = self.files.chord(&mut store, code, mods, service::now());
            return self.files_effect(e, client);
        }
        // Shifted navigation keys arrive as chords (Phase 11.8); elsewhere
        // they act as the plain key.
        if mods & (MOD_CTRL | MOD_ALT | MOD_SUPER) == 0 {
            return self.key(code, client);
        }
        if mods & MOD_CTRL == 0 || self.dialog != 0 || self.menu.is_some() {
            return Ok(());
        }
        let shift = mods & MOD_SHIFT != 0;
        let item = match (self.kind, code as u8) {
            (apps::TERMINAL, b'l') => Some(0),
            (apps::EDITOR, b'n') => Some(0),
            (apps::EDITOR, b'O') if shift => Some(4),
            (apps::EDITOR, b'o') => Some(1),
            (apps::EDITOR, b's') => Some(2),
            (apps::EDITOR, b'S') if shift => Some(3),
            (apps::MONITOR, b'r') => Some(0),
            _ => None,
        };
        match item {
            Some(i) => self.act(i, client),
            None => Ok(()),
        }
    }
    /// Scroll wheel: `delta` notches away from the user scroll back.
    fn wheel(&mut self, delta: i8) {
        if self.kind == apps::FILES {
            self.files.wheel(delta);
            return;
        }
        let l = self.layout();
        let step = usize::from(delta.unsigned_abs()) * 3;
        let back = delta > 0;
        let (limit, reversed) = match self.kind {
            // Terminal `top` counts lines back from the newest.
            apps::TERMINAL => (self.terminal.count.saturating_sub(l.TERMINAL_ROWS), true),
            apps::EDITOR => (
                l.visual_row_count(&self.editor).saturating_sub(l.EDIT_ROWS),
                false,
            ),
            apps::MONITOR => (self.process_count.saturating_sub(l.MONITOR_ROWS), false),
            _ => return,
        };
        let up = back != reversed;
        self.top = if up {
            self.top.saturating_sub(step)
        } else {
            (self.top + step).min(limit)
        };
    }
    /// Keep scroll positions valid for the current layout (after a resize).
    fn fit(&mut self) {
        let l = self.layout();
        match self.kind {
            apps::TERMINAL => {
                self.top = self
                    .top
                    .min(self.terminal.count.saturating_sub(l.TERMINAL_ROWS));
            }
            apps::EDITOR => {
                let row = l.visual_row(&self.editor);
                if row < self.top {
                    self.top = row;
                }
                if row >= self.top + l.EDIT_ROWS {
                    self.top = row.saturating_sub(l.EDIT_ROWS - 1);
                }
            }
            apps::FILES => self.files.resize(self.size),
            apps::MONITOR => {
                self.top = self
                    .top
                    .min(self.process_count.saturating_sub(l.MONITOR_ROWS));
            }
            _ => {}
        }
    }
    fn sample(&mut self) -> Result<(), i64> {
        self.counts = service::observe()?;
        self.process_count = service::processes(&mut self.processes)?;
        self.next_sample = service::now() + 500_000;
        self.status = "LIVE COUNTERS / ARROW KEYS SCROLL PROCESSES";
        Ok(())
    }
    /// Everything this window paints, borrowed from the model (Phase 11.1
    /// keyed bands decide which parts are repainted and published).
    fn view(&self, appearance: u8) -> scene::View<'_> {
        let files = (self.kind == apps::FILES && self.afs.is_some()).then_some(&self.files);
        scene::View {
            kind: self.kind,
            appearance,
            status: if files.is_some() {
                self.files.status()
            } else {
                self.status
            },
            terminal: &self.terminal,
            editor: &self.editor,
            line: &self.line,
            top: self.top,
            dialog: self.dialog,
            names: &self.names[..self.count],
            selected: self.selected,
            preview: &self.preview[..self.preview_len],
            display: self.display,
            counts: &self.counts,
            processes: &self.processes[..self.process_count],
            gallery_theme: self.gallery_theme,
            size: self.size,
            files,
        }
    }
}
fn append(b: &mut [u8; 64], n: &mut usize, s: &[u8]) {
    let count = s.len().min(64 - *n);
    b[*n..*n + count].copy_from_slice(&s[..count]);
    *n += count;
}
fn number(b: &mut [u8; 64], n: &mut usize, mut v: u64) {
    let mut d = [0; 20];
    let mut len = 0;
    loop {
        d[len] = b'0' + (v % 10) as u8;
        len += 1;
        v /= 10;
        if v == 0 {
            break;
        }
    }
    for digit in d[..len].iter().rev() {
        append(b, n, core::slice::from_ref(digit));
    }
}
arena_desktop::entry!(main, 64 * 1024);
extern "C" fn main() -> ! {
    match arena_runtime::startup::run(|view| {
        application_main(view);
    }) {
        Ok(()) | Err(_) => client::exit(70),
    }
}
fn application_main(view: StartupView<'_>) -> ! {
    let (expected_kind, expected_dark, expected_motion, expected_path) =
        startup_values(&view).unwrap_or_else(|_| client::exit(70));
    let (kind, dark, motion, path) = service::startup().unwrap_or_else(|_| client::exit(70));
    if kind != expected_kind
        || dark != expected_dark
        || motion != expected_motion
        || path != expected_path
    {
        client::exit(72);
    }
    let app = unsafe { &mut *(&raw mut APP) };
    service::audit(kind, view.instance_generation()).unwrap_or_else(|_| client::exit(76));
    let mut client = Client::connect_v2(
        m::WINDOW_WIDTH,
        m::WINDOW_HEIGHT,
        apps::TITLES[kind as usize],
    )
    .unwrap_or_else(|_| client::exit(71));
    // Every built-in application re-lays out to any size from its minimum
    // to the work area; the Gallery specimen sheet keeps its fixed size.
    if kind != apps::GALLERY {
        let (min_w, min_h) = layout::min_size(kind);
        client
            .set_resizable(min_w, min_h)
            .unwrap_or_else(|_| client::exit(71));
    }
    client
        .appearance
        .set(u8::from(dark) | (u8::from(motion) << 1));
    app.kind = kind;
    // Child slot 4: a filesd capability (ADR-0077) for the terminal and
    // Files (/Users/user) or the Editor (the document it was opened with).
    let granted = describe(4).is_some_and(|d| d[0] == 12);
    if granted && matches!(kind, apps::TERMINAL | apps::FILES | apps::EDITOR) {
        app.start_files(&client, 4);
        if kind == apps::EDITOR {
            app.doc = 4;
        } else {
            app.home = 4;
        }
    }
    let init = match kind {
        apps::TERMINAL => {
            app.terminal.write(if app.afs.is_some() {
                b"ArenaOS ordinary command session\nType help. Your home folder is /Users/user."
                    as &[u8]
            } else {
                b"ArenaOS ordinary command session\nFile service offline: no file commands."
            });
            Ok(())
        }
        apps::FILES => {
            app.files.resize(app.size);
            // A start folder named at launch (the desktop opening one of
            // its folders), inside the granted home; else Documents.
            let n = path.iter().position(|b| *b == 0).unwrap_or(32);
            let start = if n == 0 {
                XPath::of(b"Documents")
            } else {
                XPath::of(&path[..n])
            };
            app.store().map(|mut store| {
                app.files.start(&mut store, start);
            })
        }
        apps::EDITOR if app.doc != CAP_NONE => app.open_document(path),
        apps::SETTINGS => match service::exchange(Frame::Display, client::V2_BACKING) {
            Ok((v, _)) => {
                app.display = (v as u16, (v >> 32) as u16);
                Ok(())
            }
            Err(e) => Err(e),
        },
        apps::MONITOR => app.sample(),
        _ => Ok(()),
    };
    if let Err(rc) = init {
        app.status = error(rc);
    }
    let mut dirty = true;
    let mut appearance = client.appearance.get();
    // Bands of the frame the backing currently holds (Phase 11.1).
    let mut shown = scene::Bands::EMPTY;
    let mut shown_once = false;
    // Opt-in latency probes (ARENA_PERF builds only; folded away otherwise):
    // [paint, damage IPC, first event -> damage published, poll IPC,
    // notification wake (time asleep)].
    // [paint, damage, event2damage, poll, wake, key events, pointer events,
    // other events, damaged pixels per publication]
    let mut perf_stats = [arena_desktop::perf::Stat::ZERO; 9];
    let mut perf_last = 0u64;
    let mut first_event = 0u64;
    loop {
        for _ in 0..32 {
            let polled = if arena_desktop::perf::ENABLED {
                service::now()
            } else {
                0
            };
            let event = client.poll().unwrap_or_else(|_| client::exit(72));
            if arena_desktop::perf::ENABLED {
                let now = service::now();
                perf_stats[3].add(now - polled);
                if event.is_some() && first_event == 0 {
                    first_event = now;
                }
                match event {
                    Some(Event::Key(_) | Event::Chord { .. }) => perf_stats[5].add(0),
                    Some(
                        Event::Pointer { .. } | Event::PopupPointer { .. } | Event::Wheel { .. },
                    ) => perf_stats[6].add(0),
                    Some(_) => perf_stats[7].add(0),
                    None => {}
                }
            }
            match event {
                Some(Event::Close) => {
                    if kind == apps::EDITOR && app.editor.dirty {
                        app.closing = true;
                        app.dialog = 4;
                        app.status = "UNSAVED CHANGES / SAVE, DISCARD OR CANCEL";
                        dirty = true;
                    } else {
                        client::exit(42)
                    }
                }
                Some(Event::Key(k)) => {
                    if let Err(rc) = app.key(k, &client) {
                        app.status = error(rc);
                        if kind == apps::TERMINAL {
                            app.terminal.write(error(rc).as_bytes());
                        }
                    }
                    dirty = true;
                }
                Some(Event::Pointer { x, y, buttons }) => {
                    if let Err(rc) = app.pointer(x, y, buttons, &client) {
                        app.status = error(rc);
                    }
                    dirty = true;
                }
                Some(Event::Focus(focused)) => {
                    // Freshness comes from the folder watch, not from focus.
                    if kind == apps::FILES {
                        app.files.ui.focused = focused;
                    }
                    dirty = true;
                }
                Some(Event::Chosen) => {
                    if let Err(rc) = app.chosen(&client) {
                        app.status = error(rc);
                    }
                    dirty = true;
                }
                Some(Event::Configure { width, height }) => {
                    app.pending = Some((width, height));
                    dirty = true;
                }
                Some(Event::PopupPointer { x, y, buttons }) => {
                    if let Err(rc) = app.menu_pointer(x, y, buttons, &client) {
                        app.status = error(rc);
                    }
                    dirty = true;
                }
                Some(Event::Chord { code, mods }) => {
                    if let Err(rc) = app.chord(code, mods, &client) {
                        app.status = error(rc);
                    }
                    dirty = true;
                }
                Some(Event::Wheel { delta, .. }) => {
                    app.wheel(delta);
                    dirty = true;
                }
                Some(Event::Dismissed(handle)) => {
                    if app
                        .menu
                        .as_ref()
                        .is_some_and(|m| m.surface.handle == handle)
                    {
                        app.menu = None;
                    }
                }
                None => break,
            }
            // Nothing more queued: no empty poll (a later event signals the
            // clock again, since this poll cleared the broker's wake mark).
            if !client.more.get() {
                break;
            }
        }
        if client.appearance.get() != appearance {
            appearance = client.appearance.get();
            dirty = true;
        }
        if kind == apps::MONITOR && service::now() >= app.next_sample {
            if let Err(rc) = app.sample() {
                app.status = error(rc);
                app.next_sample = service::now() + 500_000;
            }
            dirty = true;
        }
        // Adopt a configured size: the next frame is painted and published
        // whole at exactly that size.
        let mut resized = false;
        if let Some((width, height)) = app.pending.take()
            && (width, height) != app.size
            && client
                .adopt(usize::from(width), usize::from(height))
                .is_ok()
        {
            app.size = (width, height);
            app.fit();
            resized = true;
            dirty = true;
        }
        if dirty {
            if kind == apps::FILES {
                app.files.status_line();
            }
            let painted = if arena_desktop::perf::ENABLED {
                service::now()
            } else {
                0
            };
            let stride = client.width;
            let pixels = unsafe {
                core::slice::from_raw_parts_mut(client.pixels, client.width * client.height)
            };
            // Repaint only the bands whose keys changed since the frame the
            // backing already holds, and publish exactly those rectangles.
            let next = app.view(appearance);
            let mut bands = scene::Bands::EMPTY;
            next.bands(&mut bands);
            let damage = if shown_once && !resized {
                scene::dirty(&shown, &bands)
            } else {
                scene::Dirty::full(client.width as i32, client.height as i32)
            };
            // Repaint each dirty rectangle in row chunks that fit a bounded
            // scratch holding the chunk's current pixels, so the publication
            // shrinks to the pixels that really changed: one box per dirty
            // rectangle, the union of its chunks' changed boxes.
            let before = unsafe { &mut *(&raw mut BEFORE) };
            let tighten = !damage.is_full();
            let mut tight = [arena_gfxkit::Rect {
                x: 0,
                y: 0,
                width: 0,
                height: 0,
            }; scene::MAX_DIRTY];
            let mut tight_n = 0;
            fn canvas(pixels: &mut [u32], w: usize, h: usize) -> Canvas<'_> {
                Canvas::new(pixels, w, h, w).unwrap_or_else(|_| client::exit(73))
            }
            let (cw, ch) = (client.width, client.height);
            if tighten {
                for r in damage.rects() {
                    let w = r.width as usize;
                    let rows = (before.len() / w.max(1)).max(1);
                    let mut changed: Option<arena_gfxkit::Rect> = None;
                    let mut y = 0usize;
                    while y < r.height as usize {
                        let h = rows.min(r.height as usize - y);
                        let chunk = arena_gfxkit::Rect {
                            x: r.x,
                            y: r.y + y as i32,
                            width: r.width,
                            height: h as u32,
                        };
                        for row in 0..h {
                            let start = (chunk.y as usize + row) * stride + r.x as usize;
                            before[row * w..row * w + w].copy_from_slice(&pixels[start..start + w]);
                        }
                        {
                            let mut c = canvas(&mut *pixels, cw, ch);
                            c.set_clip(chunk);
                            next.paint(&mut c);
                        }
                        if let Some(b) = scene::changed_box(&before[..w * h], pixels, stride, chunk)
                        {
                            changed = Some(changed.map_or(b, |c| scene::union(c, b)));
                        }
                        y += h;
                    }
                    if let Some(b) = changed {
                        tight[tight_n] = b;
                        tight_n += 1;
                    }
                }
            } else {
                scene::repaint(&mut canvas(&mut *pixels, cw, ch), &next, &damage);
            }
            let damaged = if arena_desktop::perf::ENABLED {
                service::now()
            } else {
                0
            };
            if resized {
                client.commit().unwrap_or_else(|_| client::exit(74));
            } else if damage.is_full() {
                client.damage().unwrap_or_else(|_| client::exit(74));
            } else if tight_n > 0 {
                // Exactly the changed pixels; an unchanged repaint publishes
                // nothing.
                client
                    .damage_rects(&tight[..tight_n])
                    .unwrap_or_else(|_| client::exit(74));
            }
            shown = bands;
            shown_once = true;
            if arena_desktop::perf::ENABLED {
                let now = service::now();
                perf_stats[0].add(damaged - painted);
                perf_stats[1].add(now - damaged);
                // Reported in the "us" columns: pixels per publication.
                let published = if tighten {
                    &tight[..tight_n]
                } else {
                    damage.rects()
                };
                let pixels: u64 = published
                    .iter()
                    .map(|r| u64::from(r.width) * u64::from(r.height))
                    .sum();
                perf_stats[8].add(if damage.is_full() {
                    (client.width * client.height) as u64
                } else {
                    pixels
                });
                if first_event != 0 {
                    perf_stats[2].add(now - first_event);
                    first_event = 0;
                }
            }
            dirty = false;
        }
        let mut labels = [""; 8];
        let labels_n = app.menu_labels(&mut labels);
        if let Some(menu) = app.menu.as_mut().filter(|m| m.dirty) {
            let t = menu.surface;
            let pixels = unsafe { core::slice::from_raw_parts_mut(t.pixels, t.width * t.height) };
            let mut canvas = Canvas::new(pixels, t.width, t.height, t.width)
                .unwrap_or_else(|_| client::exit(73));
            view::menu(
                &mut canvas,
                &labels[..labels_n],
                menu.hover,
                arena_ui::theme::palette(appearance & 1 != 0),
            );
            menu.dirty = false;
            // A refused publication means the policy already dismissed it.
            if client.publish_transient(&t).is_err() {
                app.menu = None;
            }
        }
        if arena_desktop::perf::ENABLED && service::now().saturating_sub(perf_last) >= 1_000_000 {
            perf_last = service::now();
            if perf_stats[0].count + perf_stats[2].count + perf_stats[4].count > 0 {
                let mut line = arena_desktop::perf::Line::new();
                line.push(b"[perf app");
                line.number(u64::from(kind));
                line.push(b"]");
                line.stat(b"paint", perf_stats[0]);
                line.stat(b"damage", perf_stats[1]);
                line.stat(b"event2damage", perf_stats[2]);
                line.stat(b"poll", perf_stats[3]);
                line.stat(b"wake", perf_stats[4]);
                line.stat(b"evkey", perf_stats[5]);
                line.stat(b"evptr", perf_stats[6]);
                line.stat(b"evother", perf_stats[7]);
                line.stat(b"pixels", perf_stats[8]);
                line.push(b"\n");
                line.emit(client::log);
            }
            perf_stats = [arena_desktop::perf::Stat::ZERO; 9];
        }
        // Only Monitor has time-driven work; every other built-in client
        // sleeps until the broker signals an event for it.
        let deadline = (kind == apps::MONITOR).then_some(app.next_sample);
        let slept = if arena_desktop::perf::ENABLED {
            service::now()
        } else {
            0
        };
        if kind == apps::FILES {
            app.files_rewatch();
        }
        let woke = service::idle(deadline).unwrap_or_else(|_| client::exit(75));
        if kind == apps::FILES && woke & service::WATCH_BADGE != 0 && app.files_watched() {
            dirty = true;
        }
        if arena_desktop::perf::ENABLED {
            perf_stats[4].add(service::now() - slept);
        }
    }
}
