//! Ordinary multicall ELF. Startup metadata selects a model; capabilities
//! independently determine which operations the broker accepts.
#![no_std]
#![no_main]
#![allow(clippy::deref_addrof)]
use arena_desktop::{
    app_client as service,
    apps::{
        self, layout as l,
        model::{Editor, Line, Terminal},
        scene, view,
    },
    client::{self, Client},
    model::Event,
    service_wire::Frame,
};
use arena_gfxkit::Canvas;
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
    sizes: [u64; 32],
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
}
static mut APP: App = App {
    editor: Editor::new(),
    terminal: Terminal::new(),
    line: Line::new(),
    names: [[0; 32]; 32],
    sizes: [0; 32],
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
};
fn name(bytes: &[u8]) -> Result<[u8; 32], i64> {
    let mut n = [0; 32];
    if bytes.len() > 31 {
        return Err(-2);
    }
    n[..bytes.len()].copy_from_slice(bytes);
    if arena_desktop::scope::public_name(&n) {
        Ok(n)
    } else {
        Err(-2)
    }
}
fn length(n: &[u8; 32]) -> usize {
    n.iter().position(|b| *b == 0).unwrap_or(32)
}
fn error(rc: i64) -> &'static str {
    match rc {
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
fn read(client: &Client, path: [u8; 32]) -> Result<&[u8], i64> {
    let n = call(Frame::Read { name: path })?.0 as usize;
    if n > 4096 {
        return Err(-2);
    }
    // The service filled only this application's first backing page.
    Ok(unsafe { core::slice::from_raw_parts(client.io, n) })
}
fn put(client: &Client, path: [u8; 32], bytes: &[u8]) -> Result<(), i64> {
    if bytes.len() > 4096 {
        return Err(-2);
    }
    unsafe { core::ptr::copy_nonoverlapping(bytes.as_ptr(), client.io, bytes.len()) };
    let result = call(Frame::Put {
        name: path,
        length: bytes.len() as u16,
    })?
    .0;
    if result != bytes.len() as u64 {
        return Err(-2);
    }
    Ok(())
}
fn launch(kind: u8, path: [u8; 32]) -> Result<(), i64> {
    call(Frame::Launch { kind, path }).map(|_| ())
}
impl App {
    fn refresh(&mut self) -> Result<(), i64> {
        let mut cursor = 0;
        self.count = 0;
        for _ in 0..32 {
            let (_, row) = call(Frame::List { cursor })?;
            let Frame::Entry {
                cursor: next,
                size,
                name,
            } = row
            else {
                return Err(-2);
            };
            if next == u32::MAX {
                break;
            }
            if next <= cursor || name[0] == 0 {
                return Err(-2);
            }
            self.names[self.count] = name;
            self.sizes[self.count] = size;
            self.count += 1;
            cursor = next;
        }
        self.selected = self.selected.min(self.count.saturating_sub(1));
        self.top = self.top.min(self.selected);
        Ok(())
    }
    fn select(&mut self, client: &Client) -> Result<(), i64> {
        self.preview_len = 0;
        if self.count == 0 {
            return Ok(());
        }
        let data = read(client, self.names[self.selected])?;
        if !data
            .iter()
            .all(|b| b.is_ascii_graphic() || matches!(b, b' ' | b'\n' | b'\t'))
        {
            return Err(-2);
        }
        self.preview[..data.len()].copy_from_slice(data);
        self.preview_len = data.len();
        Ok(())
    }
    fn open(&mut self, client: &Client, path: [u8; 32]) -> Result<(), i64> {
        let data = read(client, path)?;
        self.editor
            .load(data, view::string(&path[..length(&path)]))
            .map_err(|_| -2)?;
        self.top = 0;
        self.status = "OPENED FROM AFS1";
        Ok(())
    }
    fn save(&mut self, client: &Client, path: [u8; 32]) -> Result<(), i64> {
        if !arena_desktop::scope::public_name(&path) {
            return Err(-2);
        }
        put(client, path, &self.editor.data[..self.editor.len])?;
        self.editor.path = path;
        self.editor.dirty = false;
        self.status = "SAVED: TRANSACTION COMMITTED";
        if self.closing {
            client::exit(42)
        }
        Ok(())
    }
    fn editor_dialog(&mut self, mode: u8) {
        self.dialog = mode;
        self.line.set(if self.editor.path[0] != 0 {
            &self.editor.path[..length(&self.editor.path)]
        } else {
            b"user-note"
        });
    }
    fn accept(&mut self, client: &Client) -> Result<(), i64> {
        let path = name(&self.line.bytes[..self.line.len])?;
        match self.dialog {
            1 => self.save(client, path)?,
            2 => self.open(client, path)?,
            3 => {
                call(Frame::Create { name: path })?;
                self.refresh()?;
                self.select(client)?;
                self.status = "CREATED EMPTY FILE";
            }
            _ => return Err(-2),
        }
        self.dialog = 0;
        Ok(())
    }
    fn command(&mut self, client: &Client) -> Result<(), i64> {
        let (bytes, n) = self.terminal.consume();
        let text = &bytes[..n];
        self.terminal.write(text);
        let split = text.iter().position(|b| *b == b' ').unwrap_or(n);
        let cmd = &text[..split];
        let args = if split < n { &text[split + 1..] } else { b"" };
        match cmd {
            b"" => {},
            b"help" => self.terminal.write(b"help echo ls cat ps put rm launch clear\nFiles: user-* / text up to 4096 bytes\nlaunch term|files|edit|settings|monitor|gallery"),
            b"echo" => self.terminal.write(args),
            b"ls" => {self.refresh()?; for i in 0..self.count {self.terminal.write(&self.names[i][..length(&self.names[i])]);}},
            b"cat" => {let data=read(client,name(args)?)?; if !data.is_ascii() {return Err(-2)} self.terminal.write(data);},
            b"rm" => {call(Frame::Delete{name:name(args)?})?; self.terminal.write(b"Deleted");},
            b"put" => {let s=args.iter().position(|b|*b==b' ').unwrap_or(args.len()); let data=if s<args.len(){&args[s+1..]}else{b""}; put(client,name(&args[..s])?,data)?; self.terminal.write(b"Committed");},
            b"ps" => {self.process_count=service::processes(&mut self.processes)?; for i in 0..self.process_count {let mut b=[0;64]; let mut n=0; append(&mut b,&mut n,b"PID ");number(&mut b,&mut n,self.processes[i].0);append(&mut b,&mut n,b" THREADS ");number(&mut b,&mut n,self.processes[i].1);self.terminal.write(&b[..n]);}},
            b"launch" => {let kind=match args {b"term"=>0,b"files"=>1,b"edit"=>2,b"settings"=>3,b"monitor"=>4,b"gallery"=>5,_=>return Err(-2)};launch(kind,[0;32])?; self.terminal.write(b"Real application started");},
            b"clear" => {self.terminal.count=0;},
            _ => self.terminal.write(b"Unknown command; use help"),
        }
        Ok(())
    }
    fn key(&mut self, key: u16, client: &Client) -> Result<(), i64> {
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
                        (self.top + 1).min(self.terminal.count.saturating_sub(l::TERMINAL_ROWS));
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
                let row = l::visual_row(&self.editor);
                if row < self.top {
                    self.top = row;
                }
                if row >= self.top + l::EDIT_ROWS {
                    self.top = row.saturating_sub(l::EDIT_ROWS - 1);
                }
                self.status = if self.editor.dirty {
                    "MODIFIED / SAVE COMMITS COMPLETE FILE"
                } else {
                    "READY"
                };
            }
            apps::FILES => {
                match key {
                    258 => self.selected = self.selected.saturating_sub(1),
                    259 => self.selected = (self.selected + 1).min(self.count.saturating_sub(1)),
                    13 if self.count > 0 => launch(apps::EDITOR, self.names[self.selected])?,
                    _ => {}
                }
                if self.selected < self.top {
                    self.top = self.selected;
                }
                if self.selected >= self.top + l::FILE_ROWS {
                    self.top = self.selected.saturating_sub(l::FILE_ROWS - 1);
                }
                self.select(client)?;
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
                    self.top =
                        (self.top + 1).min(self.process_count.saturating_sub(l::MONITOR_ROWS))
                }
            }
            _ => {}
        }
        Ok(())
    }
    fn pointer(&mut self, x: i32, y: i32, buttons: u8, client: &Client) -> Result<(), i64> {
        let pressed = buttons & 1 != 0 && self.buttons & 1 == 0;
        self.buttons = buttons;
        if !pressed {
            return Ok(());
        }
        if self.dialog != 0 {
            if self.dialog == 4 {
                if l::hit(l::NAME_FIELD, x, y) {
                    if self.editor.path[0] == 0 {
                        self.editor_dialog(1)
                    } else {
                        self.save(client, self.editor.path)?;
                    }
                } else if l::hit(l::PRIMARY, x, y) {
                    client::exit(42)
                } else if l::hit(l::SECONDARY, x, y) {
                    self.dialog = 0;
                    self.closing = false;
                    client.cancel_close()?;
                }
                return Ok(());
            }
            if l::hit(l::PRIMARY, x, y) {
                self.accept(client)?;
            } else if l::hit(l::SECONDARY, x, y) {
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
                if l::hit(l::NEW, x, y) {
                    if self.editor.dirty {
                        self.status = "SAVE FIRST OR OPEN A NEW EDITOR";
                    } else {
                        self.editor.load(b"", "").map_err(|_| -2)?;
                        self.top = 0;
                    }
                } else if l::hit(l::SAVE, x, y) {
                    if self.editor.path[0] == 0 {
                        self.editor_dialog(1)
                    } else {
                        self.save(client, self.editor.path)?;
                    }
                } else if l::hit(l::SAVE_AS, x, y) {
                    self.editor_dialog(1)
                } else if l::hit(l::OPEN, x, y) {
                    if self.editor.dirty {
                        self.status = "SAVE FIRST OR OPEN A NEW EDITOR";
                    } else {
                        self.editor_dialog(2)
                    }
                } else if l::hit(l::EDIT_TEXT, x, y) {
                    self.editor.cursor = l::visual_cursor(
                        &self.editor,
                        self.top + ((y - l::EDIT_TEXT.y - 6).max(0) / m::LINE_HEIGHT) as usize,
                        ((x - l::EDIT_TEXT.x - 6).max(0) / m::FONT_ADVANCE) as usize,
                    );
                }
            }
            apps::FILES => {
                if l::hit(l::NEW, x, y) {
                    self.line.set(b"user-new");
                    self.dialog = 3;
                } else if l::hit(l::SAVE, x, y) {
                    self.refresh()?;
                    self.select(client)?;
                    self.status = "REFRESHED";
                } else if l::hit(l::OPEN, x, y) && self.count > 0 {
                    launch(apps::EDITOR, self.names[self.selected])?;
                } else if l::hit(l::DELETE, x, y) && self.count > 0 {
                    call(Frame::Delete {
                        name: self.names[self.selected],
                    })?;
                    self.refresh()?;
                    self.select(client)?;
                    self.status = "DELETED";
                } else if l::hit(l::FILE_LIST, x, y) {
                    let row = ((y - l::FILE_LIST.y) / l::ROW_H) as usize + self.top;
                    if row < self.count {
                        self.selected = row;
                        self.select(client)?;
                        self.status = "SELECTED / OPEN IN EDITOR";
                    }
                }
            }
            apps::SETTINGS if l::hit(l::APPEARANCE, x, y) => {
                let dark = client.appearance.get() & 1 == 0;
                call(Frame::Configure {
                    theme: u8::from(dark),
                    motion: client.appearance.get() & 2 != 0,
                })?;
                self.status = "APPEARANCE COMMITTED TO AFS1";
            }
            apps::SETTINGS if l::hit(l::MOTION, x, y) => {
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
        scene::View {
            kind: self.kind,
            appearance,
            status: self.status,
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
#[unsafe(no_mangle)]
pub extern "C" fn _start() -> ! {
    let (kind, dark, motion, path) = service::startup().unwrap_or_else(|_| client::exit(70));
    service::audit(kind).unwrap_or_else(|_| client::exit(76));
    let client = Client::connect(
        m::WINDOW_WIDTH,
        m::WINDOW_HEIGHT,
        apps::TITLES[kind as usize],
    )
    .unwrap_or_else(|_| client::exit(71));
    client
        .appearance
        .set(u8::from(dark) | (u8::from(motion) << 1));
    let app = unsafe { &mut *(&raw mut APP) };
    app.kind = kind;
    let init = match kind {
        apps::TERMINAL => {
            app.terminal
                .write(b"ArenaOS ordinary command session\nType help. Files are scoped to user-*.");
            Ok(())
        }
        apps::FILES => app.refresh().and_then(|_| app.select(&client)),
        apps::EDITOR if path[0] != 0 => app.open(&client, path),
        apps::SETTINGS => match service::exchange(Frame::Display, 1) {
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
    let mut perf_stats = [arena_desktop::perf::Stat::ZERO; 5];
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
                Some(Event::Focus(_)) => dirty = true,
                None => break,
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
        if dirty {
            let painted = if arena_desktop::perf::ENABLED {
                service::now()
            } else {
                0
            };
            let pixels = unsafe {
                core::slice::from_raw_parts_mut(client.pixels, client.width * client.height)
            };
            let mut canvas = Canvas::new(pixels, client.width, client.height, client.width)
                .unwrap_or_else(|_| client::exit(73));
            // Repaint only the bands whose keys changed since the frame the
            // backing already holds, and publish exactly those rectangles.
            let next = app.view(appearance);
            let mut bands = scene::Bands::EMPTY;
            next.bands(&mut bands);
            let damage = if shown_once {
                scene::dirty(&shown, &bands)
            } else {
                scene::Dirty::full()
            };
            scene::repaint(&mut canvas, &next, &damage);
            let damaged = if arena_desktop::perf::ENABLED {
                service::now()
            } else {
                0
            };
            if damage.is_full() {
                client.damage().unwrap_or_else(|_| client::exit(74));
            } else {
                client
                    .damage_rects(damage.rects())
                    .unwrap_or_else(|_| client::exit(74));
            }
            shown = bands;
            shown_once = true;
            if arena_desktop::perf::ENABLED {
                let now = service::now();
                perf_stats[0].add(damaged - painted);
                perf_stats[1].add(now - damaged);
                if first_event != 0 {
                    perf_stats[2].add(now - first_event);
                    first_event = 0;
                }
            }
            dirty = false;
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
                line.push(b"\n");
                client::log(line.as_bytes());
            }
            perf_stats = [arena_desktop::perf::Stat::ZERO; 5];
        }
        // Only Monitor has time-driven work; every other built-in client
        // sleeps until the broker signals an event for it.
        let deadline = (kind == apps::MONITOR).then_some(app.next_sample);
        let slept = if arena_desktop::perf::ENABLED {
            service::now()
        } else {
            0
        };
        service::idle(deadline).unwrap_or_else(|_| client::exit(75));
        if arena_desktop::perf::ENABLED {
            perf_stats[4].add(service::now() - slept);
        }
    }
}
