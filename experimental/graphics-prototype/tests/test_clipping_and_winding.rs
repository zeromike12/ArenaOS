//! Regression tests for near-plane homogeneous clipping, projection edge cases,
//! and front-face/back-face winding and culling.

use arena_graphics_prototype::framebuffer::{Framebuffer, PixelFormat};
use arena_graphics_prototype::math::{Vec2, Vec4};
use arena_graphics_prototype::rasterizer::{
    CullMode, ScreenVertex, ShadingMode, WindingOrder, rasterize_triangle,
};
use arena_graphics_prototype::renderer::{
    ClipVertex, clip_triangle_near_plane, project_clip_to_screen,
};
use arena_graphics_prototype::texture::FilterMode;
use arena_graphics_prototype::zbuffer::ZBuffer;

#[test]
fn test_near_plane_clipping_all_inside() {
    let near_w = 0.1;
    let v0 = ClipVertex::new(Vec4::new(-1.0, -1.0, 0.5, 1.0), Vec2::ZERO, 1.0, 0xFF);
    let v1 = ClipVertex::new(Vec4::new(1.0, -1.0, 0.5, 1.2), Vec2::ZERO, 1.0, 0xFF);
    let v2 = ClipVertex::new(Vec4::new(0.0, 1.0, 0.5, 1.5), Vec2::ZERO, 1.0, 0xFF);

    let (clipped, count) = clip_triangle_near_plane(v0, v1, v2, near_w);
    assert_eq!(count, 1, "Triangle entirely in front must remain 1 triangle");
    assert_eq!(clipped[0], v0);
    assert_eq!(clipped[1], v1);
    assert_eq!(clipped[2], v2);
}

#[test]
fn test_near_plane_clipping_all_outside_or_behind() {
    let near_w = 0.1;
    // Vertices behind camera (w <= 0 or w < near_w)
    let v0 = ClipVertex::new(Vec4::new(-1.0, -1.0, -0.5, -0.5), Vec2::ZERO, 1.0, 0xFF);
    let v1 = ClipVertex::new(Vec4::new(1.0, -1.0, -0.5, 0.0), Vec2::ZERO, 1.0, 0xFF);
    let v2 = ClipVertex::new(Vec4::new(0.0, 1.0, -0.5, 0.05), Vec2::ZERO, 1.0, 0xFF);

    let (_clipped, count) = clip_triangle_near_plane(v0, v1, v2, near_w);
    assert_eq!(count, 0, "Triangle entirely behind near plane must be discarded");
}

#[test]
fn test_near_plane_clipping_one_inside_two_outside() {
    let near_w = 0.1;
    // v0 is inside (w = 1.0), v1 and v2 are behind camera (w = -1.0)
    let v0 = ClipVertex::new(Vec4::new(0.0, 0.0, 0.5, 1.0), Vec2::ZERO, 1.0, 0xFF);
    let v1 = ClipVertex::new(Vec4::new(-2.0, 0.0, 0.5, -1.0), Vec2::ZERO, 1.0, 0xFF);
    let v2 = ClipVertex::new(Vec4::new(2.0, 0.0, 0.5, -1.0), Vec2::ZERO, 1.0, 0xFF);

    let (clipped, count) = clip_triangle_near_plane(v0, v1, v2, near_w);
    assert_eq!(count, 1, "1 vertex inside must produce 1 clipped triangle");

    // All 3 vertices in the clipped triangle must satisfy w >= near_w
    for i in 0..3 {
        assert!(
            clipped[i].pos.w >= near_w - 1e-6,
            "Clipped vertex {i} pos.w = {} must be >= near_w {near_w}",
            clipped[i].pos.w
        );
    }
}

#[test]
fn test_near_plane_clipping_two_inside_one_outside() {
    let near_w = 0.1;
    // v0 and v1 are inside (w = 1.0), v2 is behind camera (w = -0.5)
    let v0 = ClipVertex::new(Vec4::new(-1.0, -1.0, 0.5, 1.0), Vec2::ZERO, 1.0, 0xFF);
    let v1 = ClipVertex::new(Vec4::new(1.0, -1.0, 0.5, 1.0), Vec2::ZERO, 1.0, 0xFF);
    let v2 = ClipVertex::new(Vec4::new(0.0, 2.0, 0.5, -0.5), Vec2::ZERO, 1.0, 0xFF);

    let (clipped, count) = clip_triangle_near_plane(v0, v1, v2, near_w);
    assert_eq!(count, 2, "2 vertices inside must produce 2 clipped triangles (quad)");

    // All 6 vertices across the two triangles must satisfy w >= near_w
    for i in 0..6 {
        assert!(
            clipped[i].pos.w >= near_w - 1e-6,
            "Clipped vertex {i} pos.w = {} must be >= near_w {near_w}",
            clipped[i].pos.w
        );
    }
}

#[test]
fn test_safe_perspective_projection_no_singularity() {
    // Vertex exactly on near plane (w = 0.1)
    let v_near = ClipVertex::new(Vec4::new(0.05, -0.05, 0.01, 0.1), Vec2::ZERO, 1.0, 0xFF);
    let screen = project_clip_to_screen(v_near, 320.0, 240.0);
    assert!(screen.x.is_finite());
    assert!(screen.y.is_finite());
    assert!(screen.z.is_finite());
    assert!(screen.inv_w > 0.0);
}

