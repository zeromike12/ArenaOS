//! Independently linked ring-3 client with its own cap and bitmap surface.
use arena_compositor_model::wire::{self, Frame};
use arena_gfxkit::Canvas;
#[path = "../userspace/abi.rs"]
mod abi;
use abi::*;
const EP: u64 = 0;
const REGION: u64 = 1;
const WIDTH: usize = 160;
const HEIGHT: usize = 120;
const PAGES: u64 = (WIDTH * HEIGHT * 4).div_ceil(4096) as u64;

pub fn die(code: u64) -> ! {
    unsafe {
        let _ = syscall1(SYS_THREAD_EXIT, code);
    }
    loop {
        core::hint::spin_loop();
    }
}
fn log(s: &[u8]) {
    unsafe {
        let _ = syscall2(SYS_DEBUG_WRITE, s.as_ptr() as u64, s.len() as u64);
    }
}
fn call(frame: Frame, cap: u64) -> ([u64; 3], Frame) {
    let mut inline = [0u8; wire::BYTES];
    frame.encode(&mut inline).unwrap_or_else(|_| die(80));
    let mut out = [0u64; 3];
    if unsafe {
        syscall6(
            SYS_IPC_CALL,
            EP,
            0,
            0,
            cap,
            out.as_mut_ptr() as u64,
            inline.as_mut_ptr() as u64,
        )
    } != 0
    {
        die(81)
    }
    let echo = Frame::decode(&inline).unwrap_or_else(|_| die(82));
    if out[2] != CAP_NONE {
        // Never retain a reply cap that this protocol does not expect.
        let _ = unsafe { syscall1(SYS_CAP_DESTROY, out[2]) };
        die(83)
    }
    (out, echo)
}

pub fn run(name: &'static [u8], color: u32, title: &'static str, x: i32, y: i32, focus: bool) -> ! {
    // Root provisioned this exact bounded backing to this child and a
    // matching independent READ witness to the compositor. No pool grant.
    let cap = REGION;
    let mut desc = [0u64; 3];
    let mut bound = [0u64; 2];
    if unsafe { syscall2(SYS_CAP_DESCRIBE, cap, desc.as_mut_ptr() as u64) } != 0
        || desc[0] != 7
        || desc[1] == 0
        || desc[2] & (RIGHTS_READ | RIGHTS_WRITE | RIGHTS_COPY)
            != (RIGHTS_READ | RIGHTS_WRITE | RIGHTS_COPY)
        || unsafe { syscall6(SYS_SHARED_INFO, cap, bound.as_mut_ptr() as u64, 0, 0, 0, 0) } != 0
        || bound != [desc[1], PAGES]
    {
        die(85)
    }
    let mapped = unsafe { syscall2(SYS_SHARED_MAP, cap, 1) };
    if mapped <= 0 {
        die(86)
    }
    let ptr = mapped as *mut u32;
    let data = unsafe { core::slice::from_raw_parts_mut(ptr, WIDTH * HEIGHT) };
    {
        let mut canvas = Canvas::new(data, WIDTH, HEIGHT, WIDTH).unwrap_or_else(|_| die(87));
        canvas.clear(color);
        canvas
            .text(12, 10, title, 0x00_f8_ee_cc)
            .unwrap_or_else(|_| die(88));
    }
    let create = Frame::Create {
        x,
        y,
        w: WIDTH as u16,
        h: HEIGHT as u16,
    };
    let (out, echo) = call(create, cap);
    if out[0] != 0 || out[1] == 0 || echo != create {
        die(89)
    }
    let handle = out[1];
    let damage = Frame::Damage {
        handle,
        x: 0,
        y: 0,
        w: WIDTH as u16,
        h: HEIGHT as u16,
    };
    if call(damage, cap).0[0] != 0 {
        die(90)
    }
    if focus {
        let frame = Frame::Focus { handle };
        let (answer, echo) = call(frame, cap);
        if answer[0] != 0 || echo != frame {
            die(91)
        }
    }
    log(name);
    if !focus {
        // The nonfocused independent client has no key authority. 40 cap-
        // bearing forged requests must refuse without leaking landed slots.
        for _ in 0..40 {
            let (answer, _) = call(
                Frame::Key {
                    ascii: b'q',
                    pressed: true,
                },
                cap,
            );
            if answer[0] != 2 {
                die(92)
            }
        }
        log(b"[window_a] forged input token refused\n");
    }
    loop {
        let request = Frame::Poll { handle };
        let (answer, echo) = call(request, cap);
        if answer[0] != 0 || echo != request {
            die(93)
        }
        if focus && answer[1] == (u64::from(b'x') | (1 << 8)) {
            // Deliberately exit without DESTROY: root and compositor must
            // retire an original child, not just a polite surface request.
            log(b"[window_b] original child exiting without DESTROY\n");
            die(42)
        }
        if focus && answer[1] == (u64::from(b'q') | (1 << 8)) {
            // Unique pixel on the actual client's shared backing. The
            // compositor cannot manufacture this through a serial marker.
            unsafe { core::ptr::write_volatile(ptr.add(40 * WIDTH + 20), 0x00_ff_bb_11) };
            let region = Frame::Damage {
                handle,
                x: 20,
                y: 40,
                w: 1,
                h: 1,
            };
            if call(region, cap).0[0] != 0 {
                die(94)
            }
            log(b"[window_b] real key pixel painted\n");
        }
    }
}
