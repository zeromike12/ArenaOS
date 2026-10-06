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

/// The kernel's SharedRegion object table (`shared::MAX_REGIONS`, ADR-0088).
const REGIONS: usize = 80;

#[unsafe(no_mangle)]
pub extern "C" fn _start() -> ! {
    // Pool/WRITE is the only inherited cap. No GPU, framebuffer, DMA,
    // Power, process or image authority enters this guest.
    let mut out = [0u64; 3];
    if unsafe { syscall3(SYS_SHARED_CREATE, 0, 512, out.as_mut_ptr() as u64) } != 0
        || !matches!(out[0], 1 | 2)
        || out[1] == 0
        || out[2] != 2 * 1024 * 1024
    {
        exit(81)
    }
    let original = out[0];
    let first_id = out[1];
    let read_copy_slot = if original == 2 { 3 } else { 2 };
    // The optional slot-1 Mmio cap is deliberately a *truncated* BAR,
    // read-only and owned by this throwaway process. SYS_DEV_INFO may
    // not disclose any matched device record without coverage of the
    // full common/notify/ISR/device structure spans. Failure may not
    // write to the guest's output, even when other devices are present.
    if original == 2 {
        for idx in 0..8 {
            let mut denied = [0xa5a5_1729_u64; 12];
            if unsafe { syscall2(SYS_DEV_INFO, idx, denied.as_mut_ptr() as u64) } != -2
                || denied != [0xa5a5_1729_u64; 12]
            {
                exit(95)
            }
        }
        let marker = b"[sharedprobe] truncated virtio BAR device-info refused PASS\n";
        unsafe {
            let _ = syscall2(SYS_DEBUG_WRITE, marker.as_ptr() as u64, marker.len() as u64);
        }
    }
    // A server cannot trust a client's claimed surface size. The generic
    // possession-gated INFO query returns [full ID, pages], never phys.
    // All wrong/missing/attenuated/reserved/address cases must preserve
    // its sentinel unchanged (the service has no ambient pid grant).
    let mut info = [0xdec0_de01_u64; 2];
    if unsafe { syscall6(SYS_SHARED_INFO, 0, info.as_mut_ptr() as u64, 0, 0, 0, 0) } != -2
        || info != [0xdec0_de01_u64; 2]
        || unsafe {
            syscall6(
                SYS_SHARED_INFO,
                original,
                info.as_mut_ptr() as u64,
                1,
                0,
                0,
                0,
            )
        } != -2
        || info != [0xdec0_de01_u64; 2]
        || unsafe { syscall6(SYS_SHARED_INFO, original, 0, 0, 0, 0, 0) } != -3
        || info != [0xdec0_de01_u64; 2]
        || unsafe {
            syscall6(
                SYS_SHARED_INFO,
                original,
                info.as_mut_ptr() as u64,
                0,
                0,
                0,
                0,
            )
        } != 0
        || info != [first_id, 512]
    {
        exit(92)
    }
    if unsafe { syscall3(SYS_CAP_COPY, original, 3, RIGHTS_WRITE | RIGHTS_DESTROY) } != 0 {
        exit(93)
    }
    info = [0xdec0_de01_u64; 2];
    if unsafe { syscall6(SYS_SHARED_INFO, 3, info.as_mut_ptr() as u64, 0, 0, 0, 0) } != -2
        || info != [0xdec0_de01_u64; 2]
        || unsafe { syscall1(SYS_CAP_DESTROY, 3) } != 0
    {
        exit(94)
    }
    let info_marker = b"[sharedprobe] held SharedRegion INFO bound/refusal PASS\n";
    unsafe {
        let _ = syscall2(
            SYS_DEBUG_WRITE,
            info_marker.as_ptr() as u64,
            info_marker.len() as u64,
        );
    }
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
    if unsafe {
        syscall3(
            SYS_CAP_COPY,
            original,
            read_copy_slot,
            RIGHTS_READ | RIGHTS_DESTROY,
        )
    } != 0
        || unsafe { syscall2(SYS_SHARED_MAP, read_copy_slot, 1) } != -2
    {
        exit(85)
    }
    let ro = unsafe { syscall2(SYS_SHARED_MAP, read_copy_slot, 0) };
    if ro <= 0
        || unsafe { core::ptr::read_volatile((ro as *const u8).add(2 * 1024 * 1024 - 1)) } != 0x7d
    {
        exit(86)
    }
    if unsafe { syscall1(SYS_CAP_DESTROY, original) } != 0
        || unsafe { syscall1(SYS_CAP_DESTROY, read_copy_slot) } != 0
        || unsafe { core::ptr::read_volatile((ro as *const u8).add(2 * 1024 * 1024 - 1)) } != 0x7d
    {
        exit(87)
    }
    // The two mappings pin the first region without any cap references.
    // Fill every remaining object slot of the kernel's 80-region table
    // (ADR-0088); the eighty-first allocation must refuse 40 consecutive
    // times WITHOUT consuming a generation ID.
    let mut slots = [0u64; REGIONS - 1];
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
    // The next ID must be immediately after the last real object: no
    // refused request may silently burn a generation or physical run.
    let mut again = [0u64; 3];
    if unsafe { syscall3(SYS_SHARED_CREATE, 0, 1, again.as_mut_ptr() as u64) } != 0
        || again[1] != first_id + REGIONS as u64
    {
        exit(91)
    }
    // Exact own-mapping lifecycle: revoke the last cap *before* unmap
    // and prove its PTE pin still holds the byte; wrong/partial/duplicate
    // VAs and noncanonical flags refuse. Forty-eight create/map/destroy/
    // unmap rounds must reuse one region-table stride instead of slowly
    // exhausting its 80 slots or the global 128-map/80-object tables.
    for round in 0..48u64 {
        let current = if round == 0 {
            again
        } else {
            let mut next = [0u64; 3];
            if unsafe { syscall3(SYS_SHARED_CREATE, 0, 1, next.as_mut_ptr() as u64) } != 0
                || next[1] != first_id + REGIONS as u64 + round
            {
                exit(96)
            }
            next
        };
        let addr = unsafe { syscall2(SYS_SHARED_MAP, current[0], 1) };
        if addr <= 0 {
            log_line(|o| {
                o.str("sharedprobe: unmap map failed round=");
                o.u64(round);
                o.str(" slot=");
                o.u64(current[0]);
                o.str(" status=");
                o.i64(addr);
            });
            exit(97)
        }
        if unsafe { syscall6(SYS_SHARED_UNMAP, addr as u64 + 1, 0, 0, 0, 0, 0) } != -2
            || unsafe { syscall6(SYS_SHARED_UNMAP, addr as u64, 1, 0, 0, 0, 0) } != -2
            || unsafe { syscall6(SYS_SHARED_UNMAP, addr as u64 + 4096, 0, 0, 0, 0, 0) } != -2
        {
            exit(98)
        }
        unsafe { core::ptr::write_volatile(addr as *mut u8, (round as u8) + 1) };
        if unsafe { syscall1(SYS_CAP_DESTROY, current[0]) } != 0
            || unsafe { core::ptr::read_volatile(addr as *const u8) } != (round as u8) + 1
            || unsafe { syscall6(SYS_SHARED_UNMAP, addr as u64, 0, 0, 0, 0, 0) } != 0
            || unsafe { syscall6(SYS_SHARED_UNMAP, addr as u64, 0, 0, 0, 0, 0) } != -2
        {
            exit(99)
        }
    }
    let mut leave = [0u64; 3];
    if unsafe { syscall3(SYS_SHARED_CREATE, 0, 1, leave.as_mut_ptr() as u64) } != 0
        || leave[1] != first_id + REGIONS as u64 + 48
    {
        exit(100)
    }
    let unmap_marker = b"[sharedprobe] exact own SharedRegion UNMAP 48x capless churn PASS\n";
    unsafe {
        let _ = syscall2(
            SYS_DEBUG_WRITE,
            unmap_marker.as_ptr() as u64,
            unmap_marker.len() as u64,
        );
    }
    // Deliberately leave the last cap + BOTH original map pins behind.
    // Kernel bootstrap must sweep them and account every physical page.
    let marker = b"[sharedprobe] capacity/rights/zero PASS\n";
    unsafe {
        let _ = syscall2(SYS_DEBUG_WRITE, marker.as_ptr() as u64, marker.len() as u64);
    }
    exit(42)
}