#[test]
fn test_degenerate_collinear_triangles_culled() {
    let mut fb_storage = [0u32; 100];
    let mut zb_storage = [1.0f32; 100];
    let mut fb = Framebuffer::new(&mut fb_storage, 10, 10, 10, PixelFormat::Xrgb8888).unwrap();
    let mut zb = ZBuffer::new(&mut zb_storage, 10, 10).unwrap();

    // Three collinear vertices along the horizontal line y = 5.0
    let v0 = ScreenVertex { x: 1.0, y: 5.0, z: 0.5, inv_w: 1.0, u_over_w: 0.0, v_over_w: 0.0, light: 1.0 };
    let v1 = ScreenVertex { x: 5.0, y: 5.0, z: 0.5, inv_w: 1.0, u_over_w: 0.0, v_over_w: 0.0, light: 1.0 };
    let v2 = ScreenVertex { x: 9.0, y: 5.0, z: 0.5, inv_w: 1.0, u_over_w: 0.0, v_over_w: 0.0, light: 1.0 };

    rasterize_triangle(
        &mut fb, &mut zb, v0, v1, v2,
        None, ShadingMode::Flat, FilterMode::Nearest,
        CullMode::None, WindingOrder::CounterClockwise,
        0x00_FF_FF_00,
    );

    assert_eq!(fb.take_damage(), None, "Collinear triangle must produce 0 damage");
    assert!(fb.raw_slice().iter().all(|&p| p == 0), "No pixels should be rendered for collinear triangle");
}

#[test]
fn test_winding_modes_exhaustive() {
    let mut fb_storage = [0u32; 100];
    let mut zb_storage = [1.0f32; 100];
    let mut fb = Framebuffer::new(&mut fb_storage, 10, 10, 10, PixelFormat::Xrgb8888).unwrap();
    let mut zb = ZBuffer::new(&mut zb_storage, 10, 10).unwrap();

    // Triangle A: Counter-Clockwise in 3D (signed_area < 0 in screen space)
    let a0 = ScreenVertex { x: 2.0, y: 2.0, z: 0.5, inv_w: 1.0, u_over_w: 0.0, v_over_w: 0.0, light: 1.0 };
    let a1 = ScreenVertex { x: 2.0, y: 8.0, z: 0.5, inv_w: 1.0, u_over_w: 0.0, v_over_w: 0.0, light: 1.0 };
    let a2 = ScreenVertex { x: 8.0, y: 2.0, z: 0.5, inv_w: 1.0, u_over_w: 0.0, v_over_w: 0.0, light: 1.0 };

    // Triangle B: Clockwise in 3D (signed_area > 0 in screen space)
    let b0 = ScreenVertex { x: 2.0, y: 2.0, z: 0.5, inv_w: 1.0, u_over_w: 0.0, v_over_w: 0.0, light: 1.0 };
    let b1 = ScreenVertex { x: 8.0, y: 2.0, z: 0.5, inv_w: 1.0, u_over_w: 0.0, v_over_w: 0.0, light: 1.0 };
    let b2 = ScreenVertex { x: 2.0, y: 8.0, z: 0.5, inv_w: 1.0, u_over_w: 0.0, v_over_w: 0.0, light: 1.0 };

    // 1. CullMode::Back with WindingOrder::CounterClockwise
    // A (CCW) is front-facing -> rendered!
    // B (CW) is back-facing -> culled!
    fb.clear(0);
    fb.take_damage();
    zb.clear(1.0);
    rasterize_triangle(&mut fb, &mut zb, a0, a1, a2, None, ShadingMode::Flat, FilterMode::Nearest, CullMode::Back, WindingOrder::CounterClockwise, 0xFF);
    assert!(fb.take_damage().is_some());

    fb.clear(0);
    fb.take_damage();
    zb.clear(1.0);
    rasterize_triangle(&mut fb, &mut zb, b0, b1, b2, None, ShadingMode::Flat, FilterMode::Nearest, CullMode::Back, WindingOrder::CounterClockwise, 0xFF);
    assert_eq!(fb.take_damage(), None);

    // 2. CullMode::Front with WindingOrder::CounterClockwise
    // A (CCW) is front-facing -> culled!
    // B (CW) is back-facing -> rendered!
    fb.clear(0);
    fb.take_damage();
    zb.clear(1.0);
    rasterize_triangle(&mut fb, &mut zb, a0, a1, a2, None, ShadingMode::Flat, FilterMode::Nearest, CullMode::Front, WindingOrder::CounterClockwise, 0xFF);
    assert_eq!(fb.take_damage(), None);

    fb.clear(0);
    fb.take_damage();
    zb.clear(1.0);
    rasterize_triangle(&mut fb, &mut zb, b0, b1, b2, None, ShadingMode::Flat, FilterMode::Nearest, CullMode::Front, WindingOrder::CounterClockwise, 0xFF);
    assert!(fb.take_damage().is_some());

    // 3. CullMode::Back with WindingOrder::Clockwise (reverse definition of front face)
    // A (CCW) is now back-facing -> culled!
    // B (CW) is now front-facing -> rendered!
    fb.clear(0);
    fb.take_damage();
    zb.clear(1.0);
    rasterize_triangle(&mut fb, &mut zb, a0, a1, a2, None, ShadingMode::Flat, FilterMode::Nearest, CullMode::Back, WindingOrder::Clockwise, 0xFF);
    assert_eq!(fb.take_damage(), None);

    fb.clear(0);
    fb.take_damage();
    zb.clear(1.0);
    rasterize_triangle(&mut fb, &mut zb, b0, b1, b2, None, ShadingMode::Flat, FilterMode::Nearest, CullMode::Back, WindingOrder::Clockwise, 0xFF);
    assert!(fb.take_damage().is_some());
}
