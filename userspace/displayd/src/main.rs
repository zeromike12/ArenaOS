//! Phase-9 GOP fallback display service. All framebuffer stores run at CPL3.
//! Boot-granted slot 0 is the *only* Mmio window: immutable GOP geometry is
//! disclosed through possession-gated DISPLAY_INFO, then self-mapped RW/NX.
//! An endpoint exists for the forthcoming typed compositor protocol; until
//! it is implemented, every call fails closed and landed caps are destroyed.
#![no_std]
#![no_main]

use core::panic::PanicInfo;
#[path = "../../abi.rs"]
mod abi;
use abi::*;

const SLOT_GOP: u64 = 0;
const SLOT_EP: u64 = 1;
const SLOT_POOL: u64 = 2;
const SLOT_DMA: u64 = 3;
const BOOT_PATTERN: u64 = 0x5048_3901;

fn write_log(bytes: &[u8]) {
    unsafe {
        let _ = syscall2(SYS_DEBUG_WRITE, bytes.as_ptr() as u64, bytes.len() as u64);
    }
}
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

/// Pure arithmetic, shared with unit tests. The lower 24 bits represent
/// native RGB values, independently converted to the GOP's observed mode.
fn pattern(x: usize, y: usize, w: usize, h: usize) -> u32 {
    if y < h / 8 {
        return 0x00_22_33_55;
    }
    if x < w / 3 {
        0x00_e3_35_42
    } else if x < w * 2 / 3 {
        0x00_2e_c7_71
    } else {
        0x00_3b_67_e1
    }
}
fn encode(rgb: u32, fmt: u64) -> u32 {
    if fmt == 0 {
        (rgb & 0xff) << 16 | (rgb & 0x00ff00) | ((rgb >> 16) & 0xff)
    } else {
        rgb
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn _start() -> ! {
    let mut mode = [0u64; 5];
    if unsafe { syscall2(SYS_DISPLAY_INFO, SLOT_GOP, mode.as_mut_ptr() as u64) } != 0 {
        write_log(b"[displayd] no exact GOP cap or mode\n");
        exit(81)
    }
    let [w, h, pitch, fmt, span] = mode;
    if w == 0
        || h == 0
        || w > 1024
        || h > 768
        || pitch < w
        || pitch > 1024
        || fmt > 1
        || pitch
            .checked_mul(h)
            .and_then(|v| v.checked_mul(4))
            .is_none_or(|v| v > span)
    {
        exit(82)
    }
    // Stable-boundary guest probe: this is real ring-3 memory, not an
    // allocator model. The writer loses all caps; two mappings keep its
    // zeroed physical run alive while a read-only copy cannot write-map.
    let mut created = [0u64; 3];
    if unsafe { syscall3(SYS_SHARED_CREATE, SLOT_POOL, 4, created.as_mut_ptr() as u64) } != 0 {
        exit(86)
    }
    let shared = created[0];
    if shared != 4 || created[1] == 0 || created[2] != 4 * 4096 {
        exit(87)
    }
    let mut backing = [0u64; 3];
    if unsafe {
        syscall3(
            SYS_SHARED_PHYS,
            shared,
            SLOT_EP,
            backing.as_mut_ptr() as u64,
        )
    } != -2
        || unsafe { syscall1(SYS_CAP_PHYS, shared) } != -2
        || unsafe {
            syscall3(
                SYS_SHARED_PHYS,
                shared,
                SLOT_DMA,
                backing.as_mut_ptr() as u64,
            )
        } != 0
        || backing[0] == 0
        || backing[1] != 4
        || backing[2] != created[1]
    {
        exit(88)
    }
    let rw = unsafe { syscall2(SYS_SHARED_MAP, shared, 1) };
    if rw <= 0 {
        exit(89)
    }
    for i in [0, 4095, 4096, 12288, 16383] {
        if unsafe { core::ptr::read_volatile((rw as *const u8).add(i)) } != 0 {
            exit(90)
        }
    }
    unsafe { core::ptr::write_volatile((rw as *mut u8).add(12288), 0xa7) }
    if unsafe { syscall3(SYS_CAP_COPY, shared, 5, RIGHTS_READ | RIGHTS_DESTROY) } != 0
        || unsafe { syscall2(SYS_SHARED_MAP, 5, 1) } != -2
    {
        exit(91)
    }
    let ro = unsafe { syscall2(SYS_SHARED_MAP, 5, 0) };
    if ro <= 0 || unsafe { core::ptr::read_volatile((ro as *const u8).add(12288)) } != 0xa7 {
        exit(92)
    }
    if unsafe { syscall1(SYS_CAP_DESTROY, shared) } != 0
        || unsafe { syscall1(SYS_CAP_DESTROY, 5) } != 0
        || unsafe { core::ptr::read_volatile((ro as *const u8).add(12288)) } != 0xa7
    {
        exit(93)
    }
    write_log(b"[displayd] SharedRegion guest authority/zero/copy/mapping PASS\n");

    let va = unsafe { syscall2(SYS_MAP_MEMORY, SLOT_GOP, 1) };
    if va <= 0 {
        exit(83)
    }
    let fb = va as *mut u32;
    for y in 0..h as usize {
        for x in 0..w as usize {
            let color = encode(pattern(x, y, w as usize, h as usize), fmt);
            unsafe { core::ptr::write_volatile(fb.add(y * pitch as usize + x), color) }
        }
    }
    write_log(b"[displayd] ring3 GOP pixels ready (pattern v1)\n");
    // No compositor or client exists yet: do not interpret input as a
    // framebuffer command. The reserved endpoint remains an owned service
    // capability and all unrecognized traffic receives a typed refusal.
    let mut msg = [0u64; 3];
    loop {
        let r = unsafe { syscall3(SYS_IPC_RECV, SLOT_EP, msg.as_mut_ptr() as u64, 0) };
        if r < 0 {
            exit(84)
        }
        if msg[2] != CAP_NONE {
            if unsafe { syscall1(SYS_CAP_DESTROY, msg[2]) } != 0 {
                exit(85)
            }
        }
        let _ = unsafe { syscall5(SYS_IPC_REPLY, SLOT_EP, BOOT_PATTERN, 0, CAP_NONE, 0) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pattern_bounded_and_channel_order() {
        assert_eq!(pattern(0, 0, 800, 600), 0x00223355);
        assert_eq!(pattern(0, 100, 800, 600), 0x00e33542);
        assert_eq!(pattern(400, 100, 800, 600), 0x002ec771);
        assert_eq!(pattern(799, 599, 800, 600), 0x003b67e1);
        assert_eq!(encode(0x00e33542, 0), 0x004235e3);
    }
}
