//! 3D Software Rendering Pipeline.
//!
//! Transforms geometric meshes from model space to screen space, handles
//! vertex shading/lighting, perspective projection, and triangle rasterization.

#![allow(dead_code)]

use crate::framebuffer::Framebuffer;
use crate::math::{Mat4, Vec3, Vec4};
use crate::mesh::Mesh;
use crate::rasterizer::{ScreenVertex, ShadingMode, rasterize_triangle};
use crate::texture::{FilterMode, Texture};
use crate::zbuffer::ZBuffer;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RendererConfig {
    pub shading: ShadingMode,
    pub filter: FilterMode,
    pub light_dir: Vec3,
    pub ambient_light: f32,
    pub diffuse_light: f32,
}

impl Default for RendererConfig {
    fn default() -> Self {
        Self {
            shading: ShadingMode::TexturedLit,
            filter: FilterMode::Bilinear,
            light_dir: Vec3::new(0.5, 0.8, 1.0).normalize(),
            ambient_light: 0.25,
            diffuse_light: 0.75,
        }
    }
}

pub struct Renderer<'r, 'b> {
    fb: &'r mut Framebuffer<'b>,
    zb: &'r mut ZBuffer<'b>,
    view_proj: Mat4,
    width: f32,
    height: f32,
    config: RendererConfig,
}

impl<'r, 'b> Renderer<'r, 'b> {
    pub fn new(
        fb: &'r mut Framebuffer<'b>,
        zb: &'r mut ZBuffer<'b>,
        view: Mat4,
        proj: Mat4,
        config: RendererConfig,
    ) -> Self {
        let width = fb.width() as f32;
        let height = fb.height() as f32;
        let view_proj = proj.mul(&view);
        Self {
            fb,
            zb,
            view_proj,
            width,
            height,
            config,
        }
    }

    /// Clear both color and depth buffers.
    pub fn clear(&mut self, clear_color: u32, clear_depth: f32) {
        self.fb.clear(clear_color);
        self.zb.clear(clear_depth);
    }

    /// Draw a 3D mesh with model matrix and optional texture.
    pub fn draw_mesh(
        &mut self,
        mesh: &Mesh,
        model: &Mat4,
        texture: Option<&Texture<'_>>,
    ) {
        let mvp = self.view_proj.mul(model);
        let normal_matrix = *model; // For uniform scaling / rotation

        // Vertex transform and lighting buffer
        let mut screen_verts = [ScreenVertex {
            x: 0.0,
            y: 0.0,
            z: 0.0,
            inv_w: 0.0,
            u_over_w: 0.0,
            v_over_w: 0.0,
            light: 0.0,
        }; 24];

        for (i, v) in mesh.vertices.iter().enumerate() {
            // World normal for directional lighting
            let world_norm = normal_matrix
                .mul_vec4(Vec4::from_vec3(v.normal, 0.0))
                .to_vec3()
                .normalize();

            let dot = world_norm.dot(self.config.light_dir).max(0.0);
            let light = (self.config.ambient_light + dot * self.config.diffuse_light).clamp(0.0, 1.0);

            // Clip space coordinates
            let clip = mvp.mul_vec4(Vec4::from_vec3(v.pos, 1.0));

            // Guard against near-plane singularity
            let w = if clip.w.abs() < 1e-6 { 1e-6 } else { clip.w };
            let inv_w = 1.0 / w;

            // NDC [-1, 1]
            let ndc_x = clip.x * inv_w;
            let ndc_y = clip.y * inv_w;
            let ndc_z = clip.z * inv_w;

            // Viewport mapping: top-left screen origin
            let screen_x = (ndc_x + 1.0) * 0.5 * self.width;
            let screen_y = (1.0 - ndc_y) * 0.5 * self.height;
            let screen_z = (ndc_z + 1.0) * 0.5;

            screen_verts[i] = ScreenVertex {
                x: screen_x,
                y: screen_y,
                z: screen_z,
                inv_w,
                u_over_w: v.uv.x * inv_w,
                v_over_w: v.uv.y * inv_w,
                light,
            };
        }

        // Rasterize triangles
        for tri in &mesh.indices {
            let sv0 = screen_verts[tri.v0];
            let sv1 = screen_verts[tri.v1];
            let sv2 = screen_verts[tri.v2];

            rasterize_triangle(
                self.fb,
                self.zb,
                sv0,
                sv1,
                sv2,
                texture,
                self.config.shading,
                self.config.filter,
                tri.color,
            );
        }
    }
}
