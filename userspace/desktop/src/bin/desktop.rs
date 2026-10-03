//! Authority/lifecycle service. Presentation belongs in desktop_view.rs.
#![no_std]
#![no_main]
#![allow(clippy::deref_addrof, clippy::collapsible_if)]
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
#[path = "../desktop_view.rs"]
mod view;
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
    kind: u8,
    scope: u8,
    path: [u8; 32],
    close_pending: bool,
    ending: bool,
    reveal: arena_ui::motion::Motion,
    focus: arena_ui::motion::Motion,
    reveal_last: i32,
    focus_last: i32,
}
const EMPTY: Session = Session {
    region: CAP_NONE,
    id: 0,
    va: 0,
    process: CAP_NONE,
    handle: 0,
    title: [0; 32],
    kind: 5,
    scope: 0,
    path: [0; 32],
    close_pending: false,
    ending: false,
    reveal: arena_ui::motion::Motion::fixed(arena_ui::metrics::TITLE_HEIGHT),
    focus: arena_ui::motion::Motion::fixed(0),
    reveal_last: arena_ui::metrics::TITLE_HEIGHT,
    focus_last: 0,
};
static mut PREFS: arena_desktop::preferences::Preferences =
    arena_desktop::preferences::Preferences {
        dark: false,
        motion: true,
    };
static mut FILES: Option<arena_desktop::fs_backend::Fs> = None;
static mut NOTICE: Option<(&'static str, u64)> = None;
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
    launch_image(APPLICATION, kind, scope, function_rights, path, kind == 4)
}
fn launch_image(
    image: u64,
    kind: u8,
    scope: u8,
    function_rights: u64,
    path: [u8; 32],
    diagnostics: bool,
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
            CAP_NONE,
            0,
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
        path,
        ..EMPTY
    };
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
fn render(ram: u64, w: usize, h: usize, scanout: u64) {
    let pixels = unsafe { core::slice::from_raw_parts_mut(ram as *mut u32, w * h) };
    let mut c = Canvas::new(pixels, w, h, w).unwrap_or_else(|_| die(87));
    let state = unsafe { &*(&raw const WM) };
    view::background(&mut c, arena_ui::theme::palette(unsafe { PREFS.dark }));
    let mut order = [0usize; LIMIT];
    let mut n = 0;
    for (i, s) in unsafe { &*(&raw const SESSIONS) }.iter().enumerate() {
        if s.handle != 0 {
            order[n] = i;
            n += 1
        }
    }
    order[..n].sort_unstable_by_key(|i| {
        state
            .find(unsafe { SESSIONS[*i].handle })
            .map(|w| w.z)
            .unwrap_or(0)
    });
    for index in &order[..n] {
        let s = unsafe { SESSIONS[*index] };
        let window = state.find(s.handle).unwrap_or_else(|| die(88));
        let source = unsafe {
            core::slice::from_raw_parts(
                (s.va as usize + PIXEL_OFFSET) as *const u32,
                window.width as usize * window.height as usize,
            )
        };
        let reveal = s
            .reveal
            .sample(arena_desktop::app_client::now())
            .clamp(0, window.height as i32) as usize;
        if reveal == 0 {
            continue;
        }
        c.blit(
            source,
            window.width as usize,
            reveal,
            window.width as usize,
            window.x,
            window.y,
        )
        .unwrap_or_else(|_| die(89));
        let end = s.title.iter().position(|b| *b == 0).unwrap_or(32);
        let title = core::str::from_utf8(&s.title[..end]).unwrap_or("Application");
        view::chrome(
            &mut c,
            window,
            title,
            state.focused() == Some(s.handle),
            s.focus.sample(arena_desktop::app_client::now()),
            arena_ui::theme::palette(unsafe { PREFS.dark }),
        );
    }
    let mut running = [0u8; 6];
    let mut active = None;
    for s in unsafe { &*(&raw const SESSIONS) }
        .iter()
        .filter(|s| s.id != 0)
    {
        if s.kind < 6 {
            running[s.kind as usize] += 1;
            if Some(s.handle) == state.focused() {
                active = Some(s.kind);
            }
        }
    }
    let notice = unsafe { NOTICE }
        .filter(|(_, until)| *until > arena_desktop::app_client::now())
        .map(|(text, _)| text);
    view::system(
        &mut c,
        state,
        &running,
        active,
        notice,
        arena_ui::theme::palette(unsafe { PREFS.dark }),
    );
    let frame = arena_compositor_model::wire::Frame::Present {
        x: 0,
        y: 0,
        w: w as u16,
        h: h as u16,
    };
    let (out, b) = display(frame, scanout);
    if out != [0, 0, CAP_NONE] || arena_compositor_model::wire::Frame::decode(&b) != Ok(frame) {
        die(90)
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
        S::Put { .. } => O::Put,
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
            }
            Ok(0)
        }
        S::Launch { kind, path } => launch(kind, path).map(|_| 0),
        _ => Err(-2),
    }
}

