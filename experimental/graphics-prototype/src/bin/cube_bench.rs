//! Standalone 3D rotating cube benchmark and frame generator.

use arena_graphics_prototype::cube_demo::CubeDemo;
use arena_graphics_prototype::framebuffer::{Framebuffer, PixelFormat};
use arena_graphics_prototype::renderer::RendererConfig;

const WIDTH: usize = 320;
const HEIGHT: usize = 240;
const STRIDE: usize = 320;
const FRAMES: usize = 30;

fn main() {
    println!("=== ArenaOS 3D Software Renderer Benchmark ===");
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

    println!("\nBenchmark Results:");
    println!("  Average Frame Time: {} us ({:.2} ms)", avg_time_us, avg_time_us as f64 / 1000.0);
    println!("  Min Frame Time:     {} us ({:.2} ms)", min_time_us, min_time_us as f64 / 1000.0);
    println!("  Max Frame Time:     {} us ({:.2} ms)", max_time_us, max_time_us as f64 / 1000.0);
    println!("  Effective Frame Rate: {:.1} FPS (single-threaded software rendering)", fps);

    // Export frame 0 to PPM file for visual inspection
    let fb = Framebuffer::new(&mut fb_storage, WIDTH, HEIGHT, STRIDE, PixelFormat::Xrgb8888).unwrap();
    let mut ppm_buf = vec![0u8; WIDTH * HEIGHT * 3 + 1024];
    if let Ok(len) = fb.write_ppm_p6(&mut ppm_buf) {
        let _ = std::fs::write("experimental/graphics-prototype/cube_frame0.ppm", &ppm_buf[..len]);
        println!("\nExported frame 0 to experimental/graphics-prototype/cube_frame0.ppm ({} bytes)", len);
    }
}
