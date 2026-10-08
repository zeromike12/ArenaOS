//! Short-lived, independent ring-3 caller for the canonical GRAPHICS v1
//! MODE/PRESENT display bridge. No MMIO, DMA bearer or physical address.
//! Boot-root grants only Endpoint/WRITE and MemoryPool/WRITE (for a wrong-
//! region negative); service cap possession is rechecked by the receiver.
#![no_std]
#![no_main]
use arena_compositor_model::wire::{self, Frame};
use core::panic::PanicInfo;
#[path = "../../abi.rs"]
mod abi;
use abi::*;

const EP: u64 = 0;
const POOL: u64 = 1;
const COLOR: u32 = 0x00_a2_51_f4;

fn exit(code: u64) -> ! {
    unsafe {
        let _ = syscall1(SYS_THREAD_EXIT, code);
    }
    loop {
        core::hint::spin_loop()
    }
}
#[panic_handler]
fn panic(_: &PanicInfo<'_>) -> ! {
    exit(99)
}

fn call(frame: Frame, cap: u64) -> Result<([u64; 3], [u8; wire::BYTES]), u64> {
    let mut payload = [0u8; wire::BYTES];
    frame.encode(&mut payload).map_err(|_| 80u64)?;
    let mut answer = [0u64; 3];
    let r = unsafe {
        syscall6(
            SYS_IPC_CALL,
            EP,
            0,
            0,
            cap,
            answer.as_mut_ptr() as u64,
            payload.as_mut_ptr() as u64,
        )
    };
    if r != 0 {
        return Err(81);
    }
    if Frame::decode(&payload).is_err() {
        return Err(82);
    }
    Ok((answer, payload))
}
fn reject(frame: Frame, cap: u64) {
    let Ok((result, wire)) = call(frame, cap) else {
        exit(83)
    };
    if result != [2, 0, CAP_NONE] || Frame::decode(&wire) != Ok(Frame::Mode) {
        exit(84)
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn _start() -> ! {
    reject(
        Frame::Present {
            x: 0,
            y: 0,
            w: 1,
            h: 1,
        },
        CAP_NONE,
    );
    let (mode, response) = call(Frame::Mode, CAP_NONE).unwrap_or_else(|code| exit(code));
    let width = (mode[1] & 0xffff_ffff) as usize;
    let height = (mode[1] >> 32) as usize;
    if mode[0] != 0
        || !(320..=1024).contains(&width)
        || !(240..=768).contains(&height)
        || mode[2] == CAP_NONE
        || Frame::decode(&response) != Ok(Frame::Mode)
    {
        exit(85)
    }
    let target_x = width - 100;
    let target_y = height / 2;
    let source = mode[2];
    let mut cap = [0u64; 3];
    let mut bound = [0u64; 2];
    let mut denied = [0xfeed_u64; 3];
    if unsafe { syscall2(SYS_CAP_DESCRIBE, source, cap.as_mut_ptr() as u64) } != 0
        || cap[0] != 7
        || cap[2] & (RIGHTS_READ | RIGHTS_WRITE | RIGHTS_COPY)
            != (RIGHTS_READ | RIGHTS_WRITE | RIGHTS_COPY)
        || unsafe {
            syscall6(
                SYS_SHARED_INFO,
                source,
                bound.as_mut_ptr() as u64,
                0,
                0,
                0,
                0,
            )
        } != 0
        || bound[0] != cap[1]
        || bound[1] != (width * height * 4).div_ceil(4096) as u64
        || unsafe { syscall3(SYS_SHARED_PHYS, source, 0, denied.as_mut_ptr() as u64) } != -2
        || denied != [0xfeed_u64; 3]
    {
        exit(86)
    }
    let ram = unsafe { syscall2(SYS_SHARED_MAP, source, 1) };
    if ram <= 0 {
        exit(87)
    }
    // This client can derive dimensions from its *held* cap, but no
    // numerical region ID could ever grant PRESENT by itself.
    for _ in 0..40 {
        reject(
            Frame::Present {
                x: 0,
                y: 0,
                w: 1,
                h: 1,
            },
            CAP_NONE,
        );
    }
    if unsafe {
        syscall3(
            SYS_CAP_COPY,
            source,
            3,
            RIGHTS_READ | RIGHTS_COPY | RIGHTS_DESTROY,
        )
    } != 0
    {
        exit(88)
    }
    for _ in 0..40 {
        reject(
            Frame::Present {
                x: target_x as i32,
                y: target_y as i32,
                w: 1,
                h: 1,
            },
            3,
        );
    }
    // A valid capability cannot authorize *invalid geometry*.
    reject(
        Frame::Present {
            x: width as i32 - 1,
            y: height as i32 - 1,
            w: 2,
            h: 2,
        },
        source,
    );
    // Another region, although correctly minted and held, is not the
    // display's owned scanout generation.
    let mut wrong = [0u64; 3];
    if unsafe { syscall3(SYS_SHARED_CREATE, POOL, 1, wrong.as_mut_ptr() as u64) } != 0 {
        exit(89)
    }
    reject(
        Frame::Present {
            x: target_x as i32,
            y: target_y as i32,
            w: 1,
            h: 1,
        },
        wrong[0],
    );
    if unsafe { syscall1(SYS_CAP_DESTROY, wrong[0]) } != 0 {
        exit(90)
    }

    let at = target_y * width + target_x;
    unsafe {
        core::ptr::write_volatile((ram as *mut u32).add(at), COLOR);
    }
    let frame = Frame::Present {
        x: target_x as i32,
        y: target_y as i32,
        w: 1,
        h: 1,
    };
    let (present, echoed) = call(frame, source).unwrap_or_else(|code| exit(code));
    if present != [0, 0, CAP_NONE] || Frame::decode(&echoed) != Ok(frame) {
        exit(91)
    }
    if unsafe { syscall6(SYS_SHARED_UNMAP, ram as u64, 0, 0, 0, 0, 0) } != 0
        || unsafe { syscall1(SYS_CAP_DESTROY, 3) } != 0
        || unsafe { syscall1(SYS_CAP_DESTROY, source) } != 0
    {
        exit(92)
    }
    let message = b"[displayprobe] ring3 MODE/cap-refusal/PRESENT 80x drain PASS\n";
    unsafe {
        let _ = syscall2(
            SYS_DEBUG_WRITE,
            message.as_ptr() as u64,
            message.len() as u64,
        );
    }
    exit(42)
}
