//! Real ring-3 bounded SharedRegion capacity and process-teardown probe.
//! Runs as a short-lived kernel boot fixture, never as an app or service.
#![no_std]
#![no_main]
use core::panic::PanicInfo;
#[path = "../../abi.rs"]
mod abi;
use abi::*;

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

#[unsafe(no_mangle)]
pub extern "C" fn _start() -> ! {
    // Pool/WRITE is the only inherited cap. No GPU, framebuffer, DMA,
    // Power, process or image authority enters this guest.
    let mut out = [0u64; 3];
    if unsafe { syscall3(SYS_SHARED_CREATE, 0, 512, out.as_mut_ptr() as u64) } != 0
        || out[0] != 1
        || out[1] == 0
        || out[2] != 2 * 1024 * 1024
    {
        exit(81)
    }
    let original = out[0];
    let first_id = out[1];
    if unsafe { syscall1(SYS_CAP_PHYS, original) } != -2
        || unsafe { syscall3(SYS_SHARED_PHYS, original, 0, out.as_mut_ptr() as u64) } != -2
    {
        exit(82)
    }
    let va = unsafe { syscall2(SYS_SHARED_MAP, original, 1) };
    if va <= 0 {
        exit(83)
    }
    for i in [0, 4096, 1024 * 1024, 2 * 1024 * 1024 - 1] {
        if unsafe { core::ptr::read_volatile((va as *const u8).add(i)) } != 0 {
            exit(84)
        }
    }
    unsafe { core::ptr::write_volatile((va as *mut u8).add(2 * 1024 * 1024 - 1), 0x7d) }
    if unsafe { syscall3(SYS_CAP_COPY, original, 2, RIGHTS_READ | RIGHTS_DESTROY) } != 0
        || unsafe { syscall2(SYS_SHARED_MAP, 2, 1) } != -2
    {
        exit(85)
    }
    let ro = unsafe { syscall2(SYS_SHARED_MAP, 2, 0) };
    if ro <= 0
        || unsafe { core::ptr::read_volatile((ro as *const u8).add(2 * 1024 * 1024 - 1)) } != 0x7d
    {
        exit(86)
    }
    if unsafe { syscall1(SYS_CAP_DESTROY, original) } != 0
        || unsafe { syscall1(SYS_CAP_DESTROY, 2) } != 0
        || unsafe { core::ptr::read_volatile((ro as *const u8).add(2 * 1024 * 1024 - 1)) } != 0x7d
    {
        exit(87)
    }
    // The two mappings pin the first region without any cap references.
    // Fill all seven remaining object slots; the ninth allocation must
    // refuse 40 consecutive times WITHOUT consuming a generation ID.
    let mut slots = [0u64; 7];
    for slot in &mut slots {
        let mut result = [0u64; 3];
        if unsafe { syscall3(SYS_SHARED_CREATE, 0, 1, result.as_mut_ptr() as u64) } != 0
            || result[1] <= first_id
        {
            exit(88)
        }
        *slot = result[0];
    }
    let sentinel = [0xabcdu64; 3];
    for _ in 0..40 {
        let mut refused = sentinel;
        if unsafe { syscall3(SYS_SHARED_CREATE, 0, 1, refused.as_mut_ptr() as u64) } != -4
            || refused != sentinel
        {
            exit(89)
        }
    }
    for slot in slots {
        if unsafe { syscall1(SYS_CAP_DESTROY, slot) } != 0 {
            exit(90)
        }
    }
    // The next ID must be immediately after the eighth real object: no
    // refused request may silently burn a generation or physical run.
    let mut again = [0u64; 3];
    if unsafe { syscall3(SYS_SHARED_CREATE, 0, 1, again.as_mut_ptr() as u64) } != 0
        || again[1] != first_id + 8
    {
        exit(91)
    }
    // Deliberately leave that cap + BOTH original map pins behind. The
    // kernel bootstrap thread will destroy this dead process and prove
    // exact physical, record and process accounting across the sweep.
    let marker = b"[sharedprobe] capacity/rights/zero PASS\n";
    unsafe {
        let _ = syscall2(SYS_DEBUG_WRITE, marker.as_ptr() as u64, marker.len() as u64);
    }
    exit(42)
}
