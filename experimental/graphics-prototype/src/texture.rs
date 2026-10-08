//! Texture representation and sampling for 3D software rendering.
//!
//! Supports procedural texture generation (checkerboards, test patterns),
//! nearest-neighbor and bilinear texture filtering, and wrap modes.

#![allow(dead_code)]

use crate::math::{Vec2, det_floor};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FilterMode {
    Nearest,
    Bilinear,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WrapMode {
    Repeat,
    Clamp,
}

pub struct Texture<'a> {
    pixels: &'a [u32],
    width: usize,
    height: usize,
}

impl<'a> Texture<'a> {
    pub const fn new(pixels: &'a [u32], width: usize, height: usize) -> Self {
        Self {
            pixels,
            width,
            height,
        }
    }

    #[inline(always)]
    pub fn width(&self) -> usize {
        self.width
    }

    #[inline(always)]
    pub fn height(&self) -> usize {
        self.height
    }

    /// Read raw texel with wrap mode.
    #[inline(always)]
    pub fn get_texel(&self, mut x: i32, mut y: i32, wrap: WrapMode) -> u32 {
        let w = self.width as i32;
        let h = self.height as i32;
        match wrap {
            WrapMode::Repeat => {
                x = x.rem_euclid(w);
                y = y.rem_euclid(h);
            }
            WrapMode::Clamp => {
                x = x.clamp(0, w - 1);
                y = y.clamp(0, h - 1);
            }
        }
        self.pixels[(y as usize) * self.width + (x as usize)]
    }

    /// Sample texture at normalized UV coordinates in [0, 1].
    pub fn sample(&self, uv: Vec2, filter: FilterMode, wrap: WrapMode) -> u32 {
        match filter {
            FilterMode::Nearest => {
                let fx = uv.x * (self.width as f32);
                let fy = uv.y * (self.height as f32);
                let x = det_floor(fx) as i32;
                let y = det_floor(fy) as i32;
                self.get_texel(x, y, wrap)
            }
            FilterMode::Bilinear => {
                let fx = uv.x * (self.width as f32) - 0.5;
                let fy = uv.y * (self.height as f32) - 0.5;
                let x0 = det_floor(fx) as i32;
                let y0 = det_floor(fy) as i32;
                let x1 = x0 + 1;
                let y1 = y0 + 1;

                let tx = fx - det_floor(fx);
                let ty = fy - det_floor(fy);

                let c00 = self.get_texel(x0, y0, wrap);
                let c10 = self.get_texel(x1, y0, wrap);
                let c01 = self.get_texel(x0, y1, wrap);
                let c11 = self.get_texel(x1, y1, wrap);

                bilinear_blend(c00, c10, c01, c11, tx, ty)
            }
        }
    }
}

/// Bilinear interpolation of four 32-bit XRGB color channels.
fn bilinear_blend(c00: u32, c10: u32, c01: u32, c11: u32, tx: f32, ty: f32) -> u32 {
    let mut out = 0u32;
    for shift in [0, 8, 16] {
        let v00 = ((c00 >> shift) & 0xFF) as f32;
        let v10 = ((c10 >> shift) & 0xFF) as f32;
        let v01 = ((c01 >> shift) & 0xFF) as f32;
        let v11 = ((c11 >> shift) & 0xFF) as f32;

        let top = v00 * (1.0 - tx) + v10 * tx;
        let bot = v01 * (1.0 - tx) + v11 * tx;
        let final_val = (top * (1.0 - ty) + bot * ty).clamp(0.0, 255.0) as u32;

        out |= final_val << shift;
    }
    out
}

/// Procedural checkerboard generator.
pub fn generate_checkerboard(
    out: &mut [u32],
    width: usize,
    height: usize,
    check_size: usize,
    color1: u32,
    color2: u32,
) {
    for y in 0..height {
        let check_y = (y / check_size) % 2;
        for x in 0..width {
            let check_x = (x / check_size) % 2;
            let color = if (check_x ^ check_y) == 0 {
                color1
            } else {
                color2
            };
            out[y * width + x] = color;
        }
    }
}

/// Procedural test pattern with coordinate grid and colored quadrants.
pub fn generate_test_pattern(out: &mut [u32], width: usize, height: usize) {
    let mid_x = width / 2;
    let mid_y = height / 2;
    for y in 0..height {
        for x in 0..width {
            let mut color = match (x < mid_x, y < mid_y) {
                (true, true) => 0x00_E3_35_42,    // Red quadrant (ArenaOS palette)
                (false, true) => 0x00_2E_C7_71,   // Green quadrant
                (true, false) => 0x00_3B_67_E1,  // Blue quadrant
                (false, false) => 0x00_F8_EE_CC, // Light accent quadrant
            };
            // Border or grid line
            if x == 0 || x == width - 1 || y == 0 || y == height - 1 || x == mid_x || y == mid_y {
                color = 0x00_11_18_27; // Dark border
            }
            out[y * width + x] = color;
        }
    }
}
