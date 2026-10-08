//! Textured Rotating 3D Cube Demo and Animation Harness.
//!
//! Provides deterministic animation generation, performance and memory metrics,
//! and verification hashes matching the ArenaOS qualification style.

#![allow(dead_code)]

use crate::framebuffer::{DamageRect, Framebuffer, PixelFormat};
use crate::math::{Mat4, PI, Vec3};
use crate::mesh::Mesh;
use crate::renderer::{Renderer, RendererConfig};
use crate::texture::{Texture, generate_checkerboard, generate_test_pattern};
use crate::zbuffer::ZBuffer;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FrameReceipt {
    pub frame_index: usize,
    pub damage_rect: Option<DamageRect>,
    pub pixel_hash: u64,
    pub render_time_micros: u64,
    pub memory_bytes: usize,
}

pub struct CubeDemo {
    pub width: usize,
    pub height: usize,
    pub stride: usize,
    mesh: Mesh,
    pub texture_buffer: [u32; 64 * 64],
}

impl CubeDemo {
    pub fn new(width: usize, height: usize, stride: usize) -> Self {
        let mut texture_buffer = [0u32; 64 * 64];
        generate_test_pattern(&mut texture_buffer, 64, 64);
        Self {
            width,
            height,
            stride,
            mesh: Mesh::create_cube(1.6),
            texture_buffer,
        }
    }

    /// Set a custom checkerboard texture.
    pub fn set_checkerboard(&mut self, check_size: usize, col1: u32, col2: u32) {
        generate_checkerboard(&mut self.texture_buffer, 64, 64, check_size, col1, col2);
    }

    /// Calculate total memory footprint for the rendering session in bytes.
    pub fn memory_footprint(&self) -> usize {
        let fb_bytes = self.stride * self.height * 4;
        let zb_bytes = self.width * self.height * 4;
        let tex_bytes = 64 * 64 * 4;
        let mesh_bytes = core::mem::size_of::<Mesh>();
        fb_bytes + zb_bytes + tex_bytes + mesh_bytes
    }

    /// Render a single animation frame at the given angle step.
    pub fn render_frame(
        &self,
        frame_idx: usize,
        fb_pixels: &mut [u32],
        zb_depths: &mut [f32],
        config: RendererConfig,
    ) -> FrameReceipt {
        let mem = self.memory_footprint();

        let mut fb = Framebuffer::new(
            fb_pixels,
            self.width,
            self.height,
            self.stride,
            PixelFormat::Xrgb8888,
        )
        .expect("Valid Framebuffer allocation");

        let mut zb = ZBuffer::new(zb_depths, self.width, self.height)
            .expect("Valid ZBuffer allocation");

        // Camera setup
        let eye = Vec3::new(0.0, 1.2, 3.5);
        let center = Vec3::new(0.0, 0.0, 0.0);
        let up = Vec3::new(0.0, 1.0, 0.0);
        let view = Mat4::look_at(eye, center, up);

        let aspect = (self.width as f32) / (self.height as f32);
        let proj = Mat4::perspective(60.0 * (PI / 180.0), aspect, 0.1, 100.0);

        // Rotation angles based on frame index (smooth deterministic rotation)
        let angle_step = 0.08;
        let angle_y = (frame_idx as f32) * angle_step;
        let angle_x = (frame_idx as f32) * (angle_step * 0.6);
        let angle_z = (frame_idx as f32) * (angle_step * 0.3);

        let rot_x = Mat4::rotation_x(angle_x);
        let rot_y = Mat4::rotation_y(angle_y);
        let rot_z = Mat4::rotation_z(angle_z);
        let model = rot_y.mul(&rot_x).mul(&rot_z);

        let tex = Texture::new(&self.texture_buffer, 64, 64);

        // Background dark slate color (matching ArenaOS dark shell: 0x00_1a_23_32)
        let bg_color = 0x00_1A_23_32;

        let start_time = current_time_micros();

        {
            let mut renderer = Renderer::new(&mut fb, &mut zb, view, proj, config);
            renderer.clear(bg_color, 1.0);
            renderer.draw_mesh(&self.mesh, &model, Some(&tex));
        }

        let damage = fb.take_damage();
        let pixel_hash = fb.compute_pixel_hash();
        let elapsed = current_time_micros().saturating_sub(start_time);

        FrameReceipt {
            frame_index: frame_idx,
            damage_rect: damage,
            pixel_hash,
            render_time_micros: elapsed,
            memory_bytes: mem,
        }
    }

    /// Render a multi-frame animation sequence.
    pub fn render_animation(
        &self,
        frame_count: usize,
        fb_pixels: &mut [u32],
        zb_depths: &mut [f32],
        config: RendererConfig,
    ) -> [FrameReceipt; 16] {
        let mut receipts = [FrameReceipt {
            frame_index: 0,
            damage_rect: None,
            pixel_hash: 0,
            render_time_micros: 0,
            memory_bytes: 0,
        }; 16];

        let count = frame_count.min(16);
        for i in 0..count {
            receipts[i] = self.render_frame(i, fb_pixels, zb_depths, config);
        }
        receipts
    }
}

/// Helper to get microsecond timestamps when std is available, or deterministic tick in no_std.
fn current_time_micros() -> u64 {
    #[cfg(feature = "std")]
    {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_micros() as u64)
            .unwrap_or(0)
    }
    #[cfg(not(feature = "std"))]
    {
        0
    }
}
