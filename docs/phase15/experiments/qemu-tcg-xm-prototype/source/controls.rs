#![no_std]
#![no_main]

core::arch::global_asm!(
    include_str!("independent-xm-controls.s"),
    options(att_syntax),
);

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo<'_>) -> ! {
    loop {
        core::hint::spin_loop();
    }
}
