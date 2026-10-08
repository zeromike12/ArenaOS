//! 3D Textured Rotating Cube Demo tests, golden reference hashes, and benchmarks.

use arena_graphics_prototype::cube_demo::CubeDemo;
use arena_graphics_prototype::renderer::RendererConfig;

const WIDTH_160: usize = 160;
const HEIGHT_120: usize = 120;
const STRIDE_160: usize = 160;

const WIDTH_320: usize = 320;
const HEIGHT_240: usize = 240;
const STRIDE_320: usize = 320;

// Golden reference hashes for Frame 0 (deterministic software rendering baseline)
pub const GOLDEN_HASH_FRAME0_160X120: u64 = 0x0dce5885c41363f1;
pub const GOLDEN_HASH_FRAME0_320X240: u64 = 0x3ea9171a41daefd0;

#[test]
fn test_cube_demo_deterministic_execution() {
    let demo = CubeDemo::new(WIDTH_160, HEIGHT_120, STRIDE_160);

    let mut fb1 = vec![0u32; STRIDE_160 * HEIGHT_120];
    let mut zb1 = vec![1.0f32; WIDTH_160 * HEIGHT_120];

    let mut fb2 = vec![0u32; STRIDE_160 * HEIGHT_120];
    let mut zb2 = vec![1.0f32; WIDTH_160 * HEIGHT_120];

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
fn test_golden_frame_pixel_verification_160x120() {
    let demo = CubeDemo::new(WIDTH_160, HEIGHT_120, STRIDE_160);
    let mut fb = vec![0u32; STRIDE_160 * HEIGHT_120];
    let mut zb = vec![1.0f32; WIDTH_160 * HEIGHT_120];
    let config = RendererConfig::default();

    let receipt = demo.render_frame(0, &mut fb, &mut zb, config);

    // Verify background color at corners (0, 0)
    let bg_color = fb[0];
    assert_eq!(bg_color, 0x00_1A_23_32, "Corner (0, 0) must be background dark slate");

    // Center pixel of cube projection (80, 60) must be colored by cube and lighting
    let center_pixel = fb[60 * STRIDE_160 + 80];
    assert_ne!(center_pixel, bg_color, "Center (80, 60) must be covered by cube");

    // Assert exact golden hash
    assert_eq!(
        receipt.pixel_hash, GOLDEN_HASH_FRAME0_160X120,
        "Frame 0 at 160x120 must strictly match golden hash 0x{:016x}",
        GOLDEN_HASH_FRAME0_160X120
    );
}

#[test]
fn test_golden_frame_pixel_verification_320x240() {
    let demo = CubeDemo::new(WIDTH_320, HEIGHT_240, STRIDE_320);
    let mut fb = vec![0u32; STRIDE_320 * HEIGHT_240];
    let mut zb = vec![1.0f32; WIDTH_320 * HEIGHT_240];
    let config = RendererConfig::default();

    let receipt = demo.render_frame(0, &mut fb, &mut zb, config);

    // Verify background color at corners (0, 0)
    let bg_color = fb[0];
    assert_eq!(bg_color, 0x00_1A_23_32, "Corner (0, 0) must be background dark slate");

    // Center pixel of cube projection (160, 120) must be colored by cube and lighting
    let center_pixel = fb[120 * STRIDE_320 + 160];
    assert_ne!(center_pixel, bg_color, "Center (160, 120) must be covered by cube");

    // Assert exact golden hash
    assert_eq!(
        receipt.pixel_hash, GOLDEN_HASH_FRAME0_320X240,
        "Frame 0 at 320x240 must strictly match golden hash 0x{:016x}",
        GOLDEN_HASH_FRAME0_320X240
    );
}

#[test]
fn test_animation_sequence_generates_motion() {
    let demo = CubeDemo::new(WIDTH_160, HEIGHT_120, STRIDE_160);

    let mut fb = vec![0u32; STRIDE_160 * HEIGHT_120];
    let mut zb = vec![1.0f32; WIDTH_160 * HEIGHT_120];
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
    let demo = CubeDemo::new(WIDTH_160, HEIGHT_120, STRIDE_160);
    let mem = demo.memory_footprint();

    // 160 * 120 * 4 = 76,800 bytes for Framebuffer
    // 160 * 120 * 4 = 76,800 bytes for Z-Buffer
    // 64 * 64 * 4   = 16,384 bytes for Texture
    // Mesh struct   = ~1,600 bytes
    // Total should be ~171 KiB (well within ArenaOS 16 MiB heap envelope!)
    assert!(mem > 150_000 && mem < 200_000, "Memory footprint must be ~171 KiB, got {mem}");
}
