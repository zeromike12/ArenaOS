//! Source prototype: ring-3 compositor authority bridge, not boot-linked yet.
//! Root must bind the two held Process caps and provisioned SharedRegions.
#![no_std]
#![no_main]
// Receiver authentication, backing bounds and model state are visibly
// separate gates; keep them nested rather than merging authorization checks.
#![allow(clippy::collapsible_if)]
use arena_compositor_model::render::{self, Layer};
use arena_compositor_model::{
    Key, State,
    wire::{self, Frame},
};
use core::panic::PanicInfo;
#[path = "../userspace/abi.rs"]
mod abi;
use abi::*;
const DISPLAY: u64 = 0;
const CLIENT: u64 = 1;
const INPUT_PROOF: u64 = 2;
const WORKER_A: u64 = 3;
const WORKER_B: u64 = 4;
const BACKING_A: u64 = 5;
const BACKING_B: u64 = 6;
const CAP_KIND_REGION: u64 = 7;
const CAP_KIND_TOKEN: u64 = 10;
const REFUSED: u64 = 2;
const MAX_PIXELS: usize = 1024 * 768;
const NONE: Surface = Surface {
    id: 0,
    cap: CAP_NONE,
    va: 0,
    width: 0,
    height: 0,
    x: 0,
    y: 0,
    z: 0,
    handle: 0,
};
#[derive(Clone, Copy)]
struct Surface {
    id: u64,
    cap: u64,
    va: u64,
    width: usize,
    height: usize,
    x: i32,
    y: i32,
    z: u64,
    handle: u64,
}
static mut BASE: [u32; MAX_PIXELS] = [0; MAX_PIXELS];
fn die(code: u64) -> ! {
    unsafe {
        let _ = syscall1(SYS_THREAD_EXIT, code);
    }
    loop {
        core::hint::spin_loop()
    }
}
#[panic_handler]
fn panic(_: &PanicInfo<'_>) -> ! {
    die(99)
}
fn log(msg: &[u8]) {
    unsafe {
        let _ = syscall2(SYS_DEBUG_WRITE, msg.as_ptr() as u64, msg.len() as u64);
    }
}
fn describe(slot: u64) -> Option<[u64; 3]> {
    let mut out = [0u64; 3];
    (slot != CAP_NONE && unsafe { syscall2(SYS_CAP_DESCRIBE, slot, out.as_mut_ptr() as u64) } == 0)
        .then_some(out)
}
fn info(slot: u64) -> Option<[u64; 2]> {
    let mut out = [0u64; 2];
    (unsafe { syscall6(SYS_SHARED_INFO, slot, out.as_mut_ptr() as u64, 0, 0, 0, 0) } == 0)
        .then_some(out)
}
fn destroy(slot: u64) {
    if slot != CAP_NONE && unsafe { syscall1(SYS_CAP_DESTROY, slot) } != 0 {
        die(80)
    }
}
fn unmap(va: u64) {
    if va != 0 && unsafe { syscall6(SYS_SHARED_UNMAP, va, 0, 0, 0, 0, 0) } != 0 {
        die(81)
    }
}
fn ipc_call(frame: Frame, cap: u64) -> ([u64; 3], [u8; wire::BYTES]) {
    let mut bytes = [0u8; wire::BYTES];
    frame.encode(&mut bytes).unwrap_or_else(|_| die(82));
    let mut out = [0u64; 3];
    if unsafe {
        syscall6(
            SYS_IPC_CALL,
            DISPLAY,
            0,
            0,
            cap,
            out.as_mut_ptr() as u64,
            bytes.as_mut_ptr() as u64,
        )
    } != 0
    {
        die(83)
    }
    (out, bytes)
}
fn present(scanout: u64, w: usize, h: usize) {
    let frame = Frame::Present {
        x: 0,
        y: 0,
        w: w as u16,
        h: h as u16,
    };
    let (out, bytes) = ipc_call(frame, scanout);
    if out != [0, 0, CAP_NONE] || Frame::decode(&bytes) != Ok(frame) {
        die(84)
    }
}
fn reply(status: u64, result: u64, echo: Frame) {
    let mut inline = [0u8; wire::BYTES];
    echo.encode(&mut inline).unwrap_or_else(|_| die(85));
    if unsafe {
        syscall5(
            SYS_IPC_REPLY,
            CLIENT,
            status,
            result,
            CAP_NONE,
            inline.as_ptr() as u64,
        )
    } != 0
    {
        die(86)
    }
}
fn draw(
    base: *const u32,
    scanout: u64,
    ram: u64,
    w: usize,
    h: usize,
    surfaces: &[Surface; render::MAX_LAYERS],
) {
    let mut layers = [Layer {
        pixels: core::ptr::null(),
        pixel_len: 0,
        width: 0,
        height: 0,
        x: 0,
        y: 0,
        z: 0,
    }; render::MAX_LAYERS];
    let mut n = 0;
    for surface in surfaces.iter().filter(|s| s.id != 0) {
        layers[n] = Layer {
            pixels: surface.va as *const u32,
            pixel_len: surface.width * surface.height,
            width: surface.width,
            height: surface.height,
            x: surface.x,
            y: surface.y,
            z: surface.z,
        };
        n += 1;
    }
    // SAFETY: pinned mappings verified and owned by this process, disjoint
    // from immutable BASE and writable scanout. Only root-bound clients run.
    let base = unsafe { core::slice::from_raw_parts(base, w * h) };
    if unsafe { render::compose(base, ram as *mut u32, w * h, w, h, &layers[..n]) }.is_err() {
        die(87)
    }
    present(scanout, w, h);
}
fn region(slot: u64, scanout_id: u64) -> Option<(u64, u64)> {
    let [kind, id, rights] = describe(slot)?;
    let [bound_id, pages] = info(slot)?;
    if kind != CAP_KIND_REGION
        || id == scanout_id
        || id != bound_id
        || pages == 0
        || (rights & (RIGHTS_READ | RIGHTS_WRITE | RIGHTS_COPY))
            != (RIGHTS_READ | RIGHTS_WRITE | RIGHTS_COPY)
    {
        return None;
    }
    Some((id, pages))
}
fn owner_slot(id: u64) -> Option<u64> {
    if describe(BACKING_A)?.get(1).copied() == Some(id) {
        Some(WORKER_A)
    } else if describe(BACKING_B)?.get(1).copied() == Some(id) {
        Some(WORKER_B)
    } else {
        None
    }
}
fn owned(surfaces: &[Surface; render::MAX_LAYERS], id: u64, handle: u64) -> Option<usize> {
    surfaces
        .iter()
        .position(|s| s.id != 0 && s.id == id && s.handle == handle)
}
#[unsafe(no_mangle)]
pub extern "C" fn _start() -> ! {
    let (mode, bytes) = ipc_call(Frame::Mode, CAP_NONE);
    let w = (mode[1] & 0xffff_ffff) as usize;
    let h = (mode[1] >> 32) as usize;
    let scanout = mode[2];
    let [kind, id, rights] = describe(scanout).unwrap_or_else(|| die(88));
    let [bound_id, pages] = info(scanout).unwrap_or_else(|| die(89));
    if mode[0] != 0
        || Frame::decode(&bytes) != Ok(Frame::Mode)
        || kind != CAP_KIND_REGION
        || id != bound_id
        || pages == 0
        || (rights & (RIGHTS_READ | RIGHTS_WRITE | RIGHTS_COPY))
            != (RIGHTS_READ | RIGHTS_WRITE | RIGHTS_COPY)
        || w == 0
        || w > render::MAX_SCREEN_WIDTH
        || h == 0
        || h > render::MAX_SCREEN_HEIGHT
        || w.checked_mul(h)
            .and_then(|v| v.checked_mul(4))
            .is_none_or(|v| v > pages as usize * 4096)
    {
        die(90)
    }
    let ram = unsafe { syscall2(SYS_SHARED_MAP, scanout, 1) };
    if ram <= 0 {
        die(91)
    }
    let ram = ram as u64;
    let base = (&raw mut BASE).cast::<u32>();
    for i in 0..w * h {
        unsafe {
            core::ptr::write(
                base.add(i),
                core::ptr::read_volatile((ram as *const u32).add(i)),
            )
        }
    }
    let [token_kind, token_id, token_rights] = describe(INPUT_PROOF).unwrap_or_else(|| die(92));
    if token_kind != CAP_KIND_TOKEN || token_id == 0 || token_rights & RIGHTS_READ == 0 {
        die(93)
    }
    let mut state = State::new();
    let mut surfaces = [NONE; render::MAX_LAYERS];
    let mut next_z = 1u64;
    log(b"[compositord] MODE cap pinned; typed user compositor serving\n");
    loop {
        let mut request = [0u64; 3];
        let mut payload = [0u8; wire::BYTES];
        if unsafe {
            syscall3(
                SYS_IPC_RECV,
                CLIENT,
                request.as_mut_ptr() as u64,
                payload.as_mut_ptr() as u64,
            )
        } < 0
        {
            die(94)
        }
        let transferred = request[2];
        let frame = Frame::decode(&payload);
        let mut status = REFUSED;
        let mut result = 0;
        let mut echo = Frame::Mode;
        let mut retained = false;
        if request[0] == 0 && request[1] == 0 {
            match frame {
                Ok(Frame::Create { x, y, w: sw, h: sh }) => {
                    if let Some((source_id, pages)) = region(transferred, id) {
                        if owner_slot(source_id).is_some()
                            && (sw as usize)
                                .checked_mul(sh as usize)
                                .and_then(|v| v.checked_mul(4))
                                .is_some_and(|v| v <= pages as usize * 4096)
                            && surfaces.iter().all(|s| s.id != source_id)
                        {
                            if let Some(index) = surfaces.iter().position(|s| s.id == 0) {
                                let map = unsafe { syscall2(SYS_SHARED_MAP, transferred, 0) };
                                if map > 0 {
                                    if state.register_owner(source_id).is_ok() {
                                        if let Ok(handle) =
                                            state.create(source_id, source_id as u32, sw, sh, x, y)
                                        {
                                            surfaces[index] = Surface {
                                                id: source_id,
                                                cap: transferred,
                                                va: map as u64,
                                                width: sw as usize,
                                                height: sh as usize,
                                                x,
                                                y,
                                                z: next_z,
                                                handle,
                                            };
                                            next_z =
                                                next_z.checked_add(1).unwrap_or_else(|| die(95));
                                            retained = true;
                                            draw(base, scanout, ram, w, h, &surfaces);
                                            status = 0;
                                            result = handle;
                                            echo = Frame::Create { x, y, w: sw, h: sh };
                                        } else {
                                            let _ = state.retire_owner(source_id);
                                            unmap(map as u64);
                                        }
                                    } else {
                                        unmap(map as u64)
                                    }
                                }
                            }
                        }
                    }
                }
                Ok(Frame::Move { handle, x, y }) => {
                    if let Some((source_id, _)) = region(transferred, id) {
                        if let Some(index) = owned(&surfaces, source_id, handle) {
                            if state
                                .move_to(source_id, source_id as u32, handle, x, y)
                                .is_ok()
                            {
                                surfaces[index].x = x;
                                surfaces[index].y = y;
                                draw(base, scanout, ram, w, h, &surfaces);
                                status = 0;
                                echo = Frame::Move { handle, x, y };
                            }
                        }
                    }
                }
                Ok(Frame::Damage {
                    handle,
                    x,
                    y,
                    w: dw,
                    h: dh,
                }) => {
                    if let Some((source_id, _)) = region(transferred, id) {
                        if let Some(index) = owned(&surfaces, source_id, handle) {
                            let surface = surfaces[index];
                            if x >= 0
                                && y >= 0
                                && x as u64 + dw as u64 <= surface.width as u64
                                && y as u64 + dh as u64 <= surface.height as u64
                            {
                                draw(base, scanout, ram, w, h, &surfaces);
                                status = 0;
                                echo = Frame::Damage {
                                    handle,
                                    x,
                                    y,
                                    w: dw,
                                    h: dh,
                                };
                            }
                        }
                    }
                }
                Ok(Frame::Focus { handle }) => {
                    if let Some((source_id, _)) = region(transferred, id) {
                        if let Some(index) = owned(&surfaces, source_id, handle) {
                            if state.focus(source_id, source_id as u32, handle).is_ok()
                                && state.raise(source_id, source_id as u32, handle).is_ok()
                            {
                                surfaces[index].z = next_z;
                                next_z = next_z.checked_add(1).unwrap_or_else(|| die(96));
                                draw(base, scanout, ram, w, h, &surfaces);
                                status = 0;
                                echo = Frame::Focus { handle };
                            }
                        }
                    }
                }
                Ok(Frame::Destroy { handle }) => {
                    if let Some((source_id, _)) = region(transferred, id) {
                        if let Some(index) = owned(&surfaces, source_id, handle) {
                            if state.destroy(source_id, source_id as u32, handle).is_ok() {
                                let old = surfaces[index];
                                surfaces[index] = NONE;
                                unmap(old.va);
                                destroy(old.cap);
                                if state.retire_owner(source_id).is_err() {
                                    die(97)
                                }
                                draw(base, scanout, ram, w, h, &surfaces);
                                status = 0;
                                echo = Frame::Destroy { handle };
                            }
                        }
                    }
                }
                Ok(Frame::Key { ascii, pressed }) => {
                    if let Some([kind, token, rights]) = describe(transferred) {
                        if kind == CAP_KIND_TOKEN
                            && token == token_id
                            && rights & RIGHTS_READ != 0
                            && state.route_verified_key(Key { ascii, pressed }).is_ok()
                        {
                            status = 0;
                            echo = Frame::Key { ascii, pressed };
                            log(b"[compositord] receiver-verified input routed to focus\n");
                        }
                    }
                }
                _ => {}
            }
        }
        if !retained {
            destroy(transferred)
        }
        reply(status, result, echo);
    }
}
