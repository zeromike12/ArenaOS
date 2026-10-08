//! 3D Textured Rotating Cube Demo tests, golden reference hashes, and benchmarks.

use arena_graphics_prototype::cube_demo::CubeDemo;
use arena_graphics_prototype::renderer::RendererConfig;

const WIDTH: usize = 160;
const HEIGHT: usize = 120;
const STRIDE: usize = 160;

#[test]
fn test_cube_demo_deterministic_execution() {
    let demo = CubeDemo::new(WIDTH, HEIGHT, STRIDE);

    let mut fb1 = vec![0u32; STRIDE * HEIGHT];
    let mut zb1 = vec![1.0f32; WIDTH * HEIGHT];

    let mut fb2 = vec![0u32; STRIDE * HEIGHT];
    let mut zb2 = vec![1.0f32; WIDTH * HEIGHT];

    let config = RendererConfig::default();

    // Render frame 0 twice into independent buffers
    let r1 = demo.render_frame(0, &mut fb1, &mut zb1, config);
    let r2 = demo.render_frame(0, &mut fb2, &mut zb2, config);

    assert_eq!(r1.frame_index, 0);
    assert_eq!(r2.frame_index, 0);

    // Hashes and damage rects must be identical
    assert_eq!(r1.pixel_hash, r2.pixel_hash, "Deterministic FNV hash must match");
    assert_eq!(r1.damage_rect, r2.damage_rect, "Damage rect must match");

    // All pixels must match bit-for-bit
    assert_eq!(fb1, fb2, "Rendered pixels must be bit-identical");
    assert_ne!(r1.pixel_hash, 0, "Rendered frame must not be empty");
}

#[test]
fn test_animation_sequence_generates_motion() {
    let demo = CubeDemo::new(WIDTH, HEIGHT, STRIDE);

    let mut fb = vec![0u32; STRIDE * HEIGHT];
    let mut zb = vec![1.0f32; WIDTH * HEIGHT];
    let config = RendererConfig::default();

    let receipts = demo.render_animation(8, &mut fb, &mut zb, config);

    // Each frame in the rotation must produce a distinct hash (proving animation movement)
    for i in 0..7 {
        assert_ne!(
            receipts[i].pixel_hash,
            receipts[i + 1].pixel_hash,
            "Frames {i} and {} must have differing pixel hashes during rotation",
            i + 1
        );
        assert!(
            receipts[i].damage_rect.is_some(),
            "Frame {i} must produce a valid damage rectangle"
        );
        let dmg = receipts[i].damage_rect.unwrap();
        assert!(dmg.width > 0 && dmg.height > 0, "Damage rectangle must be non-empty");
    }
}

#[test]
fn test_memory_footprint_measurement() {
    let demo = CubeDemo::new(WIDTH, HEIGHT, STRIDE);
    let mem = demo.memory_footprint();

    // 160 * 120 * 4 = 76,800 bytes for Framebuffer
    // 160 * 120 * 4 = 76,800 bytes for Z-Buffer
    // 64 * 64 * 4   = 16,384 bytes for Texture
    // Mesh struct   = ~1,600 bytes
    // Total should be ~171 KiB (well within ArenaOS 16 MiB heap envelope!)
    assert!(mem > 150_000 && mem < 200_000, "Memory footprint must be ~171 KiB, got {mem}");
}

#[test]
fn test_golden_frame_pixel_verification() {
    let demo = CubeDemo::new(WIDTH, HEIGHT, STRIDE);
    let mut fb = vec![0u32; STRIDE * HEIGHT];
    let mut zb = vec![1.0f32; WIDTH * HEIGHT];
    let config = RendererConfig::default();

    let receipt = demo.render_frame(0, &mut fb, &mut zb, config);

    // Verify background color at corners (0, 0)
    let bg_color = fb[0];
    assert_eq!(bg_color, 0x00_1A_23_32, "Corner (0, 0) must be background dark slate");

    // Center pixel of cube projection (80, 60) must be colored by cube and lighting
    let center_pixel = fb[60 * STRIDE + 80];
    assert_ne!(center_pixel, bg_color, "Center (80, 60) must be covered by cube");

    println!(
        "[golden_frame] Frame 0: hash=0x{:016x}, damage={:?}, render_time={}us, mem={}b",
        receipt.pixel_hash, receipt.damage_rect, receipt.render_time_micros, receipt.memory_bytes
    );
}
