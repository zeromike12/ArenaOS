//! Native ArenaOS 3D Rotating Cube Desktop Application.
//!
//! Demonstrates zero-copy 3D software rendering directly into an ArenaOS desktop window
//! using the qualified Phase-13 interfaces:
//! - Startup ABI v2 (`arena_runtime::startup::run`)
//! - Desktop client session (`Client::connect_v2`)
//! - Mapped `SharedRegion` backing surface
//! - Real damage presentation (`window.damage()`)
//! - Event loop handling (`window.poll()`)
//! - Monotonic microsecond clock timing via `arena_desktop::app_client::now()`

#![no_std]
#![no_main]

extern crate alloc;

use alloc::vec;
use arena_desktop::client::{self, Client};
use arena_desktop::model::Event as WindowEvent;
use arena_graphics_prototype::cube_demo::CubeDemo;
use arena_graphics_prototype::renderer::RendererConfig;
use arena_runtime::heap::ScalableHeap;
use arena_startup_abi::startup::StartupView;

#[global_allocator]
static APPLICATION_HEAP: ScalableHeap = ScalableHeap::new();

#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    client::exit(99)
}

arena_desktop::entry!(main, 64 * 1024);

extern "C" fn main() -> ! {
    match arena_runtime::startup::run(|view| cube_main(view)) {
        Ok(()) => client::exit(0),
        Err(_) => {
            client::log(b"[cube-app] runtime startup error\n");
            client::exit(70);
        }
    }
}

fn cube_main(_view: StartupView<'_>) {
    client::log(b"[cube-app] Startup ABI v2 verified; connecting to desktop compositor\n");

    let width = 320;
    let height = 240;
    let window = Client::connect_v2(width, height, "Rotating 3D Cube")
        .unwrap_or_else(|_| {
            client::log(b"[cube-app] desktop connect_v2 failed\n");
            client::exit(75);
        });

    client::log(b"[cube-app] window connected; backing mapped via real SharedRegion\n");

    let demo = CubeDemo::new(width, height, width);
    let mut zb_storage = vec![1.0f32; width * height];
    let config = RendererConfig::default();

    let mut total_raster_us: u64 = 0;
    let mut total_present_us: u64 = 0;
    let anim_start_us = arena_desktop::app_client::now();

    // Render 60 frames demonstrating continuous 3D rotation and measure genuine guest timing
    let total_frames = 60;
    for frame_idx in 0..total_frames {
        // Direct zero-copy slice of the desktop window's mapped SharedRegion surface
        let slice = unsafe {
            core::slice::from_raw_parts_mut(window.pixels, width * height)
        };

        // 1. Measure genuine guest rasterization time (CPU rendering into surface)
        let t_raster_start = arena_desktop::app_client::now();
        let receipt = demo.render_frame(frame_idx, slice, &mut zb_storage, config);
        let t_raster_end = arena_desktop::app_client::now();
        let raster_us = t_raster_end.saturating_sub(t_raster_start);
        total_raster_us += raster_us;

        // 2. Measure genuine guest presentation time (synchronous damage IPC to compositor)
        let t_present_start = t_raster_end;
        window.damage().unwrap_or_else(|_| client::exit(76));
        let t_present_end = arena_desktop::app_client::now();
        let present_us = t_present_end.saturating_sub(t_present_start);
        total_present_us += present_us;

        client::log(b"[cube-app] frame rendered; damage published; frame=");
        log_u64(frame_idx as u64);
        client::log(b"; hash=0x");
        log_hex(receipt.pixel_hash);
        client::log(b"; raster_us=");
        log_u64(raster_us);
        client::log(b"; present_us=");
        log_u64(present_us);
        client::log(b"\n");

        // Poll for window events (Close, Key, etc.)
        if let Ok(Some(event)) = window.poll() {
            match event {
                WindowEvent::Close => {
                    client::log(b"[cube-app] close event received; retiring window\n");
                    break;
                }
                _ => {}
            }
        }
    }

    let anim_end_us = arena_desktop::app_client::now();
    let total_elapsed_us = anim_end_us.saturating_sub(anim_start_us);

    client::log(b"[cube-app] timing summary: total_frames=60; total_elapsed_us=");
    log_u64(total_elapsed_us);
    client::log(b"; total_raster_us=");
    log_u64(total_raster_us);
    client::log(b"; total_present_us=");
    log_u64(total_present_us);
    client::log(b"\n");

    client::log(b"[cube-app] animation sequence completed successfully; exiting\n");
}

fn log_u64(mut n: u64) {
    if n == 0 {
        client::log(b"0");
        return;
    }
    let mut buf = [0u8; 20];
    let mut i = 0;
    while n > 0 {
        buf[i] = b'0' + (n % 10) as u8;
        n /= 10;
        i += 1;
    }
    buf[..i].reverse();
    client::log(&buf[..i]);
}

fn log_hex(n: u64) {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut buf = [0u8; 16];
    for i in 0..16 {
        let shift = (15 - i) * 4;
        buf[i] = HEX[((n >> shift) & 0xF) as usize];
    }
    client::log(&buf);
}
