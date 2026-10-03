//! Phase-9 first independent ring-3 window image, not yet boot-linked.
#![no_std]
#![no_main]
#[path = "demo-common.rs"]
mod demo_common;
use core::panic::PanicInfo;
#[panic_handler]
fn panic(_: &PanicInfo<'_>) -> ! {
    demo_common::die(99)
}
#[unsafe(no_mangle)]
pub extern "C" fn _start() -> ! {
    demo_common::run(
        b"[window_a] held-cap surface painted\n",
        0x00_bd_53_38,
        "FIRST",
        50,
        50,
        false,
    )
}
