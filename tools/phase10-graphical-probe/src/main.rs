//! Signed <=4096-byte ordinary dynamic graphical fixture. Not a boot image.
#![no_std]
#![no_main]
#[path = "../../../userspace/abi.rs"]
mod abi;
use abi::*;
#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    done(99)
}
fn done(code: u64) -> ! {
    unsafe { syscall1(SYS_THREAD_EXIT, code) };
    loop {
        core::hint::spin_loop()
    }
}
#[inline(never)]
fn call(bytes: &mut [u8; 64], cap: u64) -> [u64; 3] {
    let mut out = [0, 0, CAP_NONE];
    if unsafe {
        syscall6(
            SYS_IPC_CALL,
            0,
            0,
            0,
            cap,
            out.as_mut_ptr() as u64,
            bytes.as_mut_ptr() as u64,
        )
    } != 0
        || out[2] != CAP_NONE
    {
        done(80)
    }
    out
}
#[inline(never)]
fn frame(op: u8, handle: u64) -> [u8; 64] {
    let mut b = [0u8; 64];
    b[0] = b'A';
    b[1] = b'D';
    b[2] = b'S';
    b[3] = b'K';
    b[4] = 1;
    b[5] = op;
    // Fixed in-bounds wire span; avoids linking generic panic formatting
    // into this intentionally size-constrained signed fixture.
    unsafe { core::ptr::write_unaligned(b.as_mut_ptr().add(8).cast::<u64>(), handle) };
    b
}
#[inline(never)]
fn paint(va: u64, flip: bool) {
    for y in 0..60 {
        for x in 0..80 {
            let value = if (x / 10 + y / 10) % 2 == 0 {
                0xff31_5278
            } else {
                0xffce_c5b4
            };
            unsafe {
                core::ptr::write_volatile(
                    (va as *mut u32).add(y * 80 + x),
                    value ^ if flip { 0x00ff_ffff } else { 0 },
                )
            }
        }
    }
}
#[unsafe(no_mangle)]
pub extern "C" fn _start() -> ! {
    let mut own = [0u64; 3];
    if unsafe { syscall2(SYS_CAP_DESCRIBE, 1, own.as_mut_ptr() as u64) } != 0
        || own[0] != 7
        || own[2] != 7
    {
        done(81)
    }
    for slot in 4..32 {
        if unsafe { syscall2(SYS_CAP_DESCRIBE, slot, own.as_mut_ptr() as u64) } == 0 {
            done(82)
        }
    }
    let va = unsafe { syscall2(SYS_SHARED_MAP, 1, 1) };
    if va <= 0 {
        done(83)
    }
    let mut create = frame(1, 0);
    create[24] = 80;
    create[26] = 60;
    let out = call(&mut create, 1);
    if out[0] != 0 || out[1] == 0 {
        done(84)
    }
    let handle = out[1];
    let mut title = frame(4, handle);
    unsafe {
        core::ptr::copy_nonoverlapping(b"Signed Image".as_ptr(), title.as_mut_ptr().add(32), 12)
    };
    if call(&mut title, 1)[0] != 0 {
        done(85)
    }
    // Selector forgery and a numerical handle do not grant another surface.
    if call(&mut frame(2, handle + 1000), 1)[0] == 0 {
        done(86)
    }
    if handle > 1 && call(&mut frame(2, handle - 1), 1)[0] == 0 {
        done(92)
    }
    let mut forbidden = [0u8; 64];
    forbidden[0] = b'A';
    forbidden[1] = b'S';
    forbidden[2] = b'V';
    forbidden[3] = b'C';
    forbidden[4] = 1;
    forbidden[5] = 8;
    forbidden[7] = 1;
    if call(&mut forbidden, 2)[0] == 0 {
        done(87)
    }
    let mut flip = false;
    paint(va as u64 + 4096, flip);
    if call(&mut frame(2, handle), 1)[0] != 0 {
        done(88)
    }
    loop {
        let mut event = frame(3, handle);
        if call(&mut event, 1)[0] != 0 {
            done(89)
        }
        if event[5] == 5 {
            match event[28] {
                3 => done(42),
                1 => {
                    let key = event[24];
                    if key != b'p' {
                        flip = !flip;
                        paint(va as u64 + 4096, flip);
                    }
                    // f deliberately stages an unpublished drawing. p publishes
                    // those exact prior bytes without painting anything new.
                    if key != b'f' && call(&mut frame(2, handle), 1)[0] != 0 {
                        done(90)
                    }
                }
                _ => {}
            }
        }
        if unsafe { syscall3(SYS_TIMER_ARM, 3, 1, 20_000) } < 0
            || unsafe { syscall1(SYS_WAIT, 3) } != 1
        {
            done(91)
        }
    }
}
