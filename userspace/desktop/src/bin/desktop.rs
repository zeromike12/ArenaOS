//! Authority/lifecycle service. Presentation belongs in shell.rs and compose.rs.
#![no_std]
#![no_main]
#![allow(clippy::deref_addrof, clippy::collapsible_if)]
use arena_desktop::{
    compose::{self, Damage, Scene, WindowScene},
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
const PIXEL_OFFSET: usize = 4096;
const PAGES: u64 = 127;
const LIMIT: usize = 6;
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
};
static mut PREFS: arena_desktop::preferences::Preferences =
    arena_desktop::preferences::Preferences {
        dark: false,
        motion: true,
    };
static mut FILES: Option<arena_desktop::fs_backend::Fs> = None;
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
        if (all || state.pending(s.handle)) && unsafe { syscall2(SYS_NOTIFY, 7 + i as u64, 1) } == 0
        {
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
// Private broker memory: clients never map or receive these complete frames.
// Only an authenticated Damage request publishes an owned snapshot.
const FRAME_PIXELS: usize = arena_ui::metrics::WINDOW_WIDTH * arena_ui::metrics::WINDOW_HEIGHT;
static mut PUBLISHED: [[u32; FRAME_PIXELS]; LIMIT] = [[0; FRAME_PIXELS]; LIMIT];
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
fn launch(kind: u8, path: [u8; 32]) -> Result<(), i64> {
    use arena_desktop::scope;
    if kind > 5 || (path[0] != 0 && !scope::public_name(&path)) {
        return Err(-2);
    }
    let (scope, function_rights) = match kind {
        0 | 1 => (
            scope::FILE_READ | scope::FILE_WRITE | scope::LAUNCH,
            RIGHTS_READ | RIGHTS_WRITE | RIGHTS_COPY | RIGHTS_DESTROY,
        ),
        2 => (
            scope::FILE_READ | scope::FILE_WRITE,
            RIGHTS_READ | RIGHTS_WRITE | RIGHTS_COPY | RIGHTS_DESTROY,
        ),
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
    )
}
fn launch_image(
    image: u64,
    kind: u8,
    scope: u8,
    function_rights: u64,
    path: [u8; 32],
    diagnostics: bool,
    launch_targets: u8,
) -> Result<(), i64> {
    let ready = unsafe { syscall6(SYS_SPAWN_CHECK, image, 0, 0, 0, 0, 0) };
    if ready != 0 {
        return Err(ready);
    }
    // The session table reserves original lifecycle owners, including children
    // that have not yet requested their window. No numerical caller identity.
    let sessions = unsafe { &mut *(&raw mut SESSIONS) };
    let i = sessions.iter().position(|s| s.id == 0).ok_or(STATUS_BUSY)?;
    let free = (0..32).filter(|s| describe(*s).is_none()).count();
    if free < 3 {
        return Err(STATUS_BUSY);
    }
    let mut out = [0; 3];
    let rc = unsafe {
        syscall6(
            SYS_SHARED_CREATE,
            POOL,
            PAGES,
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
    // Distinct inherited function reference to the exact fresh region. Its
    // marker rights do not enlarge the session's provisioned function scope.
    let spec = [
        (CALL_SIDE, RIGHTS_WRITE),
        (region, RIGHTS_READ | RIGHTS_WRITE | RIGHTS_COPY),
        (region, function_rights),
        (7 + i as u64, RIGHTS_READ | RIGHTS_WRITE),
        (POOL, RIGHTS_READ),
    ];
    let pid = unsafe {
        syscall5(
            SYS_SPAWN,
            image,
            spec.as_ptr() as u64,
            if diagnostics { 5 } else { 4 },
            CLOCK,
            BADGE_EXIT,
        )
    };
    if pid <= 0 {
        unsafe {
            syscall6(SYS_SHARED_UNMAP, va as u64, 0, 0, 0, 0, 0);
        }
        destroy(region);
        return Err(pid);
    }
    let process = (0..32)
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
        ..EMPTY
    };
    transient_caps();
    log(b"[desktop] real application spawned; held Process and own region bound\n");
    Ok(())
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
    if unsafe { syscall6(SYS_SHARED_UNMAP, s.va, 0, 0, 0, 0, 0) } != 0 {
        die(86)
    }
    destroy(s.region);
    let _ = unsafe { syscall1(SYS_TRY_WAIT, 7 + index as u64) };
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
/// Descriptive snapshot of everything the compositor draws (see compose.rs).
fn scene(now: u64) -> Scene {
    let state = unsafe { &*(&raw const WM) };
    let sessions = unsafe { &*(&raw const SESSIONS) };
    let mut order = [0usize; LIMIT];
    let mut n = 0;
    for (i, s) in sessions.iter().enumerate() {
        if s.handle != 0 {
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
            published: s.published,
            title: s.title,
        });
    }
    scene.count = n;
    let mut running = [0u8; 6];
    let mut active = None;
    for s in sessions.iter().filter(|s| s.id != 0) {
        if s.kind < 6 {
            running[s.kind as usize] += 1;
            if Some(s.handle) == state.focused() {
                active = Some(s.kind);
            }
        }
    }
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
        probe(P_SKIPPED, started);
        return;
    }
    let pixels = unsafe { core::slice::from_raw_parts_mut(ram as *mut u32, w * h) };
    let mut c = Canvas::new(pixels, w, h, w).unwrap_or_else(|_| die(87));
    let published = unsafe { &*(&raw const PUBLISHED) };
    let theme = arena_ui::theme::palette(next.dark);
    for rect in damage.rects() {
        c.set_clip(*rect);
        compose::compose(&mut c, &next, |slot| &published[slot][..], theme);
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
    probe(P_PRESENT, phase);
    probe(P_RENDER, started);
    if perf::ENABLED {
        unsafe {
            (*(&raw mut PERF))[P_KPIXELS].add(damage.pixels() / 1000);
            (*(&raw mut PERF))[P_RECTS].add(damage.rects().len() as u64);
        }
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
            launch(kind, path).map(|_| 0)
        }
        _ => Err(-2),
    }
}

fn reply(status: u64, result: u64, bytes: &[u8; 64]) {
    let rc = unsafe {
        syscall5(
            SYS_IPC_REPLY_CHECKED,
            SERVER,
            status,
            result,
            CAP_NONE,
            bytes.as_ptr() as u64,
        )
    };
    // A cancelled application call is ordinary liveness, not compositor death.
    // The checked operation consumes the exact abandoned-call tombstone.
    if rc != 0 && rc != STATUS_CALLER_GONE {
        die(91)
    }
}
#[unsafe(no_mangle)]
pub extern "C" fn _start() -> ! {
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
                    syscall3(SYS_TIMER_ARM, CLOCK, BADGE_TIMER, due.saturating_sub(now).max(1))
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
        let landed = request[2];
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
                        match f {
                            input_wire::Frame::Key {
                                code,
                                pressed: true,
                            } => state.keyboard_action(code),
                            input_wire::Frame::Key { .. } => Action::None,
                            input_wire::Frame::Pointer { x, y, buttons } => state.pointer(
                                (u32::from(x) * (w as u32 - 1) / 32767) as i32,
                                (u32::from(y) * (h as u32 - 1) / 32767) as i32,
                                buttons,
                            ),
                        }
                    };
                    match action {
                        Action::Launch(kind) => {
                            if launch(kind as u8, [0; 32]).is_err() {
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
                match launch_image(landed, 255, 0, RIGHTS_READ | RIGHTS_COPY, [0; 32], false, 0) {
                    Ok(()) => {
                        status = 0;
                        dirty = true;
                    }
                    Err(e) => status = e as u64,
                }
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
                        if let Ok(f) = Frame::decode(&bytes) {
                            let state = unsafe { &mut *(&raw mut WM) };
                            let s = unsafe { &mut *(&raw mut SESSIONS).cast::<Session>().add(i) };
                            match f {
                                Frame::Create { width, height } if s.handle == 0 => {
                                    if let Ok(handle) = state.create(id, width, height) {
                                        s.handle = handle;
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
                                Frame::Damage { handle } if state.owned(id, handle) => {
                                    let copy_started = perf_now();
                                    let window = state.find(handle).unwrap_or_else(|| die(88));
                                    let pixels = window.width as usize * window.height as usize;
                                    let input = (s.va as usize + PIXEL_OFFSET) as *const u32;
                                    // Sender is blocked in CALL on this single-core launch
                                    // topology. Read volatile shared data into private memory;
                                    // later writes cannot change movement/focus frames.
                                    for pixel in 0..pixels {
                                        unsafe {
                                            (*(&raw mut PUBLISHED))[i][pixel] =
                                                core::ptr::read_volatile(input.add(pixel));
                                        }
                                    }
                                    s.published = true;
                                    s.content = s.content.wrapping_add(1);
                                    probe(P_DAMAGE, copy_started);
                                    status = 0;
                                    dirty = true;
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
