//! Floating-point Z-Buffer for hidden surface removal in software rasterization.
//!
//! Provides deterministic depth testing with bounds checking and memory reuse.

#![allow(dead_code)]

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ZBufferError {
    InvalidDimensions,
    BufferTooSmall,
}

pub struct ZBuffer<'a> {
    depths: &'a mut [f32],
    width: usize,
    height: usize,
}

impl<'a> ZBuffer<'a> {
    /// Create a new ZBuffer wrapping caller-provided storage.
    pub fn new(
        depths: &'a mut [f32],
        width: usize,
        height: usize,
    ) -> Result<Self, ZBufferError> {
        if width == 0 || height == 0 {
            return Err(ZBufferError::InvalidDimensions);
        }
        let required_len = width
            .checked_mul(height)
            .ok_or(ZBufferError::InvalidDimensions)?;
        if depths.len() < required_len {
            return Err(ZBufferError::BufferTooSmall);
        }
        Ok(Self {
            depths,
            width,
            height,
        })
    }

    #[inline(always)]
    pub fn width(&self) -> usize {
        self.width
    }

    #[inline(always)]
    pub fn height(&self) -> usize {
        self.height
    }

    /// Reset all depth values to the given far plane (typically 1.0).
    pub fn clear(&mut self, depth: f32) {
        let total = self.width * self.height;
        self.depths[..total].fill(depth);
    }

    /// Perform depth test at (x, y). If incoming z < stored z, update and return true.
    #[inline(always)]
    pub fn test_and_set(&mut self, x: usize, y: usize, z: f32) -> bool {
        if x < self.width && y < self.height {
            let idx = y * self.width + x;
            let current = self.depths[idx];
            if z < current {
                self.depths[idx] = z;
                true
            } else {
                false
            }
        } else {
            false
        }
    }

    /// Fast unchecked depth test for inner loops after clipping.
    #[inline(always)]
    pub unsafe fn test_and_set_unchecked(&mut self, x: usize, y: usize, z: f32) -> bool {
        let idx = y * self.width + x;
        unsafe {
            let current = *self.depths.get_unchecked(idx);
            if z < current {
                *self.depths.get_unchecked_mut(idx) = z;
                true
            } else {
                false
            }
        }
    }

    /// Read depth value at (x, y).
    #[inline(always)]
    pub fn get_depth(&self, x: usize, y: usize) -> Option<f32> {
        if x < self.width && y < self.height {
            Some(self.depths[y * self.width + x])
        } else {
            None
        }
    }
}
