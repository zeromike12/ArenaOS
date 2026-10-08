//! 3D Software Rendering Pipeline with Near-Plane Clipping and Face Culling.
//!
//! Transforms geometric meshes from model space to clip space, clips triangles against
//! the near plane (w >= near_plane_w) in homogeneous coordinates, performs perspective
//! division and viewport mapping, and rasterizes with depth buffering.

#![allow(dead_code)]

use crate::framebuffer::Framebuffer;
use crate::math::{Mat4, Vec2, Vec3, Vec4};
use crate::mesh::Mesh;
use crate::rasterizer::{
    CullMode, ScreenVertex, ShadingMode, WindingOrder, rasterize_triangle,
};
use crate::texture::{FilterMode, Texture};
use crate::zbuffer::ZBuffer;

/// Homogeneous clip-space vertex before perspective division.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ClipVertex {
    pub pos: Vec4,
    pub uv: Vec2,
    pub light: f32,
    pub color: u32,
}

impl ClipVertex {
    pub const fn new(pos: Vec4, uv: Vec2, light: f32, color: u32) -> Self {
        Self { pos, uv, light, color }
    }
}

/// Linear interpolation between two clip vertices.
#[inline]
pub fn lerp_clip_vertex(a: ClipVertex, b: ClipVertex, t: f32) -> ClipVertex {
    ClipVertex {
        pos: Vec4::new(
            a.pos.x + t * (b.pos.x - a.pos.x),
            a.pos.y + t * (b.pos.y - a.pos.y),
            a.pos.z + t * (b.pos.z - a.pos.z),
            a.pos.w + t * (b.pos.w - a.pos.w),
        ),
        uv: Vec2::new(
            a.uv.x + t * (b.uv.x - a.uv.x),
            a.uv.y + t * (b.uv.y - a.uv.y),
        ),
        light: a.light + t * (b.light - a.light),
        color: a.color,
    }
}

/// Clip a triangle against the near plane w >= near_w in homogeneous space.
///
/// Returns 0, 1, or 2 triangles (represented as up to 6 vertices).
pub fn clip_triangle_near_plane(
    v0: ClipVertex,
    v1: ClipVertex,
    v2: ClipVertex,
    near_w: f32,
) -> ([ClipVertex; 6], usize) {
    let mut out = [v0; 6];

    let in0 = v0.pos.w >= near_w;
    let in1 = v1.pos.w >= near_w;
    let in2 = v2.pos.w >= near_w;

    let inside_count = (in0 as usize) + (in1 as usize) + (in2 as usize);

    match inside_count {
        // Completely behind the near plane: discard
        0 => (out, 0),

        // Completely in front of the near plane: keep unmodified
        3 => {
            out[0] = v0;
            out[1] = v1;
            out[2] = v2;
            (out, 1)
        }

        // 1 vertex in front, 2 behind: produce 1 clipped triangle
        1 => {
            let (v_in, v_out1, v_out2) = if in0 {
                (v0, v1, v2)
            } else if in1 {
                (v1, v2, v0)
            } else {
                (v2, v0, v1)
            };

            let denom1 = v_out1.pos.w - v_in.pos.w;
            let t1 = if denom1.abs() > 1e-9 {
                (near_w - v_in.pos.w) / denom1
            } else {
                0.0
            };

            let denom2 = v_out2.pos.w - v_in.pos.w;
            let t2 = if denom2.abs() > 1e-9 {
                (near_w - v_in.pos.w) / denom2
            } else {
                0.0
            };

            out[0] = v_in;
            out[1] = lerp_clip_vertex(v_in, v_out1, t1);
            out[2] = lerp_clip_vertex(v_in, v_out2, t2);
            (out, 1)
        }

        // 2 vertices in front, 1 behind: produce a quad split into 2 clipped triangles
        2 => {
            let (v_out, v_in1, v_in2) = if !in0 {
                (v0, v1, v2)
            } else if !in1 {
                (v1, v2, v0)
            } else {
                (v2, v0, v1)
            };

            let denom1 = v_out.pos.w - v_in1.pos.w;
            let t1 = if denom1.abs() > 1e-9 {
                (near_w - v_in1.pos.w) / denom1
            } else {
                0.0
            };

            let denom2 = v_out.pos.w - v_in2.pos.w;
            let t2 = if denom2.abs() > 1e-9 {
                (near_w - v_in2.pos.w) / denom2
            } else {
                0.0
            };

            let i1 = lerp_clip_vertex(v_in1, v_out, t1);
            let i2 = lerp_clip_vertex(v_in2, v_out, t2);

            // Triangle 1: (v_in1, v_in2, i2)
            out[0] = v_in1;
            out[1] = v_in2;
            out[2] = i2;

            // Triangle 2: (v_in1, i2, i1)
            out[3] = v_in1;
            out[4] = i2;
            out[5] = i1;

            (out, 2)
        }

        _ => (out, 0),
    }
}