fn reply(status: u64, result: u64, bytes: &[u8; 64]) {
    if unsafe {
        syscall5(
            SYS_IPC_REPLY,
            SERVER,
            status,
            result,
            CAP_NONE,
            bytes.as_ptr() as u64,
        )
    } != 0
    {
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
            if unsafe { syscall3(SYS_TIMER_ARM, CLOCK, 1, arena_ui::motion::FRAME_US) } < 0
                || unsafe { syscall1(SYS_WAIT, CLOCK) } < 0
            {
                die(96)
            }
            continue;
        }
        if rc != 0 {
            die(97)
        }
        let landed = request[2];
        let description = describe(landed);
        let mut status = 2;
        let mut result = 0;
        if request[0] == 0 && request[1] == 0 {
            if description
                .is_some_and(|d| d[0] == 10 && d[1] == expected[1] && d[2] & RIGHTS_READ != 0)
            {
                if let Ok(f) = input_wire::Frame::decode(&bytes) {
                    let state = unsafe { &mut *(&raw mut WM) };
                    match f {
                        input_wire::Frame::Key {
                            code,
                            pressed: true,
                        } => {
                            state.key(code);
                        }
                        input_wire::Frame::Key { .. } => {}
                        input_wire::Frame::Pointer { x, y, buttons } => {
                            match state.pointer(
                                (u32::from(x) * (w as u32 - 1) / 32767) as i32,
                                (u32::from(y) * (h as u32 - 1) / 32767) as i32,
                                buttons,
                            ) {
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
                                }
                                Action::Close(handle) => {
                                    if let Some(i) = unsafe { &*(&raw const SESSIONS) }
                                        .iter()
                                        .position(|s| s.handle == handle)
                                    {
                                        if unsafe { SESSIONS[i].close_pending } {
                                            retire(i, true)
                                        } else {
                                            unsafe { SESSIONS[i].close_pending = true };
                                            state.send(handle, arena_desktop::model::Event::Close);
                                        }
                                    }
                                }
                                _ => {}
                            }
                            dirty = true;
                        }
                    }
                    status = 0;
                }
            } else if description.is_some_and(|d| d[0] == 1 && d[2] & RIGHTS_READ != 0)
                && arena_desktop::service_wire::Frame::decode(&bytes)
                    == Ok(arena_desktop::service_wire::Frame::LaunchImage)
            {
                match launch_image(landed, 255, 0, RIGHTS_READ | RIGHTS_COPY, [0; 32], false) {
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
                                    status = 0;
                                    dirty = true;
                                }
                                Frame::Poll { handle } if state.owned(id, handle) => {
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
        if dirty {
            render(ram as u64, w, h, scanout)
        }
        if (dirty && (bytes[5] == 1 || description.is_some_and(|d| d[0] == 10)))
            || (bytes[5] == 10 && description.is_some_and(|d| d[0] == 1))
        {
            snapshot();
        }
    }
}
