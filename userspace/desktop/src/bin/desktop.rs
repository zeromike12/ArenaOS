//! Authority/lifecycle service. Presentation belongs in shell.rs and compose.rs.
#![no_std]
#![no_main]
#![allow(clippy::deref_addrof, clippy::collapsible_if)]
use arena_desktop::{
    compose::{self, Damage, PopupScene, Scene, WindowScene},
    model as wm,
    shell::Shell,
};
use arena_desktop::{
    input_wire,
    model::{Action, State},
    wire::Frame,
};
use arena_gfxkit::Canvas;
use core::panic::PanicInfo;
#[path = "../../../abi.rs"]
mod abi;
use abi::*;
const DISPLAY: u64 = 0;
const SERVER: u64 = 1;
const INPUT: u64 = 2;
const POOL: u64 = 3;
const CLOCK: u64 = 4;
const CALL_SIDE: u64 = 5;
const APPLICATION: u64 = 6;
/// filesd's /Users/user record (ADR-0077), minted by the kernel.
const USER_ROOT: u64 = 20;
const PIXEL_OFFSET: usize = 4096;
const LIMIT: usize = wm::MAX_WINDOWS;
/// Capability slots in the kernel table (ADR-0075).
const CAP_SLOTS: u64 = 64;
/// Pages of one bounded transient surface (ADR-0075); the final pages of a
/// session's shared reservation and of its private snapshot.
const TRANSIENT_PAGES: u64 = (wm::TRANSIENT_MAX_PIXELS * 4 / 4096) as u64;
/// The session's filesd I/O page, last in the reservation (ADR-0077).
const FILE_PAGES: u64 = arena_desktop::client::FILE_PAGES as u64;
/// Per-session memory, fixed for the screen at startup (ADR-0075): every
/// session can take any size up to the work area (maximize) without
/// reallocation, so a resize never changes which memory backs a session.
#[derive(Clone, Copy)]
struct Reserve {
    /// Main surface pages (work area, rounded up).
    surface_pages: u64,
    /// Shared region: I/O page + main surface + transient surface + the
    /// client's filesd page.
    shared_pages: u64,
    /// Private snapshot region: main surface + transient surface.
    snapshot_pages: u64,
}
static mut RESERVE: Reserve = Reserve {
    surface_pages: 0,
    shared_pages: 0,
    snapshot_pages: 0,
};
/// Private client clock of session `i`: slots 7..=12, then 14..=19 (13
/// is the filesystem endpoint).
fn clock(i: usize) -> u64 {
    if i < 6 { 7 + i as u64 } else { 8 + i as u64 }
}
/// Presentation state of a session's transient surface, valid only while
/// the window policy still holds a popup with this handle.
#[derive(Clone, Copy)]
struct PopupState {
    handle: u64,
    published: bool,
    content: u64,
    regions: compose::Regions,
}
const NO_POPUP: PopupState = PopupState {
    handle: 0,
    published: false,
    content: 0,
    regions: compose::Regions::NONE,
};
#[derive(Clone, Copy)]
struct Session {
    region: u64,
    id: u64,
    va: u64,
    process: u64,
    handle: u64,
    title: [u8; 32],
    published: bool,
    kind: u8,
    scope: u8,
    launch_targets: u8,
    path: [u8; 32],
    close_pending: bool,
    ending: bool,
    reveal: arena_ui::motion::Motion,
    focus: arena_ui::motion::Motion,
    reveal_last: i32,
    focus_last: i32,
    /// Advances on every authenticated Damage (published raster changed).
    content: u64,
    /// Published regions since the last presented frame (Phase 11.1).
    regions: compose::Regions,
    /// Private snapshot mapping (held by its mapping pin only, no cap).
    snapshot: u64,
    /// Size of the published main raster (Phase 11.3).
    surface: (u16, u16),
    popup: PopupState,
    /// Head of this session's filesd lineage (held by the broker only, no
    /// rights; revoking it retires every file capability of the session).
    files_head: u64,
    /// The chooser's outcome is ready for TakeGrant (granted or cancelled).
    grant_ready: bool,
    /// The capability the chooser granted (CAP_NONE: cancelled), its title
    /// (display only) and whether it was a save.
    grant: u64,
    grant_title: [u8; 32],
    grant_save: bool,
    grant_read_only: bool,
}
const EMPTY: Session = Session {
    region: CAP_NONE,
    id: 0,
    va: 0,
    process: CAP_NONE,
    handle: 0,
    title: [0; 32],
    published: false,
    kind: 5,
    scope: 0,
    launch_targets: 0,
    path: [0; 32],
    close_pending: false,
    ending: false,
    reveal: arena_ui::motion::Motion::fixed(arena_ui::metrics::TITLE_HEIGHT),
    focus: arena_ui::motion::Motion::fixed(0),
    reveal_last: arena_ui::metrics::TITLE_HEIGHT,
    focus_last: 0,
    content: 0,
    regions: compose::Regions::NONE,
    snapshot: 0,
    surface: (0, 0),
    popup: NO_POPUP,
    files_head: CAP_NONE,
    grant_ready: false,
    grant: CAP_NONE,
    grant_title: [0; 32],
    grant_save: false,
    grant_read_only: false,
};
static mut PREFS: arena_desktop::preferences::Preferences =
    arena_desktop::preferences::Preferences {
        dark: false,
        motion: true,
    };