/// Project a homogeneous clip-space vertex into screen coordinates.
#[inline]
pub fn project_clip_to_screen(v: ClipVertex, width: f32, height: f32) -> ScreenVertex {
    let inv_w = 1.0 / v.pos.w;
    let ndc_x = v.pos.x * inv_w;
    let ndc_y = v.pos.y * inv_w;
    let ndc_z = v.pos.z * inv_w;

    ScreenVertex {
        x: (ndc_x + 1.0) * 0.5 * width,
        y: (1.0 - ndc_y) * 0.5 * height,
        z: (ndc_z + 1.0) * 0.5,
        inv_w,
        u_over_w: v.uv.x * inv_w,
        v_over_w: v.uv.y * inv_w,
        light: v.light,
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RendererConfig {
    pub shading: ShadingMode,
    pub filter: FilterMode,
    pub cull: CullMode,
    pub winding: WindingOrder,
    pub light_dir: Vec3,
    pub ambient_light: f32,
    pub diffuse_light: f32,
    pub near_plane_w: f32,
}

impl Default for RendererConfig {
    fn default() -> Self {
        Self {
            shading: ShadingMode::TexturedLit,
            filter: FilterMode::Bilinear,
            cull: CullMode::Back,
            winding: WindingOrder::CounterClockwise,
            light_dir: Vec3::new(0.5, 0.8, 1.0).normalize(),
            ambient_light: 0.25,
            diffuse_light: 0.75,
            near_plane_w: 0.1,
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
        let mut clip_verts = [ClipVertex::new(Vec4::ZERO, Vec2::ZERO, 0.0, 0); 24];

        for (i, v) in mesh.vertices.iter().enumerate() {
            // World normal for directional lighting
            let world_norm = normal_matrix
                .mul_vec4(Vec4::from_vec3(v.normal, 0.0))
                .to_vec3()
                .normalize();

            let dot = world_norm.dot(self.config.light_dir).max(0.0);
            let light = (self.config.ambient_light + dot * self.config.diffuse_light).clamp(0.0, 1.0);

            // Clip space coordinates before perspective divide
            let clip = mvp.mul_vec4(Vec4::from_vec3(v.pos, 1.0));

            clip_verts[i] = ClipVertex::new(clip, v.uv, light, v.color);
        }

        // Process triangles with near-plane homogeneous clipping and face culling
        for tri in &mesh.indices {
            let cv0 = clip_verts[tri.v0];
            let cv1 = clip_verts[tri.v1];
            let cv2 = clip_verts[tri.v2];

            let (clipped_triangles, count) =
                clip_triangle_near_plane(cv0, cv1, cv2, self.config.near_plane_w);

            for t in 0..count {
                let offset = t * 3;
                let sv0 = project_clip_to_screen(clipped_triangles[offset], self.width, self.height);
                let sv1 = project_clip_to_screen(clipped_triangles[offset + 1], self.width, self.height);
                let sv2 = project_clip_to_screen(clipped_triangles[offset + 2], self.width, self.height);

                rasterize_triangle(
                    self.fb,
                    self.zb,
                    sv0,
                    sv1,
                    sv2,
                    texture,
                    self.config.shading,
                    self.config.filter,
                    self.config.cull,
                    self.config.winding,
                    tri.color,
                );
            }
        }
    }
}
