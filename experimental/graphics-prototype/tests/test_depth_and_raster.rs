//! Depth buffer and triangle rasterization correctness tests.

use arena_graphics_prototype::framebuffer::{Framebuffer, PixelFormat};
use arena_graphics_prototype::rasterizer::{
    ScreenVertex, ShadingMode, rasterize_triangle,
};
use arena_graphics_prototype::texture::{FilterMode, Texture, WrapMode};
use arena_graphics_prototype::zbuffer::ZBuffer;

#[test]
fn test_hidden_surface_removal_order_independence() {
    // Render two overlapping triangles covering the same pixel (10, 10):
    // Triangle Near at z = 0.3 (Red: 0x00_FF_00_00)
    // Triangle Far at z = 0.7 (Blue: 0x00_00_00_FF)
    // Case 1: Draw Far first, then Near.
    // Case 2: Draw Near first, then Far.
    // In both cases, the final pixel at (10, 10) MUST be Near (Red)!

    let v_near_0 = ScreenVertex { x: 5.0, y: 5.0, z: 0.3, inv_w: 1.0, u_over_w: 0.0, v_over_w: 0.0, light: 1.0 };
    let v_near_1 = ScreenVertex { x: 15.0, y: 5.0, z: 0.3, inv_w: 1.0, u_over_w: 0.0, v_over_w: 0.0, light: 1.0 };
    let v_near_2 = ScreenVertex { x: 10.0, y: 15.0, z: 0.3, inv_w: 1.0, u_over_w: 0.0, v_over_w: 0.0, light: 1.0 };

    let v_far_0 = ScreenVertex { x: 5.0, y: 5.0, z: 0.7, inv_w: 1.0, u_over_w: 0.0, v_over_w: 0.0, light: 1.0 };
    let v_far_1 = ScreenVertex { x: 15.0, y: 5.0, z: 0.7, inv_w: 1.0, u_over_w: 0.0, v_over_w: 0.0, light: 1.0 };
    let v_far_2 = ScreenVertex { x: 10.0, y: 15.0, z: 0.7, inv_w: 1.0, u_over_w: 0.0, v_over_w: 0.0, light: 1.0 };

    // Case 1: Far then Near
    let mut fb1 = [0u32; 400]; // 20x20
    let mut zb1 = [1.0f32; 400];
    {
        let mut fb = Framebuffer::new(&mut fb1, 20, 20, 20, PixelFormat::Xrgb8888).unwrap();
        let mut zb = ZBuffer::new(&mut zb1, 20, 20).unwrap();

        // Far (Blue)
        rasterize_triangle(&mut fb, &mut zb, v_far_0, v_far_1, v_far_2, None, ShadingMode::Flat, FilterMode::Nearest, 0x00_00_00_FF);
        // Near (Red)
        rasterize_triangle(&mut fb, &mut zb, v_near_0, v_near_1, v_near_2, None, ShadingMode::Flat, FilterMode::Nearest, 0x00_FF_00_00);
    }

    // Case 2: Near then Far
    let mut fb2 = [0u32; 400];
    let mut zb2 = [1.0f32; 400];
    {
        let mut fb = Framebuffer::new(&mut fb2, 20, 20, 20, PixelFormat::Xrgb8888).unwrap();
        let mut zb = ZBuffer::new(&mut zb2, 20, 20).unwrap();

        // Near (Red)
        rasterize_triangle(&mut fb, &mut zb, v_near_0, v_near_1, v_near_2, None, ShadingMode::Flat, FilterMode::Nearest, 0x00_FF_00_00);
        // Far (Blue)
        rasterize_triangle(&mut fb, &mut zb, v_far_0, v_far_1, v_far_2, None, ShadingMode::Flat, FilterMode::Nearest, 0x00_00_00_FF);
    }

    // Both cases must yield exact Red at (10, 10)
    let p1 = fb1[10 * 20 + 10];
    let p2 = fb2[10 * 20 + 10];
    assert_eq!(p1, 0x00_FF_00_00, "Case 1: pixel should be Red");
    assert_eq!(p2, 0x00_FF_00_00, "Case 2: pixel should be Red");

    // All pixels across both buffers must match bit-for-bit!
    assert_eq!(fb1, fb2, "Hidden surface removal must be strictly order-independent");
}

#[test]
fn test_backface_culling() {
    let mut fb_storage = [0u32; 100];
    let mut zb_storage = [1.0f32; 100];
    let mut fb = Framebuffer::new(&mut fb_storage, 10, 10, 10, PixelFormat::Xrgb8888).unwrap();
    let mut zb = ZBuffer::new(&mut zb_storage, 10, 10).unwrap();

    // Clockwise winding (back-facing)
    let v0 = ScreenVertex { x: 1.0, y: 1.0, z: 0.5, inv_w: 1.0, u_over_w: 0.0, v_over_w: 0.0, light: 1.0 };
    let v1 = ScreenVertex { x: 1.0, y: 8.0, z: 0.5, inv_w: 1.0, u_over_w: 0.0, v_over_w: 0.0, light: 1.0 };
    let v2 = ScreenVertex { x: 8.0, y: 1.0, z: 0.5, inv_w: 1.0, u_over_w: 0.0, v_over_w: 0.0, light: 1.0 };

    rasterize_triangle(&mut fb, &mut zb, v0, v1, v2, None, ShadingMode::Flat, FilterMode::Nearest, 0x00_FF_00_00);

    // No pixels should have been drawn due to backface culling
    assert_eq!(fb.take_damage(), None);
    assert!(fb_storage.iter().all(|&p| p == 0));
}

#[test]
fn test_texture_sampling_and_filtering() {
    let tex_data = [
        0x00_00_00_00, 0x00_FF_FF_FF, // Black, White
        0x00_FF_FF_FF, 0x00_00_00_00, // White, Black
    ];
    let tex = Texture::new(&tex_data, 2, 2);

    // Exact corner sample (Nearest)
    let c0 = tex.sample(arena_graphics_prototype::math::Vec2::new(0.2, 0.2), FilterMode::Nearest, WrapMode::Clamp);
    assert_eq!(c0, 0x00_00_00_00);

    let c1 = tex.sample(arena_graphics_prototype::math::Vec2::new(0.8, 0.2), FilterMode::Nearest, WrapMode::Clamp);
    assert_eq!(c1, 0x00_FF_FF_FF);

    // Center sample (Bilinear): equal blend of black and white should yield gray (~127)
    let c_mid = tex.sample(arena_graphics_prototype::math::Vec2::new(0.5, 0.5), FilterMode::Bilinear, WrapMode::Clamp);
    let r = (c_mid >> 16) & 0xFF;
    let g = (c_mid >> 8) & 0xFF;
    let b = c_mid & 0xFF;
    assert!((r as i32 - 127).abs() <= 2, "Bilinear blend red should be ~127, got {r}");
    assert!((g as i32 - 127).abs() <= 2, "Bilinear blend green should be ~127, got {g}");
    assert!((b as i32 - 127).abs() <= 2, "Bilinear blend blue should be ~127, got {b}");
}
