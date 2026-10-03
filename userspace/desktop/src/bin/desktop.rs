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
const GALLERY: u64 = 6;
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
}
const EMPTY: Session = Session {
    region: CAP_NONE,
    id: 0,
    va: 0,
    process: CAP_NONE,
    handle: 0,
    title: [0; 32],
};
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
fn launch() -> Result<(), i64> {
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
    let va = unsafe { syscall2(SYS_SHARED_MAP, region, 0) };
    if va <= 0 {
        destroy(region);
        return Err(va);
    }
    let spec = [
        (CALL_SIDE, RIGHTS_WRITE),
        (region, RIGHTS_READ | RIGHTS_WRITE | RIGHTS_COPY),
    ];
    let pid = unsafe { syscall5(SYS_SPAWN, GALLERY, spec.as_ptr() as u64, 2, CAP_NONE, 0) };
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
    unsafe {
        SESSIONS[index] = EMPTY;
    }
    log(b"[desktop] application retired: Process consumed; mapping and region released\n");
}
fn sweep() -> bool {
    let mut changed = false;
    for (i, s) in unsafe { *(&raw const SESSIONS) }.into_iter().enumerate() {
        if s.id != 0 && unsafe { syscall1(SYS_PROC_LIVE, s.process) } == 0 {
            retire(i, false);
            changed = true
        }
    }
    changed
}
fn render(ram: u64, w: usize, h: usize, scanout: u64) {
    let pixels = unsafe { core::slice::from_raw_parts_mut(ram as *mut u32, w * h) };
    let mut c = Canvas::new(pixels, w, h, w).unwrap_or_else(|_| die(87));
    let state = unsafe { &*(&raw const WM) };
    view::background(&mut c, arena_ui::theme::LIGHT);
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
        c.blit(
            source,
            window.width as usize,
            window.height as usize,
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
            arena_ui::theme::LIGHT,
        );
    }
    view::system(&mut c, state, arena_ui::theme::LIGHT);
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
    unsafe { (&mut *(&raw mut WM)).configure_screen(w as u16, h as u16, 1) }
        .unwrap_or_else(|_| die(93));
    let ram = unsafe { syscall2(SYS_SHARED_MAP, scanout, 1) };
    if ram <= 0 {
        die(94)
    }
    let expected = describe(INPUT).unwrap_or_else(|| die(95));
    render(ram as u64, w, h, scanout);
    log(b"[desktop] real desktop frame presented; gallery launcher available\n");
    loop {
        let mut dirty = sweep();
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
                render(ram as u64, w, h, scanout)
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
                                Action::Launch(0) => {
                                    if launch().is_err() {
                                        log(b"[desktop] launch refused at bounded capacity\n");
                                    }
                                }
                                Action::Close(handle) => {
                                    if let Some(i) = unsafe { &*(&raw const SESSIONS) }
                                        .iter()
                                        .position(|s| s.handle == handle)
                                    {
                                        retire(i, true)
                                    }
                                }
                                _ => {}
                            }
                            dirty = true;
                        }
                    }
                    status = 0;
                }
            } else if let Some([7, id, rights]) = description {
                if rights & (RIGHTS_READ | RIGHTS_WRITE | RIGHTS_COPY)
                    == (RIGHTS_READ | RIGHTS_WRITE | RIGHTS_COPY)
                {
                    if let Some(i) = unsafe { &*(&raw const SESSIONS) }.iter().position(|s| {
                        s.id == id && unsafe { syscall1(SYS_PROC_LIVE, s.process) } == 1
                    }) {
                        if let Ok(f) = Frame::decode(&bytes) {
                            let state = unsafe { &mut *(&raw mut WM) };
                            let s = unsafe { &mut *(&raw mut SESSIONS).cast::<Session>().add(i) };
                            match f {
                                Frame::Create { width, height } if s.handle == 0 => {
                                    if let Ok(handle) = state.create(id, width, height) {
                                        s.handle = handle;
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
                                        if let Some(event) = event {
                                            bytes = Frame::Event { handle, event }
                                                .encode()
                                                .unwrap_or_else(|_| die(98));
                                        }
                                        status = 0;
                                    }
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
    }
}
