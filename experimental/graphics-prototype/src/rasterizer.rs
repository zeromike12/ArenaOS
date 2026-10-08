//! 3D Triangle Rasterizer with perspective-correct interpolation and depth testing.
//!
//! Implements:
//! - Backface culling
//! - Sub-pixel accurate barycentric edge functions
//! - Perspective-correct attribute interpolation (1/w, u/w, v/w)
//! - Depth buffering (Z-buffer) with early-Z test
//! - Directional Lambertian lighting with ambient term
//! - Bilinear/Nearest texture sampling
//! - Damage bounding box calculation for ArenaOS presentation

#![allow(dead_code)]

use crate::framebuffer::{DamageRect, Framebuffer};
use crate::math::{Vec2, Vec3, det_ceil, det_floor};
use crate::texture::{FilterMode, Texture, WrapMode};
use crate::zbuffer::ZBuffer;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Vertex {
    pub pos: Vec3,
    pub uv: Vec2,
    pub normal: Vec3,
    pub color: u32,
}

impl Vertex {
    pub const fn new(pos: Vec3, uv: Vec2, normal: Vec3, color: u32) -> Self {
        Self {
            pos,
            uv,
            normal,
            color,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct ScreenVertex {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub inv_w: f32,
    pub u_over_w: f32,
    pub v_over_w: f32,
    pub light: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShadingMode {
    Flat,
    Gouraud,
    Textured,
    TexturedLit,
}

/// Rasterize a single triangle with perspective-correct attributes and depth test.
#[allow(clippy::too_many_arguments)]
pub fn rasterize_triangle(
    fb: &mut Framebuffer<'_>,
    zb: &mut ZBuffer<'_>,
    v0: ScreenVertex,
    v1: ScreenVertex,
    v2: ScreenVertex,
    texture: Option<&Texture<'_>>,
    shading: ShadingMode,
    filter: FilterMode,
    flat_color: u32,
) {
    // 2D cross product for signed area and back-face culling (counter-clockwise winding)
    let area = (v1.x - v0.x) * (v2.y - v0.y) - (v1.y - v0.y) * (v2.x - v0.x);
    if area <= 0.0 {
        // Back-facing or degenerate triangle
        return;
    }
    let inv_area = 1.0 / area;

    // Viewport-clipped bounding box
    let fb_w = fb.width() as i32;
    let fb_h = fb.height() as i32;

    let min_x = (det_floor(v0.x.min(v1.x).min(v2.x)) as i32).clamp(0, fb_w - 1);
    let max_x = (det_ceil(v0.x.max(v1.x).max(v2.x)) as i32).clamp(0, fb_w - 1);
    let min_y = (det_floor(v0.y.min(v1.y).min(v2.y)) as i32).clamp(0, fb_h - 1);
    let max_y = (det_ceil(v0.y.max(v1.y).max(v2.y)) as i32).clamp(0, fb_h - 1);

    if min_x > max_x || min_y > max_y {
        return;
    }

    // Edge setup
    let e01_a = v0.y - v1.y;
    let e01_b = v1.x - v0.x;
    let e01_c = v0.x * v1.y - v0.y * v1.x;

    let e12_a = v1.y - v2.y;
    let e12_b = v2.x - v1.x;
    let e12_c = v1.x * v2.y - v1.y * v2.x;

    let e20_a = v2.y - v0.y;
    let e20_b = v0.x - v2.x;
    let e20_c = v2.x * v0.y - v2.y * v0.x;

    let mut triangle_dirty = false;
    let mut dam_min_x = max_x as u16;
    let mut dam_max_x = min_x as u16;
    let mut dam_min_y = max_y as u16;
    let mut dam_max_y = min_y as u16;

    for y in min_y..=max_y {
        let py = y as f32 + 0.5;
        for x in min_x..=max_x {
            let px = x as f32 + 0.5;

            // Evaluate edge functions
            let w0 = e12_a * px + e12_b * py + e12_c;
            let w1 = e20_a * px + e20_b * py + e20_c;
            let w2 = e01_a * px + e01_b * py + e01_c;

            if w0 >= 0.0 && w1 >= 0.0 && w2 >= 0.0 {
                // Normalized barycentric weights
                let l0 = w0 * inv_area;
                let l1 = w1 * inv_area;
                let l2 = w2 * inv_area;

                // Interpolate screen-space depth
                let z = l0 * v0.z + l1 * v1.z + l2 * v2.z;

                // Early depth test
                let ux = x as usize;
                let uy = y as usize;
                if zb.test_and_set(ux, uy, z) {
                    // Interpolate 1/w for perspective correction
                    let inv_w = l0 * v0.inv_w + l1 * v1.inv_w + l2 * v2.inv_w;
                    let w = if inv_w.abs() > 1e-9 {
                        1.0 / inv_w
                    } else {
                        1.0
                    };

                    // Interpolate lighting
                    let light = (l0 * v0.light + l1 * v1.light + l2 * v2.light).clamp(0.0, 1.0);

                    // Compute pixel color based on shading mode
                    let final_color = match (shading, texture) {
                        (ShadingMode::Textured | ShadingMode::TexturedLit, Some(tex)) => {
                            // Perspective-correct texture coordinates
                            let u_over_w = l0 * v0.u_over_w + l1 * v1.u_over_w + l2 * v2.u_over_w;
                            let v_over_w = l0 * v0.v_over_w + l1 * v1.v_over_w + l2 * v2.v_over_w;
                            let uv = Vec2::new(u_over_w * w, v_over_w * w);

                            let tex_color = tex.sample(uv, filter, WrapMode::Repeat);

                            if shading == ShadingMode::TexturedLit {
                                apply_light(tex_color, light)
                            } else {
                                tex_color
                            }
                        }
                        (ShadingMode::Gouraud, _) => apply_light(flat_color, light),
                        _ => flat_color,
                    };

                    unsafe {
                        fb.set_pixel_unchecked(ux, uy, final_color);
                    }

                    // Track damage
                    triangle_dirty = true;
                    dam_min_x = dam_min_x.min(x as u16);
                    dam_max_x = dam_max_x.max(x as u16);
                    dam_min_y = dam_min_y.min(y as u16);
                    dam_max_y = dam_max_y.max(y as u16);
                }
            }
        }
    }

    if triangle_dirty {
        let width = (dam_max_x - dam_min_x) + 1;
        let height = (dam_max_y - dam_min_y) + 1;
        fb.mark_damage_rect(DamageRect::new(dam_min_x, dam_min_y, width, height));
    }
}

/// Apply directional/ambient lighting multiplier to a 32-bit XRGB color.
#[inline(always)]
pub fn apply_light(color: u32, light: f32) -> u32 {
    let r = (((color >> 16) & 0xFF) as f32 * light).clamp(0.0, 255.0) as u32;
    let g = (((color >> 8) & 0xFF) as f32 * light).clamp(0.0, 255.0) as u32;
    let b = ((color & 0xFF) as f32 * light).clamp(0.0, 255.0) as u32;
    (r << 16) | (g << 8) | b
}
