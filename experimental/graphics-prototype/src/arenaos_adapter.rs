//! ArenaOS Desktop Client Adapter.
//!
//! Demonstrates how `Framebuffer` connects with zero copy to an ArenaOS
//! desktop `Client`'s SharedRegion mapping.

#![allow(dead_code)]

use crate::framebuffer::{DamageRect, Framebuffer, FramebufferError, PixelFormat};

/// Trait abstracting an ArenaOS window client surface without requiring
/// a direct build dependency on userspace desktop crates.
pub trait WindowSurface {
    /// Pointer to the mapped SharedRegion pixels (starting at PIXEL_OFFSET = 4096).
    fn pixel_slice_mut(&mut self) -> &mut [u32];
    fn width(&self) -> usize;
    fn height(&self) -> usize;
    fn stride(&self) -> usize;
    /// Publish damaged regions to the desktop compositor.
    fn publish_damage(&mut self, damage: DamageRect);
}

/// Zero-copy 3D rendering context attached to an ArenaOS window surface.
pub struct ArenaClientContext<'a, S: WindowSurface> {
    surface: &'a mut S,
}

impl<'a, S: WindowSurface> ArenaClientContext<'a, S> {
    pub fn new(surface: &'a mut S) -> Self {
        Self { surface }
    }

    /// Execute a rendering callback directly against the mapped SharedRegion memory.
    pub fn render_with<F, R>(&mut self, render_fn: F) -> Result<R, FramebufferError>
    where
        F: FnOnce(&mut Framebuffer<'_>) -> R,
    {
        let w = self.surface.width();
        let h = self.surface.height();
        let stride = self.surface.stride();
        let pixels = self.surface.pixel_slice_mut();

        let mut fb = Framebuffer::new(pixels, w, h, stride, PixelFormat::Xrgb8888)?;
        let result = render_fn(&mut fb);

        // If rendering produced damage, publish it to the compositor
        if let Some(damage) = fb.take_damage() {
            self.surface.publish_damage(damage);
        }

        Ok(result)
    }
}
