//! Phase-9 second independent ring-3 window image, not yet boot-linked.
#![no_std]
#![no_main]
#[path = "demo-common.rs"] mod demo_common;
use core::panic::PanicInfo;
#[panic_handler] fn panic(_: &PanicInfo<'_>) -> ! { demo_common::die(99) }
#[unsafe(no_mangle)] pub extern "C" fn _start() -> ! {
    demo_common::run(b"[window_b] held-cap focused surface painted\n", 0x00_3d_cf_7a, "SECOND", 180, 150, true)
}
