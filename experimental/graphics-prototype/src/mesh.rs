//! 3D Mesh representation and geometric primitives.

#![allow(dead_code)]

use crate::math::{Vec2, Vec3};
use crate::rasterizer::Vertex;

pub struct TriangleIndices {
    pub v0: usize,
    pub v1: usize,
    pub v2: usize,
    pub color: u32,
}

pub struct Mesh {
    pub vertices: [Vertex; 24],
    pub indices: [TriangleIndices; 12],
}

impl Mesh {
    /// Create a textured 3D cube with size `s`.
    /// 24 distinct vertices (4 per face) allow independent normals and UV mapping.
    pub fn create_cube(s: f32) -> Self {
        let h = s * 0.5;

        // Distinct colors per face for flat/Gouraud testing (ArenaOS palette inspired)
        let col_front = 0x00_E3_35_42;  // Crimson Red
        let col_back = 0x00_2E_C7_71;   // Emerald Green
        let col_top = 0x00_3B_67_E1;    // Royal Blue
        let col_bottom = 0x00_F8_EE_CC; // Light Cream
        let col_right = 0x00_F5_A6_23;  // Amber Orange
        let col_left = 0x00_90_13_FE;   // Violet

        let vertices = [
            // Front face (+Z): normal (0, 0, 1)
            Vertex::new(Vec3::new(-h, -h,  h), Vec2::new(0.0, 1.0), Vec3::new(0.0, 0.0, 1.0), col_front),
            Vertex::new(Vec3::new( h, -h,  h), Vec2::new(1.0, 1.0), Vec3::new(0.0, 0.0, 1.0), col_front),
            Vertex::new(Vec3::new( h,  h,  h), Vec2::new(1.0, 0.0), Vec3::new(0.0, 0.0, 1.0), col_front),
            Vertex::new(Vec3::new(-h,  h,  h), Vec2::new(0.0, 0.0), Vec3::new(0.0, 0.0, 1.0), col_front),

            // Back face (-Z): normal (0, 0, -1)
            Vertex::new(Vec3::new( h, -h, -h), Vec2::new(0.0, 1.0), Vec3::new(0.0, 0.0, -1.0), col_back),
            Vertex::new(Vec3::new(-h, -h, -h), Vec2::new(1.0, 1.0), Vec3::new(0.0, 0.0, -1.0), col_back),
            Vertex::new(Vec3::new(-h,  h, -h), Vec2::new(1.0, 0.0), Vec3::new(0.0, 0.0, -1.0), col_back),
            Vertex::new(Vec3::new( h,  h, -h), Vec2::new(0.0, 0.0), Vec3::new(0.0, 0.0, -1.0), col_back),

            // Top face (+Y): normal (0, 1, 0)
            Vertex::new(Vec3::new(-h,  h,  h), Vec2::new(0.0, 1.0), Vec3::new(0.0, 1.0, 0.0), col_top),
            Vertex::new(Vec3::new( h,  h,  h), Vec2::new(1.0, 1.0), Vec3::new(0.0, 1.0, 0.0), col_top),
            Vertex::new(Vec3::new( h,  h, -h), Vec2::new(1.0, 0.0), Vec3::new(0.0, 1.0, 0.0), col_top),
            Vertex::new(Vec3::new(-h,  h, -h), Vec2::new(0.0, 0.0), Vec3::new(0.0, 1.0, 0.0), col_top),

            // Bottom face (-Y): normal (0, -1, 0)
            Vertex::new(Vec3::new(-h, -h, -h), Vec2::new(0.0, 1.0), Vec3::new(0.0, -1.0, 0.0), col_bottom),
            Vertex::new(Vec3::new( h, -h, -h), Vec2::new(1.0, 1.0), Vec3::new(0.0, -1.0, 0.0), col_bottom),
            Vertex::new(Vec3::new( h, -h,  h), Vec2::new(1.0, 0.0), Vec3::new(0.0, -1.0, 0.0), col_bottom),
            Vertex::new(Vec3::new(-h, -h,  h), Vec2::new(0.0, 0.0), Vec3::new(0.0, -1.0, 0.0), col_bottom),

            // Right face (+X): normal (1, 0, 0)
            Vertex::new(Vec3::new( h, -h,  h), Vec2::new(0.0, 1.0), Vec3::new(1.0, 0.0, 0.0), col_right),
            Vertex::new(Vec3::new( h, -h, -h), Vec2::new(1.0, 1.0), Vec3::new(1.0, 0.0, 0.0), col_right),
            Vertex::new(Vec3::new( h,  h, -h), Vec2::new(1.0, 0.0), Vec3::new(1.0, 0.0, 0.0), col_right),
            Vertex::new(Vec3::new( h,  h,  h), Vec2::new(0.0, 0.0), Vec3::new(1.0, 0.0, 0.0), col_right),

            // Left face (-X): normal (-1, 0, 0)
            Vertex::new(Vec3::new(-h, -h, -h), Vec2::new(0.0, 1.0), Vec3::new(-1.0, 0.0, 0.0), col_left),
            Vertex::new(Vec3::new(-h, -h,  h), Vec2::new(1.0, 1.0), Vec3::new(-1.0, 0.0, 0.0), col_left),
            Vertex::new(Vec3::new(-h,  h,  h), Vec2::new(1.0, 0.0), Vec3::new(-1.0, 0.0, 0.0), col_left),
            Vertex::new(Vec3::new(-h,  h, -h), Vec2::new(0.0, 0.0), Vec3::new(-1.0, 0.0, 0.0), col_left),
        ];

        let indices = [
            // Front (0, 1, 2, 3)
            TriangleIndices { v0: 0, v1: 1, v2: 2, color: col_front },
            TriangleIndices { v0: 0, v1: 2, v2: 3, color: col_front },
            // Back (4, 5, 6, 7)
            TriangleIndices { v0: 4, v1: 5, v2: 6, color: col_back },
            TriangleIndices { v0: 4, v1: 6, v2: 7, color: col_back },
            // Top (8, 9, 10, 11)
            TriangleIndices { v0: 8, v1: 9, v2: 10, color: col_top },
            TriangleIndices { v0: 8, v1: 10, v2: 11, color: col_top },
            // Bottom (12, 13, 14, 15)
            TriangleIndices { v0: 12, v1: 13, v2: 14, color: col_bottom },
            TriangleIndices { v0: 12, v1: 14, v2: 15, color: col_bottom },
            // Right (16, 17, 18, 19)
            TriangleIndices { v0: 16, v1: 17, v2: 18, color: col_right },
            TriangleIndices { v0: 16, v1: 18, v2: 19, color: col_right },
            // Left (20, 21, 22, 23)
            TriangleIndices { v0: 20, v1: 21, v2: 22, color: col_left },
            TriangleIndices { v0: 20, v1: 22, v2: 23, color: col_left },
        ];

        Self { vertices, indices }
    }
}