static mut FILES: Option<arena_desktop::fs_backend::Fs> = None;
/// The broker's own filesd session (None = AFS2 offline or absent).
static mut AFS2: Option<arena_desktop::files::Files> = None;
/// A file capability offered for the next Editor launch (ADR-0077).
static mut OFFER: u64 = CAP_NONE;
/// Rights an application gets on a document it opened or saved.
const R_DOC: u8 = arena_desktop::filesd_wire::R_READ | arena_desktop::filesd_wire::R_WRITE;
static mut UPTIME_SECOND: u64 = 0;
static mut CAP_HIGH_WATER: u64 = 0;
static mut NOTICE: Option<(&'static str, u64)> = None;
/// A built-in client's private clock was signalled for queued events and
/// it has not polled since (avoids re-signalling on every request).
static mut WOKEN: [bool; LIMIT] = [false; LIMIT];
/// Appearance changed: every built-in client should repaint now.
static mut WAKE_ALL: bool = false;
/// Badges on the compositor's own notification (ADR-0071). One wait
/// covers every event source; the merged badge needs no dispatch because
/// each wake re-runs sweep/animate/drain.
const BADGE_TIMER: u64 = 1;
/// A client or input request was queued while the compositor was not
/// parked in RECV (endpoint-bound notification).
const BADGE_REQUEST: u64 = 2;
/// A child application process exited (spawn exit notification).
const BADGE_EXIT: u64 = 4;
/// Nothing time-driven is due: sleep until an event.
const NO_DEADLINE: u64 = u64::MAX;
/// Next instant time-driven presentation work is due: a running motion
/// (frame pacing), the next uptime second shown in the bar, or a notice
/// expiry. Everything else (requests, input, child exit) wakes the
/// compositor through its notification instead of a poll.
fn next_deadline(now: u64) -> u64 {
    let mut due = (now / 1_000_000 + 1) * 1_000_000;
    if let Some((_, until)) = unsafe { NOTICE } {
        due = due.min(until);
    }
    let sessions = unsafe { &*(&raw const SESSIONS) };
    if sessions
        .iter()
        .any(|s| s.id != 0 && (s.reveal.active(now) || s.focus.active(now) || s.ending))
    {
        due = due.min(now + arena_ui::motion::FRAME_US);
    }
    // A held key repeats (window policy, Phase 11.4).
    if let Some(at) = unsafe { (*(&raw const WM)).repeat_deadline() } {
        due = due.min(at);
    }
    due
}
/// Signal built-in clients that have queued events (or must repaint) on
/// the private clock the broker already holds for them. Their `idle()`
/// cancels the then-redundant timer. Third-party signed applications are
/// not signalled: they are not known to cancel timers, and early wakes
/// could otherwise accumulate armed timers in the kernel's bounded table.
fn wake_clients() {
    let state = unsafe { &*(&raw const WM) };
    let all = unsafe { core::mem::replace(&mut *(&raw mut WAKE_ALL), false) };
    for (i, s) in unsafe { &*(&raw const SESSIONS) }.iter().enumerate() {
        if s.handle == 0 || s.kind >= 6 || unsafe { WOKEN[i] } {
            continue;
        }
        if (all || state.pending(s.handle)) && unsafe { syscall2(SYS_NOTIFY, clock(i), 1) } == 0 {
            unsafe { WOKEN[i] = true };
        }
    }
}
// Opt-in latency probes (ARENA_PERF builds only; folded away otherwise).
use arena_desktop::perf::{self, Stat};
const P_RENDER: usize = 0;
const P_COMPOSE: usize = 1;
const P_RECTS: usize = 2;
const P_SKIPPED: usize = 3;
const P_PRESENT: usize = 4;
const P_DAMAGE: usize = 5;
const P_INPUT: usize = 6;
const P_SLEEP: usize = 7;
const P_REQUEST: usize = 8;
const P_KPIXELS: usize = 9;
static mut PERF: [Stat; 10] = [Stat::ZERO; 10];
static mut PERF_LAST: u64 = 0;
fn probe(index: usize, since: u64) {
    if perf::ENABLED {
        let now = arena_desktop::app_client::now();
        unsafe { (*(&raw mut PERF))[index].add(now.saturating_sub(since)) };
    }
}
fn perf_now() -> u64 {
    if perf::ENABLED {
        arena_desktop::app_client::now()
    } else {
        0
    }
}
fn perf_report() {
    if !perf::ENABLED {
        return;
    }
    let now = arena_desktop::app_client::now();
    if now.saturating_sub(unsafe { PERF_LAST }) < 1_000_000 {
        return;
    }
    let stats = unsafe { *(&raw const PERF) };
    unsafe {
        PERF_LAST = now;
        PERF = [Stat::ZERO; 10];
    }
    let mut line = perf::Line::new();
    line.push(b"[perf desktop]");
    for (name, i) in [
        (&b"render"[..], P_RENDER),
        (b"compose", P_COMPOSE),
        (b"rects", P_RECTS),
        (b"nodamage", P_SKIPPED),
        (b"present", P_PRESENT),
        (b"damage", P_DAMAGE),
        (b"input2frame", P_INPUT),
        (b"sleep", P_SLEEP),
        (b"request", P_REQUEST),
        (b"kpx", P_KPIXELS),
    ] {
        line.stat(name, stats[i]);
    }
    line.push(b"\n");
    log(line.as_bytes());
}
// Private broker memory: each session's snapshot region is mapped only by
// the broker; clients never map or receive it. Only an authenticated Damage
// or Resize request publishes into it.
fn snapshot_of(s: &Session) -> (&'static [u32], &'static [u32]) {
    let r = unsafe { RESERVE };
    if s.snapshot == 0 {
        return (&[], &[]);
    }
    let main = (r.surface_pages * 1024) as usize;
    unsafe {
        (
            core::slice::from_raw_parts(s.snapshot as *const u32, main),
            core::slice::from_raw_parts(
                (s.snapshot + r.surface_pages * 4096) as *const u32,
                (TRANSIENT_PAGES * 1024) as usize,
            ),
        )
    }
}
/// Copy exactly the declared rectangles of a `stride`-wide raster from the
/// client's shared staging memory into the private snapshot. The sender is
/// blocked in CALL on this single-core topology; staging bytes outside the
/// rectangles never become visible.
fn publish_rects(src: u64, dst: u64, stride: usize, rects: &[[u16; 4]]) {
    for &[x, y, w, h] in rects {
        for row in y as usize..y as usize + h as usize {
            for col in x as usize..x as usize + w as usize {
                let pixel = row * stride + col;
                unsafe {
                    core::ptr::write_volatile(
                        (dst as *mut u32).add(pixel),
                        core::ptr::read_volatile((src as *const u32).add(pixel)),
                    );
                }
            }
        }
    }
}
static mut SESSIONS: [Session; LIMIT] = [EMPTY; LIMIT];
static mut WM: State = match State::new(800, 600) {
    Ok(s) => s,
    Err(_) => panic!("constant geometry"),
};
#[panic_handler]
fn panic(_: &PanicInfo<'_>) -> ! {
    die(99)
}
fn die(c: u64) -> ! {
    let digits = [b'0' + ((c / 10) % 10) as u8, b'0' + (c % 10) as u8];
    log(b"[desktop] failed stage ");
    log(&digits);
    log(b"\n");
    arena_desktop::client::exit(c)
}
fn log(s: &[u8]) {
    unsafe {
        syscall2(SYS_DEBUG_WRITE, s.as_ptr() as u64, s.len() as u64);
    }
}
fn log_number(mut value: u64) {
    let mut digits = [0u8; 20];
    let mut n = 0;
    loop {
        digits[n] = b'0' + (value % 10) as u8;
        n += 1;
        value /= 10;
        if value == 0 {
            break;
        }
    }
    for digit in digits[..n].iter().rev() {
        log(core::slice::from_ref(digit));
    }
}
/// Sample native occupancy while request references are still landed.
fn transient_caps() {
    let mut counts = [0u64; 9];
    if unsafe { syscall6(SYS_OBSERVE, POOL, counts.as_mut_ptr() as u64, 0, 0, 0, 0) } != 0 {
        die(77)
    }
    if counts[7] > unsafe { CAP_HIGH_WATER } {
        unsafe { CAP_HIGH_WATER = counts[7] };
        log(b"[desktop] measured broker cap high-water=");
        log_number(counts[7]);
        log(b"\n");
    }
}
fn snapshot() {
    let mut counts = [0u64; 9];
    if unsafe { syscall6(SYS_OBSERVE, POOL, counts.as_mut_ptr() as u64, 0, 0, 0, 0) } != 0 {
        die(77)
    }
    log(b"[desktop] measured frames/records/processes/regions/pages/maps/caps=");
    for (i, v) in [
        counts[0], counts[2], counts[3], counts[4], counts[5], counts[6], counts[7],
    ]
    .iter()
    .enumerate()
    {
        if i != 0 {
            log(b"/");
        }
        log_number(*v);
    }
    log(b"\n");
}

// ---- the trusted chooser (powerbox, ADR-0077) ------------------------------
//
// Drawn by the shell, driven only by the user's own input, resolving names
// through the broker's own filesd page on /Users/user. The requesting
// application learns only the outcome and receives a capability for
// exactly the chosen file, in its own lineage.
const CHOOSER_MAX: usize = 64;
struct Chooser {
    session: usize,
    save: bool,
    read_only: bool,
    /// Folder shown, relative to /Users/user.
    dir: [u8; 200],
    dir_len: usize,
    dir_cap: u64,
    entries: [arena_desktop::files::Entry; CHOOSER_MAX],
    count: usize,
    more: bool,
    selected: Option<usize>,
    top: usize,
    name: [u8; 32],
    name_len: usize,
    /// Save over an existing file asks once more.
    confirm: bool,
    message: &'static str,
    buttons: u8,
}
static mut CHOOSER: Option<Chooser> = None;

fn chooser_open(index: usize, save: bool, read_only: bool, name: [u8; 32]) -> Result<(), i64> {
    let s = unsafe { SESSIONS[index] };
    if unsafe { (*(&raw const CHOOSER)).is_some() } || s.handle == 0 || s.grant_ready {
        return Err(STATUS_BUSY);
    }
    if unsafe { (*(&raw const AFS2)).is_none() } || s.files_head == CAP_NONE {
        return Err(-3015);
    }
    let n = name.iter().position(|b| *b == 0).unwrap_or(32).min(31);
    let mut ch = Chooser {
        session: index,
        save,
        read_only,
        dir: [0; 200],
        dir_len: 9,
        dir_cap: CAP_NONE,
        entries: [arena_desktop::files::Entry::EMPTY; CHOOSER_MAX],
        count: 0,
        more: false,
        selected: None,
        top: 0,
        name: [0; 32],
        name_len: if save { n } else { 0 },
        confirm: false,
        message: "",
        buttons: 0,
    };
    ch.dir[..9].copy_from_slice(b"Documents");
    if save {
        ch.name[..n].copy_from_slice(&name[..n]);
    }
    unsafe { CHOOSER = Some(ch) };
    chooser_load();
    log(b"[desktop] trusted chooser opened\n");
    Ok(())
}
fn chooser_close() {
    if let Some(ch) = unsafe { (*(&raw mut CHOOSER)).take() } {
        if ch.dir_cap != CAP_NONE {
            if let Some(f) = unsafe { &*(&raw const AFS2) } {
                f.release(ch.dir_cap);
            }
        }
    }
}
/// (Re)list the chooser's folder through the broker's own session.
fn chooser_load() {
    let Some(ch) = (unsafe { (*(&raw mut CHOOSER)).as_mut() }) else {
        return;
    };
    let Some(f) = (unsafe { &*(&raw const AFS2) }) else {
        return;
    };
    if ch.dir_cap != CAP_NONE {
        f.release(ch.dir_cap);
        ch.dir_cap = CAP_NONE;
    }
    use arena_desktop::filesd_wire::{R_CREATE, R_LIST};
    ch.count = 0;
    ch.selected = None;
    ch.top = 0;
    // Rights only narrow from a folder to what is opened in it, so the
    // folder carries what the chooser may grant (R_DOC) as well.
    match f.walk(USER_ROOT, &ch.dir[..ch.dir_len], R_LIST | R_CREATE | R_DOC) {
        Ok((cap, 2)) => ch.dir_cap = cap,
        Ok((cap, _)) => {
            f.release(cap);
            ch.message = "NOT A FOLDER";
            return;
        }
        Err(e) => {
            ch.message = arena_desktop::files::describe_status(e);
            return;
        }
    }
    let mut after = [0u8; arena_desktop::files::NAME_MAX];
    let mut after_len = 0;
    ch.more = false;
    while ch.count < CHOOSER_MAX {
        match f.list(ch.dir_cap, &after[..after_len], &mut ch.entries[ch.count..]) {
            Ok((0, _)) => break,
            Ok((n, more)) => {
                ch.count += n;
                let last = ch.entries[ch.count - 1];
                after[..last.name().len()].copy_from_slice(last.name());
                after_len = last.name().len();
                if !more {
                    break;
                }
                ch.more = ch.count == CHOOSER_MAX;
            }
            Err(e) => {
                ch.message = arena_desktop::files::describe_status(e);
                break;
            }
        }
    }
    if ch.more {
        ch.message = "SHOWING THE FIRST 64 ITEMS";
    }
}
/// Finish: hand the outcome to the requesting session and close.
fn chooser_finish(granted: Option<(u64, [u8; 32])>) {
    let Some(index) = (unsafe { (*(&raw const CHOOSER)).as_ref() }).map(|c| c.session) else {
        return;
    };
    let (save, read_only) = unsafe { (*(&raw const CHOOSER)).as_ref() }
        .map_or((false, false), |c| (c.save, c.read_only));
    chooser_close();
    let s = unsafe { &mut *(&raw mut SESSIONS).cast::<Session>().add(index) };
    s.grant_ready = true;
    s.grant_save = save;
    s.grant_read_only = read_only;
    match granted {
        Some((cap, title)) => {
            s.grant = cap;
            s.grant_title = title;
            log(b"[desktop] trusted chooser granted one file capability\n");
        }
        None => {
            s.grant = CAP_NONE;
            log(b"[desktop] trusted chooser cancelled; nothing granted\n");
        }
    }
    // Delivered by the main loop once no window-policy borrow is live.
    unsafe { CHOSEN_TO = s.handle };
}
/// The window whose chooser just finished (0: none).
static mut CHOSEN_TO: u64 = 0;
fn deliver_chosen() {
    let handle = unsafe { core::mem::replace(&mut *(&raw mut CHOSEN_TO), 0) };
    if handle != 0 {
        unsafe { (&mut *(&raw mut WM)).send(handle, arena_desktop::model::Event::Chosen) };
    }
}
fn chooser_enter(name: &[u8]) {
    let Some(ch) = (unsafe { (*(&raw mut CHOOSER)).as_mut() }) else {
        return;
    };
    let extra = name.len() + usize::from(ch.dir_len != 0);
    if ch.dir_len + extra > ch.dir.len() {
        ch.message = "PATH TOO LONG";
        return;
    }
    if ch.dir_len != 0 {
        ch.dir[ch.dir_len] = b'/';
        ch.dir_len += 1;
    }
    ch.dir[ch.dir_len..ch.dir_len + name.len()].copy_from_slice(name);
    ch.dir_len += name.len();
    ch.message = "";
    chooser_load();
}
fn chooser_up() {
    let Some(ch) = (unsafe { (*(&raw mut CHOOSER)).as_mut() }) else {
        return;
    };
    if ch.dir_len == 0 {
        return;
    }
    ch.dir_len = ch.dir[..ch.dir_len]
        .iter()
        .rposition(|b| *b == b'/')
        .unwrap_or(0);
    ch.message = "";
    chooser_load();
}
/// Open the selected file, or save under the typed name.
fn chooser_accept() {
    let Some(ch) = (unsafe { (*(&raw mut CHOOSER)).as_mut() }) else {
        return;
    };
    let Some(f) = (unsafe { &*(&raw const AFS2) }) else {
        return;
    };
    let head = unsafe { SESSIONS[ch.session].files_head };
    let mut title = [0u8; 32];
    if !ch.save {
        let Some(i) = ch.selected else {
            ch.message = "SELECT A DOCUMENT";
            return;
        };
        let e = ch.entries[i];
        if e.is_dir() {
            return chooser_enter(e.name());
        }
        let n = e.name().len().min(31);
        title[..n].copy_from_slice(&e.name()[..n]);
        let rights = if ch.read_only {
            arena_desktop::filesd_wire::R_READ
        } else {
            R_DOC
        };
        match f.open_child_in(ch.dir_cap, e.name(), rights, head) {
            Ok((cap, _)) => chooser_finish(Some((cap, title))),
            Err(e) => ch.message = arena_desktop::files::describe_status(e),
        }
        return;
    }
    let name = &ch.name[..ch.name_len];
    if name.is_empty() {
        ch.message = "TYPE A NAME";
        return;
    }
    let exists = ch.entries[..ch.count].iter().any(|e| e.name() == name);
    if exists
        && ch.entries[..ch.count]
            .iter()
            .any(|e| e.name() == name && e.is_dir())
    {
        ch.message = "A FOLDER HAS THAT NAME";
        return;
    }
    if exists && !ch.confirm {
        ch.confirm = true;
        ch.message = "REPLACE THE EXISTING FILE? SAVE AGAIN";
        return;
    }
    if !exists {
        if let Err(e) = f.create(ch.dir_cap, name) {
            ch.message = arena_desktop::files::describe_status(e);
            return;
        }
    }
    title[..name.len()].copy_from_slice(name);
    match f.open_child_in(ch.dir_cap, name, R_DOC, head) {
        Ok((cap, _)) => chooser_finish(Some((cap, title))),
        Err(e) => ch.message = arena_desktop::files::describe_status(e),
    }
}
fn chooser_select(i: usize) {
    let Some(ch) = (unsafe { (*(&raw mut CHOOSER)).as_mut() }) else {
        return;
    };
    if i >= ch.count {
        return;
    }
    ch.selected = Some(i);
    if i < ch.top {
        ch.top = i;
    }
    if i >= ch.top + arena_desktop::shell::CHOOSER_ROWS {
        ch.top = i + 1 - arena_desktop::shell::CHOOSER_ROWS;
    }
    let e = ch.entries[i];
    if ch.save && !e.is_dir() && e.name().len() < 32 {
        ch.name = [0; 32];
        ch.name[..e.name().len()].copy_from_slice(e.name());
        ch.name_len = e.name().len();
        ch.confirm = false;
    }
}
/// A key while the chooser is open. All keys go to the chooser (modal).
fn chooser_key(code: u16) {
    let Some(ch) = (unsafe { (*(&raw mut CHOOSER)).as_mut() }) else {
        return;
    };
    match code {
        27 => chooser_finish(None),
        258 => {
            let i = ch.selected.map_or(0, |i| i.saturating_sub(1));
            chooser_select(i)
        }
        259 => {
            let i = ch
                .selected
                .map_or(0, |i| i + 1)
                .min(ch.count.saturating_sub(1));
            chooser_select(i)
        }
        13 => match ch.selected.map(|i| ch.entries[i]) {
            Some(e) if e.is_dir() => chooser_enter(e.name()),
            _ => chooser_accept(),
        },
        // Backspace edits the name while saving (it never leaves the
        // folder behind the user's back); Left goes up in either mode.
        8 if ch.save => {
            if ch.name_len > 0 {
                ch.name_len -= 1;
                ch.name[ch.name_len] = 0;
                ch.confirm = false;
            }
        }
        8 | 256 => chooser_up(),
        32..=126 if ch.save && ch.name_len < 31 && code != u16::from(b'/') => {
            ch.name[ch.name_len] = code as u8;
            ch.name_len += 1;
            ch.confirm = false;
        }
        _ => {}
    }
}
/// The pointer while the chooser is open: presses go to the chooser only.
fn chooser_pointer(x: i32, y: i32, buttons: u8, w: i32, h: i32) {
    use arena_desktop::shell::{ChooserHit as H, chooser_hit};
    let Some(ch) = (unsafe { (*(&raw mut CHOOSER)).as_mut() }) else {
        return;
    };
    let pressed = buttons & 1 != 0 && ch.buttons & 1 == 0;
    ch.buttons = buttons;
    if !pressed {
        return;
    }
    match chooser_hit(w, h, x, y) {
        H::Row(r) => {
            let i = ch.top + r;
            if ch.selected == Some(i) {
                // A second press on the selected row opens it.
                chooser_key(13);
            } else {
                chooser_select(i);
            }
        }
        H::Up => chooser_up(),
        H::Cancel => chooser_finish(None),
        H::Accept => chooser_accept(),
        _ => {}
    }
}
fn chooser_view() -> Option<arena_desktop::shell::ChooserView> {
    let ch = unsafe { (*(&raw const CHOOSER)).as_ref() }?;
    let rows_max = arena_desktop::shell::CHOOSER_ROWS;
    let mut v = arena_desktop::shell::ChooserView {
        save: ch.save,
        read_only: ch.read_only,
        place: [0; 48],
        rows: [[0; 32]; arena_desktop::shell::CHOOSER_ROWS],
        count: 0,
        selected: None,
        above: ch.top > 0,
        below: ch.top + rows_max < ch.count,
        name: ch.name,
        message: [0; 48],
    };
    let mut k = 0;
    for part in [
        b"Home" as &[u8],
        if ch.dir_len > 0 { b"/" } else { b"" },
        &ch.dir[..ch.dir_len],
    ] {
        let n = part.len().min(47 - k);
        v.place[k..k + n].copy_from_slice(&part[..n]);
        k += n;
    }
    let m = ch.message.as_bytes();
    let n = m.len().min(47);
    v.message[..n].copy_from_slice(&m[..n]);
    for (row, e) in ch.entries[ch.top..ch.count]
        .iter()
        .take(rows_max)
        .enumerate()
    {
        let name = e.name();
        let room = if e.is_dir() { 30 } else { 31 };
        let n = name.len().min(room);
        v.rows[row][..n].copy_from_slice(&name[..n]);
        if name.len() > room {
            v.rows[row][n - 1] = b'~';
        }
        if e.is_dir() {
            v.rows[row][n] = b'/';
        }
        v.count += 1;
    }
    v.selected = ch
        .selected
        .filter(|i| *i >= ch.top && *i < ch.top + rows_max)
        .map(|i| (i - ch.top) as u8);
    Some(v)
}

/// The broker's filesd session: one page of its own region (ADR-0077).
/// With the service offline (no AFS2 region) nothing is created.
fn start_afs2() {
    use arena_desktop::files::Files;
    if describe(USER_ROOT).is_none() || !Files::online(USER_ROOT) {
        log(b"[desktop] AFS2 file service unavailable; file capabilities offline\n");
        return;
    }
    let mut out = [0; 3];
    if unsafe { syscall6(SYS_SHARED_CREATE, POOL, 1, out.as_mut_ptr() as u64, 0, 0, 0) } != 0 {
        die(76)
    }
    let va = unsafe { syscall2(SYS_SHARED_MAP, out[0], 1) };
    if va <= 0 {
        die(76)
    }
    let session = Files::session(USER_ROOT, out[0], va as u64, 0);
    // The broker keeps only its mapping; filesd holds its own.
    destroy(out[0]);
    match session {
        Ok(files) => {
            unsafe { AFS2 = Some(files) };
            log(b"[desktop] AFS2 file service online; /Users/user granted per session\n");
        }
        Err(_) => {
            unsafe { syscall6(SYS_SHARED_UNMAP, va as u64, 0, 0, 0, 0, 0) };
            log(b"[desktop] AFS2 session refused; file capabilities offline\n");
        }
    }
}
// ---- the desktop surface (Phase 11.8) ---------------------------------------
//
// /Users/user/Desktop as icons on the background, resolved through the
// broker's own capability for /Users/user (the same one the chooser uses).
// Opening a document grants the new Editor exactly that file, in its own
// lineage, as a Files offer does.
static mut DESK: arena_desktop::desk::Desk = arena_desktop::desk::Desk::new();
static mut DESK_POLL: u64 = 0;
static mut DESK_BUTTONS: u8 = 0;
fn desk_store() -> Option<arena_desktop::apps::explorer::CapStore> {
    unsafe { *(&raw const AFS2) }.map(|fs| arena_desktop::apps::explorer::CapStore {
        fs,
        root: USER_ROOT,
    })
}
fn desk_load(w: i32, h: i32) {
    if let Some(mut store) = desk_store() {
        let desk = unsafe { &mut *(&raw mut DESK) };
        desk.screen = (w, h);
        if desk.load(&mut store).is_ok() {
            log(b"[desktop] desktop surface shows /Users/user/Desktop\n");
        }
    }
}
fn desk_notice(text: &'static str) {
    unsafe {
        NOTICE = Some((text, arena_desktop::app_client::now() + 3_000_000));
    }
}
fn desk_effect(e: arena_desktop::desk::Effect) {
    use arena_desktop::desk::Effect;
    match e {
        Effect::None => {}
        Effect::NoApplication => desk_notice("NO APPLICATION CAN OPEN THIS FILE"),
        Effect::OpenFolder(p) => {
            // Where Files starts, inside the home capability it is granted
            // anyway: presentation, not authority.
            let mut path = [0u8; 32];
            let b = p.bytes();
            let fits = b.len() <= 32 && b.iter().all(|c| c.is_ascii_graphic() || *c == b' ');
            let shown: &[u8] = if fits { b } else { b"Desktop" };
            path[..shown.len()].copy_from_slice(shown);
            if launch(1, path, CAP_NONE).is_err() {
                desk_notice("LAUNCH REFUSED / DESKTOP CAPACITY");
            }
        }
        Effect::OpenDocument(p) => {
            let Some(files) = (unsafe { *(&raw const AFS2) }) else {
                return;
            };
            match files.walk(USER_ROOT, p.bytes(), R_DOC) {
                Ok((doc, _)) => {
                    let mut title = [0u8; 32];
                    for (t, c) in title.iter_mut().zip(p.name()) {
                        *t = if c.is_ascii_graphic() || *c == b' ' {
                            *c
                        } else {
                            b'?'
                        };
                    }
                    let r = launch(2, title, doc);
                    files.release(doc);
                    if r.is_err() {
                        desk_notice("LAUNCH REFUSED / DESKTOP CAPACITY");
                    }
                }
                Err(_) => desk_notice("THE FILE COULD NOT BE OPENED"),
            }
        }
    }
}
/// Re-list the Desktop folder at most once a second (another application
/// or the terminal may have changed it).
fn desk_poll(now: u64) {
    if now < unsafe { DESK_POLL } {
        return;
    }
    unsafe { DESK_POLL = now + 1_000_000 };
    if let Some(mut store) = desk_store() {
        unsafe { (*(&raw mut DESK)).poll(&mut store) };
    }
}

fn describe(slot: u64) -> Option<[u64; 3]> {
    let mut d = [0; 3];
    (slot != CAP_NONE && unsafe { syscall2(SYS_CAP_DESCRIBE, slot, d.as_mut_ptr() as u64) } == 0)
        .then_some(d)
}
fn destroy(slot: u64) {
    if slot != CAP_NONE && unsafe { syscall1(SYS_CAP_DESTROY, slot) } != 0 {
        die(80)
    }
}
fn display(frame: arena_compositor_model::wire::Frame, cap: u64) -> ([u64; 3], [u8; 64]) {
    let mut b = [0; 64];
    frame.encode(&mut b).unwrap_or_else(|_| die(81));
    let mut o = [0, 0, CAP_NONE];
    if unsafe {
        syscall6(
            SYS_IPC_CALL,
            DISPLAY,
            0,
            0,
            cap,
            o.as_mut_ptr() as u64,
            b.as_mut_ptr() as u64,
        )
    } != 0
    {
        die(82)
    }
    (o, b)
}
fn launch(kind: u8, path: [u8; 32], document: u64) -> Result<(), i64> {
    use arena_desktop::scope;
    // `path` is a title only (ADR-0077): printable, never authority.
    let n = path.iter().position(|b| *b == 0).unwrap_or(32);
    if kind > 5 || n == 32 || !path[..n].iter().all(|b| (0x20..0x7f).contains(b)) {
        return Err(-2);
    }
    // Files are reached through filesd capabilities, not name scopes: the
    // AFS1 `user-*` file functions are retired for applications.
    let (scope, function_rights) = match kind {
        0 | 1 => (
            scope::LAUNCH,
            RIGHTS_READ | RIGHTS_WRITE | RIGHTS_COPY | RIGHTS_DESTROY,
        ),
        2 => (0, RIGHTS_READ | RIGHTS_WRITE | RIGHTS_COPY | RIGHTS_DESTROY),
        3 => (
            scope::PREFERENCES,
            RIGHTS_WRITE | RIGHTS_COPY | RIGHTS_DESTROY,
        ),
        _ => (0, RIGHTS_READ | RIGHTS_COPY),
    };
    let launch_targets = match kind {
        0 => 0x3f,
        1 => 1 << 2,
        _ => 0,
    };
    launch_image(
        APPLICATION,
        kind,
        scope,
        function_rights,
        path,
        kind == 4,
        launch_targets,
        document,
    )
}
/// The session's filesd lineage head and, for the terminal and Files, its
/// /Users/user capability (ADR-0077). Other kinds get file capabilities
/// only through the chooser.
fn file_grants(kind: u8, document: u64) -> (u64, u64) {
    let Some(files) = (unsafe { &*(&raw const AFS2) }) else {
        return (CAP_NONE, CAP_NONE);
    };
    let Ok(head) = files.new_lineage(USER_ROOT) else {
        return (CAP_NONE, CAP_NONE);
    };
    // An Editor opened on a document: a capability for exactly that file,
    // no more than the offered one, in the new session's lineage.
    if kind == 2 && document != CAP_NONE {
        return match files.open_in(document, R_DOC, head) {
            Ok((doc, _)) => (head, doc),
            Err(_) => (head, CAP_NONE),
        };
    }
    if !matches!(kind, 0 | 1) {
        return (head, CAP_NONE);
    }
    match files.open_in(USER_ROOT, arena_desktop::filesd_wire::R_ALL, head) {
        Ok((home, _)) => (head, home),
        Err(_) => (head, CAP_NONE),
    }
}
fn revoke_files(head: u64) {
    if head == CAP_NONE {
        return;
    }
    // Retire the whole lineage in filesd, then drop the broker's slot.
    if let Some(files) = unsafe { &*(&raw const AFS2) } {
        let _ = files.revoke(USER_ROOT, head);
    }
    destroy(head);
}
#[allow(clippy::too_many_arguments)]
fn launch_image(
    image: u64,
    kind: u8,
    scope: u8,
    function_rights: u64,
    path: [u8; 32],
    diagnostics: bool,
    launch_targets: u8,
    document: u64,
) -> Result<(), i64> {
    let ready = unsafe { syscall6(SYS_SPAWN_CHECK, image, 0, 0, 0, 0, 0) };
    if ready != 0 {
        return Err(ready);
    }
    // The session table reserves original lifecycle owners, including children
    // that have not yet requested their window. No numerical caller identity.
    let sessions = unsafe { &mut *(&raw mut SESSIONS) };
    let i = sessions.iter().position(|s| s.id == 0).ok_or(STATUS_BUSY)?;
    // Region, snapshot (transiently), Process, a landed request cap, the
    // filesd lineage head, the home grant and a lent copy (transiently).
    let free = (0..CAP_SLOTS).filter(|s| describe(*s).is_none()).count();
    if free < 7 {
        return Err(STATUS_BUSY);
    }
    let reserve = unsafe { RESERVE };
    let mut out = [0; 3];
    let rc = unsafe {
        syscall6(
            SYS_SHARED_CREATE,
            POOL,
            reserve.shared_pages,
            out.as_mut_ptr() as u64,
            0,
            0,
            0,
        )
    };
    if rc != 0 {
        return Err(rc);
    }
    let region = out[0];
    let id = out[1];
    let va = unsafe { syscall2(SYS_SHARED_MAP, region, 1) };
    if va <= 0 {
        destroy(region);
        return Err(va);
    }
    // The private snapshot lives as long as the broker's mapping of it: the
    // capability is dropped at once, so no other process can ever be given
    // it and it costs no capability slot.
    let mut snap = [0; 3];
    let rc = unsafe {
        syscall6(
            SYS_SHARED_CREATE,
            POOL,
            reserve.snapshot_pages,
            snap.as_mut_ptr() as u64,
            0,
            0,
            0,
        )
    };
    let snapshot = if rc == 0 {
        let mapped = unsafe { syscall2(SYS_SHARED_MAP, snap[0], 1) };
        destroy(snap[0]);
        mapped
    } else {
        rc
    };
    if snapshot <= 0 {
        unsafe {
            syscall6(SYS_SHARED_UNMAP, va as u64, 0, 0, 0, 0, 0);
        }
        destroy(region);
        return Err(snapshot);
    }
    // Distinct inherited function reference to the exact fresh region. Its
    // marker rights do not enlarge the session's provisioned function scope.
    // Child slot 4: diagnostics (Monitor) or the /Users/user grant.
    let (files_head, home) = file_grants(kind, document);
    let spec = [
        (CALL_SIDE, RIGHTS_WRITE),
        (region, RIGHTS_READ | RIGHTS_WRITE | RIGHTS_COPY),
        (region, function_rights),
        (clock(i), RIGHTS_READ | RIGHTS_WRITE),
        if diagnostics {
            (POOL, RIGHTS_READ)
        } else {
            (home, RIGHTS_WRITE | RIGHTS_COPY)
        },
    ];
    let pid = unsafe {
        syscall5(
            SYS_SPAWN,
            image,
            spec.as_ptr() as u64,
            if diagnostics || home != CAP_NONE {
                5
            } else {
                4
            },
            CLOCK,
            BADGE_EXIT,
        )
    };
    // The child holds its own copy; the broker never keeps the grant.
    destroy(home);
    if pid <= 0 {
        revoke_files(files_head);
        unsafe {
            syscall6(SYS_SHARED_UNMAP, va as u64, 0, 0, 0, 0, 0);
            syscall6(SYS_SHARED_UNMAP, snapshot as u64, 0, 0, 0, 0, 0);
        }
        destroy(region);
        return Err(pid);
    }
    let process = (0..CAP_SLOTS)
        .find(|s| {
            describe(*s)
                .is_some_and(|d| d[0] == 4 && d[1] == pid as u64 && d[2] & RIGHTS_DESTROY != 0)
        })
        .unwrap_or_else(|| die(83));
    sessions[i] = Session {
        region,
        id,
        va: va as u64,
        process,
        kind,
        scope,
        launch_targets,
        path,
        snapshot: snapshot as u64,
        files_head,
        ..EMPTY
    };
    transient_caps();
    log(b"[desktop] real application spawned; held Process and own region bound\n");
    Ok(())
}
/// The dock (or F-key) for `kind` first brings back a minimized window of
/// that kind; only when there is none does it launch a new instance.
fn restore_minimized(kind: u8) -> bool {
    let sessions = unsafe { &*(&raw const SESSIONS) };
    let state = unsafe { &mut *(&raw mut WM) };
    let pick = state.minimized_window(|backing| {
        sessions
            .iter()
            .any(|s| s.id == backing && s.kind == kind && s.handle != 0)
    });
    pick.is_some_and(|h| state.activate(h).is_ok())
}
fn retire(index: usize, force: bool) {
    let s = unsafe { SESSIONS[index] };
    if s.id == 0 {
        return;
    }
    if unsafe { syscall2(SYS_PROC_FINISH, s.process, u64::from(force)) } != 0 {
        die(84)
    }
    if s.handle != 0 {
        unsafe { (&mut *(&raw mut WM)).retire(s.handle) }.unwrap_or_else(|_| die(85));
    }
    if unsafe { syscall6(SYS_SHARED_UNMAP, s.va, 0, 0, 0, 0, 0) } != 0
        || unsafe { syscall6(SYS_SHARED_UNMAP, s.snapshot, 0, 0, 0, 0, 0) } != 0
    {
        die(86)
    }
    destroy(s.region);
    if unsafe { (*(&raw const CHOOSER)).as_ref() }.is_some_and(|c| c.session == index) {
        chooser_close();
    }
    destroy(s.grant);
    revoke_files(s.files_head);
    let _ = unsafe { syscall1(SYS_TRY_WAIT, clock(index)) };
    unsafe {
        SESSIONS[index] = EMPTY;
        WOKEN[index] = false;
    }
    log(b"[desktop] application retired: Process consumed; mapping and region released\n");
}
fn sweep() -> bool {
    let mut changed = false;
    let now = arena_desktop::app_client::now();
    for (i, s) in unsafe { *(&raw const SESSIONS) }.into_iter().enumerate() {
        if s.id != 0 && unsafe { syscall1(SYS_PROC_LIVE, s.process) } == 0 {
            if !s.ending && s.handle != 0 {
                unsafe {
                    SESSIONS[i].ending = true;
                    SESSIONS[i].reveal.retarget(
                        0,
                        now,
                        if PREFS.motion {
                            arena_ui::motion::CLOSE_US
                        } else {
                            0
                        },
                        arena_ui::motion::Easing::Smooth,
                    );
                }
                changed = true;
            } else if !s.reveal.active(now) {
                retire(i, false);
                changed = true;
            }
        }
    }
    changed
}
fn animate() -> bool {
    let now = arena_desktop::app_client::now();
    let state = unsafe { &*(&raw const WM) };
    let mut changed = false;
    if now / 1_000_000 != unsafe { UPTIME_SECOND } {
        unsafe { UPTIME_SECOND = now / 1_000_000 };
        changed = true;
    }
    if unsafe { NOTICE }.is_some_and(|(_, until)| until <= now) {
        unsafe { NOTICE = None };
        changed = true;
    }
    for s in unsafe { &mut *(&raw mut SESSIONS) }
        .iter_mut()
        .filter(|s| s.handle != 0)
    {
        let target = if state.focused() == Some(s.handle) {
            65536
        } else {
            0
        };
        if s.focus.target() != target {
            s.focus.retarget(
                target,
                now,
                if unsafe { PREFS.motion } {
                    arena_ui::motion::FOCUS_US
                } else {
                    0
                },
                arena_ui::motion::Easing::Smooth,
            );
            changed = true;
        }
        if !unsafe { PREFS.motion } {
            s.reveal
                .retarget(s.reveal.target(), now, 0, arena_ui::motion::Easing::Linear);
            s.focus
                .retarget(target, now, 0, arena_ui::motion::Easing::Linear);
        }
        let reveal = s.reveal.sample(now);
        let focus = s.focus.sample(now);
        changed |= reveal != s.reveal_last || focus != s.focus_last;
        s.reveal_last = reveal;
        s.focus_last = focus;
    }
    changed
}
static mut LAST_SCENE: Option<Scene> = None;
/// The presented scene now includes every published region.
fn clear_regions() {
    for s in unsafe { &mut *(&raw mut SESSIONS) } {
        s.regions = compose::Regions::NONE;
        s.popup.regions = compose::Regions::NONE;
    }
}
/// Descriptive snapshot of everything the compositor draws (see compose.rs).
fn scene(now: u64) -> Scene {
    let state = unsafe { &*(&raw const WM) };
    let sessions = unsafe { &*(&raw const SESSIONS) };
    let mut order = [0usize; LIMIT];
    let mut n = 0;
    for (i, s) in sessions.iter().enumerate() {
        // Minimized windows are not drawn (the dock shows them).
        if s.handle != 0 && state.find(s.handle).is_some_and(|w| !w.minimized) {
            order[n] = i;
            n += 1
        }
    }
    order[..n].sort_unstable_by_key(|i| state.find(sessions[*i].handle).map(|w| w.z).unwrap_or(0));
    let mut scene = Scene::EMPTY;
    for (k, index) in order[..n].iter().enumerate() {
        let s = sessions[*index];
        let window = state.find(s.handle).unwrap_or_else(|| die(88));
        scene.windows[k] = Some(WindowScene {
            slot: *index,
            handle: s.handle,
            x: window.x,
            y: window.y,
            width: window.width,
            height: window.height,
            reveal: s.reveal.sample(now).clamp(0, i32::from(window.height)),
            focus: s.focus.sample(now),
            content: s.content,
            regions: s.regions,
            published: s.published,
            title: s.title,
            surface: s.surface,
            popup: window.popup.map(|p| {
                let live = s.popup.handle == p.handle;
                PopupScene {
                    handle: p.handle,
                    x: p.x,
                    y: p.y,
                    width: p.width,
                    height: p.height,
                    content: if live { s.popup.content } else { 0 },
                    regions: if live {
                        s.popup.regions
                    } else {
                        compose::Regions::NONE
                    },
                    published: live && s.popup.published,
                }
            }),
            controls: arena_ui::components::Controls {
                minimize: true,
                maximize: window.resizable,
                maximized: window.restore.is_some() && window.placement == wm::Placement::Maximized,
                hover: match state.hover {
                    Some((h, b)) if h == s.handle => b as u8,
                    _ => 0,
                },
            },
        });
    }
    scene.count = n;
    let mut running = [0u8; 6];
    let mut minimized = [0u8; 6];
    let mut active = None;
    for s in sessions.iter().filter(|s| s.id != 0) {
        if s.kind < 6 {
            running[s.kind as usize] += 1;
            if state.find(s.handle).is_some_and(|w| w.minimized) {
                minimized[s.kind as usize] += 1;
            }
            if Some(s.handle) == state.focused() {
                active = Some(s.kind);
            }
        }
    }
    let switcher = state.switcher().map(|(selected, order, count)| {
        let mut sw = arena_desktop::shell::Switcher {
            count: count as u8,
            selected: selected as u8,
            titles: [[0; 32]; LIMIT],
            kinds: [6; LIMIT],
        };
        for (row, handle) in order[..count].iter().enumerate() {
            if let Some(s) = sessions.iter().find(|s| s.handle == *handle) {
                sw.titles[row] = s.title;
                sw.kinds[row] = s.kind.min(6);
            }
        }
        sw
    });
    scene.shell = Shell {
        pointer: state.pointer,
        open: state.windows().count(),
        focused_any: state.focused().is_some(),
        running,
        active,
        notice: unsafe { NOTICE }
            .filter(|(_, until)| *until > now)
            .map(|(text, _)| text),
        uptime: now / 1_000_000,
        minimized,
        switcher,
        snap: state.snap_preview().map(|(x, y, w, h)| arena_gfxkit::Rect {
            x,
            y,
            width: u32::from(w),
            height: u32::from(h),
        }),
        chooser: chooser_view(),
        desk: {
            desk_poll(now);
            unsafe { (*(&raw const DESK)).view }
        },
    };
    scene.dark = unsafe { PREFS.dark };
    scene
}
/// Recompose and present only the rectangles whose pixels changed since
/// the last presented scene. The scanout backing retains every other pixel.
fn render(ram: u64, w: usize, h: usize, scanout: u64) {
    let started = perf_now();
    let next = scene(arena_desktop::app_client::now());
    let mut damage = Damage::new(w, h);
    match unsafe { LAST_SCENE } {
        Some(prev) => compose::damage(&prev, &next, &mut damage),
        None => damage.full(),
    }
    if damage.is_empty() {
        unsafe { LAST_SCENE = Some(next) };
        clear_regions();
        probe(P_SKIPPED, started);
        return;
    }
    let pixels = unsafe { core::slice::from_raw_parts_mut(ram as *mut u32, w * h) };
    let mut c = Canvas::new(pixels, w, h, w).unwrap_or_else(|_| die(87));
    let sessions = unsafe { &*(&raw const SESSIONS) };
    let theme = arena_ui::theme::palette(next.dark);
    for rect in damage.rects() {
        c.set_clip(*rect);
        compose::compose(&mut c, &next, |slot| snapshot_of(&sessions[slot]), theme);
    }
    probe(P_COMPOSE, started);
    let phase = perf_now();
    for rect in damage.rects() {
        let frame = arena_compositor_model::wire::Frame::Present {
            x: rect.x,
            y: rect.y,
            w: rect.width as u16,
            h: rect.height as u16,
        };
        let (out, b) = display(frame, scanout);
        if out != [0, 0, CAP_NONE] || arena_compositor_model::wire::Frame::decode(&b) != Ok(frame) {
            die(90)
        }
    }
    unsafe { LAST_SCENE = Some(next) };
    clear_regions();
    probe(P_PRESENT, phase);
    probe(P_RENDER, started);
    if perf::ENABLED {
        unsafe {
            (*(&raw mut PERF))[P_KPIXELS].add(damage.pixels() / 1000);
            (*(&raw mut PERF))[P_RECTS].add(damage.rects().len() as u64);
        }
    }
}
/// The chooser requests (ADR-0077). Any live session with a window may ask
/// the user: the request itself grants nothing, the user decides, and the
/// grant lands in the session's own lineage. None: not a chooser frame.
fn choice(index: usize, bytes: &mut [u8; 64]) -> Option<Result<u64, i64>> {
    use arena_desktop::service_wire::Frame as S;
    match S::decode(bytes).ok()? {
        S::Choose {
            save,
            read_only,
            name,
        } => Some(chooser_open(index, save, read_only, name).map(|_| 0)),
        S::TakeGrant => {
            let s = unsafe { &mut *(&raw mut SESSIONS).cast::<Session>().add(index) };
            if !s.grant_ready {
                return Some(Err(-2));
            }
            s.grant_ready = false;
            let granted = core::mem::replace(&mut s.grant, CAP_NONE);
            let reply = if granted == CAP_NONE {
                S::TakeGrant
            } else {
                S::Granted {
                    save: s.grant_save,
                    read_only: s.grant_read_only,
                    name: s.grant_title,
                }
            };
            match reply.encode() {
                Ok(b) => {
                    *bytes = b;
                    unsafe { REPLY_CAP = granted };
                    Some(Ok(0))
                }
                Err(_) => {
                    destroy(granted);
                    Some(Err(-2))
                }
            }
        }
        _ => None,
    }
}
/// Function policy is based on held object/rights and provisioned scope.
/// Caller-supplied startup kind, name, PID and window handle grant nothing.
fn service(index: usize, rights: u64, bytes: &mut [u8; 64]) -> Result<u64, i64> {
    use arena_desktop::{
        scope::{self, Operation as O},
        service_wire::Frame as S,
    };
    let session = unsafe { SESSIONS[index] };
    let request = S::decode(bytes).map_err(|_| -2)?;
    if let Some(r) = choice(index, bytes) {
        return r;
    }
    let operation = match request {
        S::List { .. } => O::List,
        S::Read { .. } => O::Read,
        S::Put { .. } | S::Create { .. } => O::Put,
        S::Delete { .. } => O::Delete,
        S::Configure { .. } => O::Configure,
        S::Launch { .. } => O::Launch,
        _ => return Err(-2),
    };
    if !scope::permits(session.scope, rights, operation) {
        return Err(-2);
    }
    let fs = unsafe { (&mut *(&raw mut FILES)).as_mut() }.ok_or(STATUS_SERVICE_GONE)?;
    match request {
        S::List { mut cursor } => {
            for _ in 0..32 {
                let row = fs.list(cursor)?;
                if row.next == FS_CURSOR_END || scope::public_name(&row.name) {
                    *bytes = S::Entry {
                        cursor: row.next,
                        size: row.size,
                        name: row.name,
                    }
                    .encode()
                    .map_err(|_| -2)?;
                    return Ok(0);
                }
                if row.next <= cursor {
                    return Err(-2);
                }
                cursor = row.next;
            }
            Err(-2)
        }
        S::Read { name } => {
            if !scope::public_name(&name) {
                return Err(-2);
            }
            unsafe { fs.read(name, session.va as *mut u8, 4096) }.map(|n| n as u64)
        }
        S::Put { name, length } => {
            if !scope::public_name(&name) {
                return Err(-2);
            }
            unsafe { fs.put(name, session.va as *const u8, length as usize) }
                .map(|_| u64::from(length))
        }
        S::Create { name } => {
            if !scope::public_name(&name) {
                return Err(-2);
            }
            fs.create(name).map(|_| 0)
        }
        S::Delete { name } => {
            if !scope::public_name(&name) {
                return Err(-2);
            }
            fs.delete(name).map(|_| 0)
        }
        S::Configure { theme, motion } => {
            let p = arena_desktop::preferences::Preferences {
                dark: theme == 1,
                motion,
            };
            let record = p.encode();
            let mut name = [0u8; 32];
            name[..10].copy_from_slice(b"ui10-prefs");
            unsafe { fs.put(name, record.as_ptr(), record.len()) }?;
            unsafe {
                PREFS = p;
                WAKE_ALL = true;
            }
            Ok(0)
        }
        S::Launch { kind, path } => {
            if !scope::can_launch(session.launch_targets, kind) {
                return Err(-2);
            }
            // An offered file capability opens in the Editor only when it
            // belongs to the requesting session's own lineage.
            let offer = unsafe { core::mem::replace(&mut *(&raw mut OFFER), CAP_NONE) };
            let document = match unsafe { &*(&raw const AFS2) } {
                Some(f)
                    if offer != CAP_NONE
                        && kind == 2
                        && session.files_head != CAP_NONE
                        && f.same_lineage(session.files_head, offer) =>
                {
                    offer
                }
                _ => CAP_NONE,
            };
            let r = launch(kind, path, document).map(|_| 0);
            if offer != CAP_NONE {
                if let Some(f) = unsafe { &*(&raw const AFS2) } {
                    f.release(offer);
                } else {
                    destroy(offer);
                }
            }
            r
        }
        _ => Err(-2),
    }
}

/// A capability the current request's reply transfers (a copy; the
/// broker drops its own after replying).
static mut REPLY_CAP: u64 = CAP_NONE;
fn reply(status: u64, result: u64, bytes: &[u8; 64]) {
    let cap = unsafe { core::mem::replace(&mut *(&raw mut REPLY_CAP), CAP_NONE) };
    let rc = unsafe {
        syscall5(
            SYS_IPC_REPLY_CHECKED,
            SERVER,
            status,
            if status == 0 { result } else { 0 },
            if status == 0 { cap } else { CAP_NONE },
            bytes.as_ptr() as u64,
        )
    };
    destroy(cap);
    // A cancelled application call is ordinary liveness, not compositor death.
    // The checked operation consumes the exact abandoned-call tombstone.
    if rc != 0 && rc != STATUS_CALLER_GONE {
        die(91)
    }
}
arena_desktop::entry!(main, 64 * 1024);
extern "C" fn main() -> ! {
    let (mode, b) = display(arena_compositor_model::wire::Frame::Mode, CAP_NONE);
    let w = (mode[1] & 0xffff_ffff) as usize;
    let h = (mode[1] >> 32) as usize;
    let scanout = mode[2];
    let mut bound = [0u64; 2];
    if mode[0] != 0
        || arena_compositor_model::wire::Frame::decode(&b)
            != Ok(arena_compositor_model::wire::Frame::Mode)
        || unsafe {
            syscall6(
                SYS_SHARED_INFO,
                scanout,
                bound.as_mut_ptr() as u64,
                0,
                0,
                0,
                0,
            )
        } != 0
        || w * h * 4 > bound[1] as usize * 4096
    {
        die(92)
    }
    unsafe { (&mut *(&raw mut WM)).configure_screen(w as u16, h as u16, 6) }
        .unwrap_or_else(|_| die(93));
    let (max_w, max_h) = unsafe { (*(&raw const WM)).max_surface() };
    let surface_pages = (u64::from(max_w) * u64::from(max_h) * 4).div_ceil(4096);
    unsafe {
        RESERVE = Reserve {
            surface_pages,
            shared_pages: 1 + surface_pages + TRANSIENT_PAGES + FILE_PAGES,
            snapshot_pages: surface_pages + TRANSIENT_PAGES,
        };
    }
    log(b"[desktop] session reservation shared/snapshot pages=");
    log_number(unsafe { RESERVE.shared_pages });
    log(b"/");
    log_number(unsafe { RESERVE.snapshot_pages });
    log(b"\n");
    let ram = unsafe { syscall2(SYS_SHARED_MAP, scanout, 1) };
    if ram <= 0 {
        die(94)
    }
    let expected = describe(INPUT).unwrap_or_else(|| die(95));
    let mut fs = arena_desktop::fs_backend::Fs::start(13).unwrap_or_else(|_| die(79));
    let mut pref_name = [0u8; 32];
    pref_name[..10].copy_from_slice(b"ui10-prefs");
    let mut pref_bytes = [0u8; 16];
    if let Ok(n) = unsafe { fs.read(pref_name, pref_bytes.as_mut_ptr(), 16) } {
        if let Some(p) = arena_desktop::preferences::Preferences::decode(&pref_bytes[..n]) {
            unsafe {
                PREFS = p;
            }
        }
    }
    unsafe {
        FILES = Some(fs);
    }
    start_afs2();
    desk_load(w as i32, h as i32);

    // Requests and input arrive on SERVER; queue them onto CLOCK when the
    // compositor is not parked in RECV, so it never polls (ADR-0071).
    if unsafe { syscall6(SYS_ENDPOINT_BIND, SERVER, CLOCK, BADGE_REQUEST, 0, 0, 0) } != 0 {
        die(98)
    }
    render(ram as u64, w, h, scanout);
    log(b"[desktop] real desktop frame presented; gallery launcher available\n");
    snapshot();
    loop {
        let mut dirty = sweep() | animate();
        if unsafe { (*(&raw mut WM)).repeat_tick(arena_desktop::app_client::now()) } {
            wake_clients();
        }
        let mut request = [0, 0, CAP_NONE];
        let mut bytes = [0; 64];
        let rc = unsafe {
            syscall6(
                SYS_IPC_TRY_RECV,
                SERVER,
                request.as_mut_ptr() as u64,
                bytes.as_mut_ptr() as u64,
                0,
                0,
                0,
            )
        };
        if rc == STATUS_BUSY {
            if dirty {
                render(ram as u64, w, h, scanout);
                snapshot();
            }
            perf_report();
            let slept = perf_now();
            let now = arena_desktop::app_client::now();
            let due = next_deadline(now);
            let timer = if due == NO_DEADLINE {
                -1
            } else {
                let t = unsafe {
                    syscall3(
                        SYS_TIMER_ARM,
                        CLOCK,
                        BADGE_TIMER,
                        due.saturating_sub(now).max(1),
                    )
                };
                if t < 0 {
                    die(96)
                }
                t
            };
            if unsafe { syscall1(SYS_WAIT, CLOCK) } < 0 {
                die(96)
            }
            // An event (not the timer) woke us: retire the still-armed
            // timer so wakes never accumulate in the bounded table.
            if timer >= 0 {
                let _ = unsafe { syscall1(SYS_TIMER_CANCEL, timer as u64) };
            }
            probe(P_SLEEP, slept);
            continue;
        }
        if rc != 0 {
            die(97)
        }
        let received = perf_now();
        let mut input_request = false;
        let mut landed = request[2];
        let description = describe(landed);
        transient_caps();
        let mut status = 2;
        let mut result = 0;
        if request[0] == 0 && request[1] == 0 {
            if description
                .is_some_and(|d| d[0] == 10 && d[1] == expected[1] && d[2] & RIGHTS_READ != 0)
            {
                if let Ok(f) = input_wire::Frame::decode(&bytes) {
                    input_request = true;
                    // End the mutable policy borrow before lifecycle actions
                    // take their own access to the same static state.
                    let action = {
                        let state = unsafe { &mut *(&raw mut WM) };
                        state.set_time(arena_desktop::app_client::now());
                        let modal = unsafe { (*(&raw const CHOOSER)).is_some() };
                        match f {
                            // The trusted chooser is modal: presses go to it
                            // only; releases still reach the window policy so
                            // no key stays held (and repeats) behind it.
                            input_wire::Frame::Key {
                                code,
                                pressed: true,
                                ..
                            } if modal => {
                                chooser_key(code);
                                Action::Changed
                            }
                            input_wire::Frame::Key {
                                code,
                                pressed,
                                mods,
                            } => {
                                let a = state.key_input(code, pressed, mods);
                                // With no window focused, Enter, Delete and
                                // Esc act on the desktop's selected icons.
                                if pressed
                                    && state.focused().is_none()
                                    && matches!(code, 13 | 27 | 262)
                                    && let Some(mut store) = desk_store()
                                {
                                    let desk = unsafe { &mut *(&raw mut DESK) };
                                    desk_effect(desk.key(&mut store, code));
                                    Action::Changed
                                } else {
                                    a
                                }
                            }
                            input_wire::Frame::Pointer {
                                x,
                                y,
                                buttons,
                                wheel,
                            } => {
                                let (px, py) = (
                                    (u32::from(x) * (w as u32 - 1) / 32767) as i32,
                                    (u32::from(y) * (h as u32 - 1) / 32767) as i32,
                                );
                                if modal {
                                    chooser_pointer(px, py, buttons, w as i32, h as i32);
                                }
                                // The desktop surface takes presses on bare
                                // desktop and everything while it holds a
                                // press or its menu; the window policy then
                                // sees the pointer without buttons.
                                let desk = unsafe { &mut *(&raw mut DESK) };
                                let pressing = buttons & 3 != 0 && unsafe { DESK_BUTTONS } & 3 == 0;
                                unsafe { DESK_BUTTONS = buttons };
                                let to_desk = !modal
                                    && desk_store().is_some()
                                    && (desk.busy() || (pressing && state.bare(px, py)));
                                if to_desk && let Some(mut store) = desk_store() {
                                    if pressing && !desk.busy() {
                                        state.blur();
                                        desk.poll(&mut store);
                                    }
                                    let ctrl = state.mods & arena_desktop::model::MOD_CTRL != 0;
                                    let now = arena_desktop::app_client::now();
                                    let e = desk.pointer(&mut store, px, py, buttons, ctrl, now);
                                    desk_effect(e);
                                }
                                let a = state.pointer(
                                    px,
                                    py,
                                    if modal || to_desk { 0 } else { buttons },
                                );
                                let a = if modal || to_desk { Action::Changed } else { a };
                                if wheel != 0 {
                                    state.wheel(wheel);
                                }
                                a
                            }
                        }
                    };
                    match action {
                        Action::Launch(kind) if restore_minimized(kind as u8) => {
                            dirty = true;
                        }
                        Action::Launch(kind) => {
                            if launch(kind as u8, [0; 32], CAP_NONE).is_err() {
                                unsafe {
                                    NOTICE = Some((
                                        "LAUNCH REFUSED / DESKTOP CAPACITY",
                                        arena_desktop::app_client::now() + 3_000_000,
                                    ));
                                }
                                log(b"[desktop] launch refused at bounded capacity\n");
                            }
                            dirty = true;
                        }
                        Action::Close(handle) => {
                            if let Some(i) = unsafe { &*(&raw const SESSIONS) }
                                .iter()
                                .position(|s| s.handle == handle)
                            {
                                if unsafe { SESSIONS[i].close_pending } {
                                    retire(i, true)
                                } else {
                                    let _ = unsafe {
                                        (&mut *(&raw mut WM))
                                            .send(handle, arena_desktop::model::Event::Close)
                                    };
                                    unsafe {
                                        SESSIONS[i].close_pending = true;
                                    }
                                }
                            }
                            dirty = true;
                        }
                        Action::Changed => dirty = true,
                        Action::None => {}
                    }
                    status = 0;
                }
            } else if description.is_some_and(|d| d[0] == 1 && d[2] & RIGHTS_READ != 0)
                && arena_desktop::service_wire::Frame::decode(&bytes)
                    == Ok(arena_desktop::service_wire::Frame::LaunchImage)
            {
                match launch_image(
                    landed,
                    255,
                    0,
                    RIGHTS_READ | RIGHTS_COPY,
                    [0; 32],
                    false,
                    0,
                    CAP_NONE,
                ) {
                    Ok(()) => {
                        status = 0;
                        dirty = true;
                    }
                    Err(e) => status = e as u64,
                }
            } else if description
                .is_some_and(|d| d[0] == 12 && describe(USER_ROOT).is_some_and(|r| r[1] == d[1]))
                && arena_desktop::service_wire::Frame::decode(&bytes)
                    == Ok(arena_desktop::service_wire::Frame::Offer)
            {
                // Keep the offered filesd capability for the next launch; it
                // is used only for the session whose lineage holds it.
                let old = unsafe { core::mem::replace(&mut *(&raw mut OFFER), landed) };
                if old != CAP_NONE {
                    if let Some(f) = unsafe { &*(&raw const AFS2) } {
                        f.release(old);
                    } else {
                        destroy(old);
                    }
                }
                landed = CAP_NONE;
                status = 0;
            } else if let Some([7, id, rights]) = description {
                if let Some(i) = unsafe { &*(&raw const SESSIONS) }
                    .iter()
                    .position(|s| s.id == id && unsafe { syscall1(SYS_PROC_LIVE, s.process) } == 1)
                {
                    if rights & RIGHTS_DESTROY != 0 {
                        match service(i, rights, &mut bytes) {
                            Ok(value) => {
                                status = 0;
                                result = value;
                                dirty = true;
                            }
                            Err(error) => {
                                status = error as u64;
                            }
                        }
                    }
                }
                if rights & (RIGHTS_READ | RIGHTS_WRITE | RIGHTS_COPY)
                    == (RIGHTS_READ | RIGHTS_WRITE | RIGHTS_COPY)
                {
                    if let Some(i) = unsafe { &*(&raw const SESSIONS) }.iter().position(|s| {
                        s.id == id && unsafe { syscall1(SYS_PROC_LIVE, s.process) } == 1
                    }) {
                        if arena_desktop::service_wire::Frame::decode(&bytes)
                            == Ok(arena_desktop::service_wire::Frame::Bootstrap)
                        {
                            let s = unsafe { SESSIONS[i] };
                            let p = unsafe { PREFS };
                            if s.kind < 6 {
                                bytes = arena_desktop::service_wire::Frame::Started {
                                    kind: s.kind,
                                    theme: u8::from(p.dark),
                                    motion: p.motion,
                                    path: s.path,
                                }
                                .encode()
                                .unwrap_or_else(|_| die(78));
                                status = 0;
                            }
                        }
                        if arena_desktop::service_wire::Frame::decode(&bytes)
                            == Ok(arena_desktop::service_wire::Frame::Display)
                        {
                            result = w as u64 | ((h as u64) << 32);
                            status = 0;
                        }
                        // Ordinary graphical sessions (signed applications
                        // included) may ask the user through the chooser.
                        if rights & RIGHTS_DESTROY == 0 {
                            if let Some(r) = choice(i, &mut bytes) {
                                match r {
                                    Ok(v) => {
                                        status = 0;
                                        result = v;
                                        dirty = true;
                                    }
                                    Err(e) => status = e as u64,
                                }
                            }
                        }
                        if let Ok(f) = Frame::decode(&bytes) {
                            let state = unsafe { &mut *(&raw mut WM) };
                            let s = unsafe { &mut *(&raw mut SESSIONS).cast::<Session>().add(i) };
                            match f {
                                Frame::Create { width, height }
                                    if s.handle == 0
                                        && u64::from(width) * u64::from(height)
                                            <= unsafe { RESERVE.surface_pages } * 1024 =>
                                {
                                    if let Ok(handle) = state.create(id, width, height) {
                                        s.handle = handle;
                                        s.surface = (width, height);
                                        s.reveal.retarget(
                                            height as i32,
                                            arena_desktop::app_client::now(),
                                            if unsafe { PREFS.motion } {
                                                arena_ui::motion::OPEN_US
                                            } else {
                                                0
                                            },
                                            arena_ui::motion::Easing::Smooth,
                                        );
                                        result = handle;
                                        status = 0;
                                        dirty = true;
                                    }
                                }
                                Frame::Title { handle, text } if state.owned(id, handle) => {
                                    s.title = text;
                                    status = 0;
                                    dirty = true;
                                }
                                Frame::Damage { handle, rects } if state.owned(id, handle) => {
                                    let copy_started = perf_now();
                                    let (ww, wh) =
                                        (usize::from(s.surface.0), usize::from(s.surface.1));
                                    // Every declared rectangle must lie inside this
                                    // session's published surface; otherwise nothing
                                    // is published at all.
                                    let inside = rects.rects().iter().all(|&[x, y, w, h]| {
                                        x as usize + w as usize <= ww
                                            && y as usize + h as usize <= wh
                                    });
                                    if inside {
                                        let full = [[0, 0, ww as u16, wh as u16]];
                                        let list = if rects.n == 0 {
                                            &full[..]
                                        } else {
                                            rects.rects()
                                        };
                                        publish_rects(
                                            s.va + PIXEL_OFFSET as u64,
                                            s.snapshot,
                                            ww,
                                            list,
                                        );
                                        if rects.n == 0 || !s.published {
                                            s.regions.mark_full();
                                        } else {
                                            for r in rects.rects() {
                                                s.regions.add(*r);
                                            }
                                        }
                                        s.published = true;
                                        s.content = s.content.wrapping_add(1);
                                        probe(P_DAMAGE, copy_started);
                                        status = 0;
                                        dirty = true;
                                    }
                                }
                                // Publication of the session's own transient surface.
                                Frame::Damage { handle, rects }
                                    if state.popup_owned(id, handle)
                                        && s.popup.handle == handle =>
                                {
                                    let (_, p) =
                                        state.find_popup(handle).unwrap_or_else(|| die(88));
                                    let (pw, ph) = (usize::from(p.width), usize::from(p.height));
                                    let inside = rects.rects().iter().all(|&[x, y, w, h]| {
                                        x as usize + w as usize <= pw
                                            && y as usize + h as usize <= ph
                                    });
                                    if inside {
                                        let reserve = unsafe { RESERVE };
                                        let full = [[0, 0, pw as u16, ph as u16]];
                                        let list = if rects.n == 0 {
                                            &full[..]
                                        } else {
                                            rects.rects()
                                        };
                                        publish_rects(
                                            s.va + (reserve.shared_pages
                                                - TRANSIENT_PAGES
                                                - FILE_PAGES)
                                                * 4096,
                                            s.snapshot + reserve.surface_pages * 4096,
                                            pw,
                                            list,
                                        );
                                        if rects.n == 0 || !s.popup.published {
                                            s.popup.regions.mark_full();
                                        } else {
                                            for r in rects.rects() {
                                                s.popup.regions.add(*r);
                                            }
                                        }
                                        s.popup.published = true;
                                        s.popup.content = s.popup.content.wrapping_add(1);
                                        status = 0;
                                        dirty = true;
                                    }
                                }
                                // The client publishes its whole surface at exactly
                                // the size the window policy configured.
                                Frame::Resize {
                                    handle,
                                    width,
                                    height,
                                } if state.owned(id, handle) => {
                                    let window = state.find(handle).unwrap_or_else(|| die(88));
                                    if (width, height) == (window.width, window.height) {
                                        publish_rects(
                                            s.va + PIXEL_OFFSET as u64,
                                            s.snapshot,
                                            usize::from(width),
                                            &[[0, 0, width, height]],
                                        );
                                        s.surface = (width, height);
                                        s.regions.mark_full();
                                        s.published = true;
                                        s.content = s.content.wrapping_add(1);
                                        status = 0;
                                        dirty = true;
                                    }
                                }
                                Frame::Resizable {
                                    handle,
                                    min_width,
                                    min_height,
                                } if state.owned(id, handle) => {
                                    if state.set_resizable(handle, min_width, min_height).is_ok() {
                                        status = 0;
                                    }
                                }
                                Frame::Popup {
                                    handle,
                                    kind,
                                    x,
                                    y,
                                    width,
                                    height,
                                } if state.owned(id, handle) => {
                                    if let Ok(popup) =
                                        state.open_popup(id, handle, kind, x, y, width, height)
                                    {
                                        s.popup = PopupState {
                                            handle: popup,
                                            ..NO_POPUP
                                        };
                                        result = popup;
                                        status = 0;
                                        dirty = true;
                                    }
                                }
                                Frame::Dismiss { handle } if state.popup_owned(id, handle) => {
                                    if state.close_popup(id, handle).is_ok() {
                                        s.popup = NO_POPUP;
                                        status = 0;
                                        dirty = true;
                                    }
                                }
                                Frame::Poll { handle } if state.owned(id, handle) => {
                                    unsafe { WOKEN[i] = false };
                                    if let Ok(event) = state.poll(handle) {
                                        result = u64::from(unsafe { PREFS.dark })
                                            | (u64::from(unsafe { PREFS.motion }) << 1);
                                        if let Some(event) = event {
                                            bytes = Frame::Event { handle, event }
                                                .encode()
                                                .unwrap_or_else(|_| die(98));
                                        }
                                        status = 0;
                                    }
                                }
                                Frame::CancelClose { handle } if state.owned(id, handle) => {
                                    s.close_pending = false;
                                    status = 0;
                                }
                                _ => {}
                            }
                        }
                    }
                }
            }
        }
        destroy(landed);
        reply(status, result, &bytes);
        deliver_chosen();
        wake_clients();
        probe(P_REQUEST, received);
        if dirty {
            render(ram as u64, w, h, scanout)
        }
        if input_request {
            probe(P_INPUT, received);
        }
        if (dirty && (bytes[5] == 1 || description.is_some_and(|d| d[0] == 10)))
            || (bytes[5] == 10 && description.is_some_and(|d| d[0] == 1))
        {
            snapshot();
        }
    }
}
