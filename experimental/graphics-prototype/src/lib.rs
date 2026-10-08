//! ArenaOS Graphics Infrastructure Research & Prototyping
//!
//! Independent 3D software rendering engine, pipeline tests, and deterministic
//! animation proof for ArenaOS graphics capabilities.
//!
//! Provides:
//! - Complete 3D transformation and projection mathematics (`math`)
//! - Memory-safe framebuffer and damage region accumulation (`framebuffer`)
//! - Floating-point Z-buffer for depth testing and hidden surface removal (`zbuffer`)
//! - Texture mapping with nearest and bilinear filtering (`texture`)
//! - Barycentric triangle rasterizer with perspective-correct interpolation (`rasterizer`)
//! - Geometric mesh generation (`mesh`)
//! - Configurable 3D software rendering pipeline (`renderer`)
//! - Textured rotating 3D cube demo and animation sequence runner (`cube_demo`)
//! - Zero-copy adapter for ArenaOS desktop client surfaces (`arenaos_adapter`)

#![cfg_attr(not(feature = "std"), no_std)]

pub mod arenaos_adapter;
pub mod cube_demo;
pub mod framebuffer;
pub mod math;
pub mod mesh;
pub mod rasterizer;
pub mod renderer;
pub mod texture;
pub mod zbuffer;

pub use arenaos_adapter::{ArenaClientContext, WindowSurface};
pub use cube_demo::CubeDemo;
pub use framebuffer::{DamageRect, Framebuffer, PixelFormat};
pub use math::{Mat4, Vec2, Vec3, Vec4};
pub use mesh::Mesh;
pub use rasterizer::{CullMode, ShadingMode, WindingOrder};
pub use renderer::{ClipVertex, Renderer, RendererConfig, clip_triangle_near_plane, project_clip_to_screen};
pub use texture::{FilterMode, Texture, WrapMode};
pub use zbuffer::ZBuffer;
