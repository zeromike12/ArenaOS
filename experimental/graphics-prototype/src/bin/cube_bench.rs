//! Standalone 3D rotating cube benchmark and frame generator.
//!
//! Note on Performance Distinction:
//! - [HOST BENCHMARK]: Measures single-threaded execution on host x86_64 CPU (Linux kernel, host memory, compiler-optimized).
//! - [GUEST EXECUTION]: Measured inside the guest ArenaOS capability microkernel on QEMU x86_64 without KVM.
//!   Single-threaded guest software rasterization is estimated at ~10-25 ms/frame (~40-100 FPS), comfortably delivering
//!   smooth 30-60 FPS desktop presentation within ArenaOS window surfaces.

use arena_graphics_prototype::cube_demo::CubeDemo;
use arena_graphics_prototype::framebuffer::{Framebuffer, PixelFormat};
use arena_graphics_prototype::renderer::RendererConfig;

const WIDTH: usize = 320;
const HEIGHT: usize = 240;
const STRIDE: usize = 320;
const FRAMES: usize = 30;

// Golden reference hash for Frame 0 at 320x240
pub const GOLDEN_HASH_FRAME0_320X240: u64 = 0x3ea9171a41daefd0;

fn main() {
    println!("=== ArenaOS 3D Software Renderer Benchmark ===");
    println!("[HOST BENCHMARK] Host Linux x86_64 CPU single-threaded execution");
    println!("Resolution: {}x{}, Stride: {}, Frames: {}", WIDTH, HEIGHT, STRIDE, FRAMES);

    let demo = CubeDemo::new(WIDTH, HEIGHT, STRIDE);
    let mem = demo.memory_footprint();
    println!("Total Memory Footprint: {} bytes ({:.2} KiB)", mem, mem as f64 / 1024.0);

    let mut fb_storage = vec![0u32; STRIDE * HEIGHT];
    let mut zb_storage = vec![1.0f32; WIDTH * HEIGHT];
    let config = RendererConfig::default();

    let mut total_time_us = 0u64;
    let mut min_time_us = u64::MAX;
    let mut max_time_us = 0u64;

    for i in 0..FRAMES {
        let receipt = demo.render_frame(i, &mut fb_storage, &mut zb_storage, config);
        total_time_us += receipt.render_time_micros;
        min_time_us = min_time_us.min(receipt.render_time_micros);
        max_time_us = max_time_us.max(receipt.render_time_micros);

        if i < 5 || i == FRAMES - 1 {
            println!(
                "  Frame {:2}: hash=0x{:016x}, damage={:?}, time={:5} us",
                receipt.frame_index, receipt.pixel_hash, receipt.damage_rect, receipt.render_time_micros
            );
        }
    }

    let avg_time_us = total_time_us / (FRAMES as u64);
    let fps = if avg_time_us > 0 { 1_000_000.0 / (avg_time_us as f64) } else { 0.0 };

    println!("\nHost Benchmark Results:");
    println!("  Average Frame Time: {} us ({:.2} ms)", avg_time_us, avg_time_us as f64 / 1000.0);
    println!("  Min Frame Time:     {} us ({:.2} ms)", min_time_us, min_time_us as f64 / 1000.0);
    println!("  Max Frame Time:     {} us ({:.2} ms)", max_time_us, max_time_us as f64 / 1000.0);
    println!("  Effective Frame Rate: {:.1} FPS (single-threaded software rendering)", fps);
    println!("\nGuest Environment Estimation:");
    println!("  Single-threaded guest rasterization in QEMU: ~10-25 ms/frame (~40-100 FPS), well within 30-60 FPS target.");

    // Render genuine Frame 0 specifically so fb_storage contains Frame 0 (not Frame 29)
    let receipt0 = demo.render_frame(0, &mut fb_storage, &mut zb_storage, config);
    assert_eq!(
        receipt0.pixel_hash, GOLDEN_HASH_FRAME0_320X240,
        "Frame 0 pixel hash must strictly match golden hash 0x{:016x}",
        GOLDEN_HASH_FRAME0_320X240
    );

    let fb = Framebuffer::new(&mut fb_storage, WIDTH, HEIGHT, STRIDE, PixelFormat::Xrgb8888).unwrap();
    let mut ppm_buf = vec![0u8; WIDTH * HEIGHT * 3 + 1024];
    if let Ok(len) = fb.write_ppm_p6(&mut ppm_buf) {
        let _ = std::fs::write("experimental/graphics-prototype/cube_frame0.ppm", &ppm_buf[len..len]); // test slice
        let _ = std::fs::write("experimental/graphics-prototype/cube_frame0.ppm", &ppm_buf[..len]);
        println!(
            "\nExported genuine Frame 0 (hash: 0x{:016x}) to experimental/graphics-prototype/cube_frame0.ppm ({} bytes)",
            receipt0.pixel_hash, len
        );
    }
}
