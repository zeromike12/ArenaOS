//! Ordinary graphical gallery ELF; integration/QMP qualification is separate.
#![no_std]
#![no_main]
use arena_desktop::{
    client::{self, Client},
    model::Event,
};
use arena_gfxkit::Canvas;
use arena_ui::{components, metrics, theme};
#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    client::exit(99)
}
#[unsafe(no_mangle)]
pub extern "C" fn _start() -> ! {
    let client = Client::connect(
        metrics::WINDOW_WIDTH,
        metrics::WINDOW_HEIGHT,
        "ArenaOS UI Gallery",
    )
    .unwrap_or_else(|_| client::exit(80));
    let mut dark = false;
    let paint = |dark| {
        // SAFETY: connect checked the full backing and established an owned writable
        // mapping. This single-threaded client creates only this mutable view.
        let pixels =
            unsafe { core::slice::from_raw_parts_mut(client.pixels, client.width * client.height) };
        let mut canvas = Canvas::new(pixels, client.width, client.height, client.width)
            .unwrap_or_else(|_| client::exit(81));
        components::gallery(&mut canvas, theme::palette(dark));
        client.damage().unwrap_or_else(|_| client::exit(82));
    };
    paint(dark);
    loop {
        match client.poll().unwrap_or_else(|_| client::exit(83)) {
            Some(Event::Close) => client::exit(42),
            Some(Event::Key(116)) => {
                dark = !dark;
                paint(dark);
            }
            _ => {}
        }
    }
}
