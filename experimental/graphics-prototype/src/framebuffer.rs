//! ArenaOS Framebuffer abstraction and damage region tracking.
//!
//! Conforms to the ArenaOS Native Graphics ABI proposal:
//! - 32-bit pixel words (XRGB8888 / ARGB8888)
//! - Explicit pixel stride (supports padding and alignment)
//! - Memory safety: bounds-checked memory access preventing out-of-slice writes
//! - Damage tracking: bounds calculation for regional compositor publication

#![allow(dead_code)]

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PixelFormat {
    Xrgb8888,
    Argb8888,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FramebufferError {
    InvalidDimensions,
    BufferTooSmall,
    OutOfBounds,
    MisalignedStride,
}

/// A damage rectangle in surface-local coordinates `[x, y, width, height]`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DamageRect {
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
}

impl DamageRect {
    pub const EMPTY: Self = Self {
        x: 0,
        y: 0,
        width: 0,
        height: 0,
    };

    pub fn new(x: u16, y: u16, width: u16, height: u16) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    pub fn union(self, other: Self) -> Self {
        if self.width == 0 || self.height == 0 {
            return other;
        }
        if other.width == 0 || other.height == 0 {
            return self;
        }
        let min_x = self.x.min(other.x);
        let min_y = self.y.min(other.y);
        let max_x = (self.x + self.width).max(other.x + other.width);
        let max_y = (self.y + self.height).max(other.y + other.height);
        Self {
            x: min_x,
            y: min_y,
            width: max_x - min_x,
            height: max_y - min_y,
        }
    }

    pub fn to_array(self) -> [u16; 4] {
        [self.x, self.y, self.width, self.height]
    }
}

/// Accumulator for damage regions to exercise the proposed damage publication ABI.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DamageAccumulator {
    bounds: DamageRect,
    dirty: bool,
}

impl DamageAccumulator {
    pub const fn new() -> Self {
        Self {
            bounds: DamageRect::EMPTY,
            dirty: false,
        }
    }

    pub fn record_point(&mut self, x: u16, y: u16) {
        if !self.dirty {
            self.bounds = DamageRect::new(x, y, 1, 1);
            self.dirty = true;
        } else {
            let min_x = self.bounds.x.min(x);
            let min_y = self.bounds.y.min(y);
            let max_x = (self.bounds.x + self.bounds.width).max(x + 1);
            let max_y = (self.bounds.y + self.bounds.height).max(y + 1);
            self.bounds = DamageRect {
                x: min_x,
                y: min_y,
                width: max_x - min_x,
                height: max_y - min_y,
            };
        }
    }

    pub fn record_rect(&mut self, rect: DamageRect) {
        if rect.width > 0 && rect.height > 0 {
            if !self.dirty {
                self.bounds = rect;
                self.dirty = true;
            } else {
                self.bounds = self.bounds.union(rect);
            }
        }
    }

    pub fn take_damage(&mut self) -> Option<DamageRect> {
        if self.dirty {
            self.dirty = false;
            let res = self.bounds;
            self.bounds = DamageRect::EMPTY;
            Some(res)
        } else {
            None
        }
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty
    }
}

/// Framebuffer viewing an underlying 32-bit pixel slice.
pub struct Framebuffer<'a> {
    pixels: &'a mut [u32],
    width: usize,
    height: usize,
    stride: usize,
    format: PixelFormat,
    damage: DamageAccumulator,
}

impl<'a> Framebuffer<'a> {
    /// Create a new Framebuffer over borrowed memory with validated dimensions.
    pub fn new(
        pixels: &'a mut [u32],
        width: usize,
        height: usize,
        stride: usize,
        format: PixelFormat,
    ) -> Result<Self, FramebufferError> {
        if width == 0 || height == 0 || stride < width {
            return Err(FramebufferError::InvalidDimensions);
        }
        let required_len = stride
            .checked_mul(height)
            .ok_or(FramebufferError::InvalidDimensions)?;
        if pixels.len() < required_len {
            return Err(FramebufferError::BufferTooSmall);
        }
        Ok(Self {
            pixels,
            width,
            height,
            stride,
            format,
            damage: DamageAccumulator::new(),
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

    #[inline(always)]
    pub fn stride(&self) -> usize {
        self.stride
    }

    #[inline(always)]
    pub fn format(&self) -> PixelFormat {
        self.format
    }

    /// Clear the visible framebuffer with a solid color, recording full damage.
    pub fn clear(&mut self, color: u32) {
        if self.stride == self.width {
            let total = self.width * self.height;
            self.pixels[..total].fill(color);
        } else {
            for y in 0..self.height {
                let start = y * self.stride;
                self.pixels[start..start + self.width].fill(color);
            }
        }
        self.damage.record_rect(DamageRect::new(
            0,
            0,
            self.width as u16,
            self.height as u16,
        ));
    }

    /// Plot a pixel safely with bounds checking and damage tracking.
    #[inline(always)]
    pub fn set_pixel(&mut self, x: usize, y: usize, color: u32) {
        if x < self.width && y < self.height {
            let idx = y * self.stride + x;
            self.pixels[idx] = color;
            self.damage.record_point(x as u16, y as u16);
        }
    }

    /// Read a pixel value.
    #[inline(always)]
    pub fn get_pixel(&self, x: usize, y: usize) -> Option<u32> {
        if x < self.width && y < self.height {
            Some(self.pixels[y * self.stride + x])
        } else {
            None
        }
    }

    /// Fast unchecked pixel write for inner rasterizer loops after clip testing.
    #[inline(always)]
    pub unsafe fn set_pixel_unchecked(&mut self, x: usize, y: usize, color: u32) {
        let idx = y * self.stride + x;
        unsafe {
            *self.pixels.get_unchecked_mut(idx) = color;
        }
    }

    /// Record damage for a known bounding box (e.g. from rasterized triangle).
    pub fn mark_damage_rect(&mut self, rect: DamageRect) {
        self.damage.record_rect(rect);
    }

    /// Retrieve and clear accumulated damage for presentation.
    pub fn take_damage(&mut self) -> Option<DamageRect> {
        self.damage.take_damage()
    }

    /// Access the underlying pixel slice.
    pub fn raw_slice(&self) -> &[u32] {
        self.pixels
    }

    /// Access the mutable underlying pixel slice.
    pub fn raw_slice_mut(&mut self) -> &mut [u32] {
        self.pixels
    }

    /// Write output as standard ASCII/Binary PPM format (Netpbm P6).
    pub fn write_ppm_p6(&self, out: &mut [u8]) -> Result<usize, FramebufferError> {
        let (header, header_len) = format_ppm_header(self.width, self.height);
        let header_bytes = &header[..header_len];
        let payload_len = self.width * self.height * 3;
        let total_len = header_len + payload_len;
        if out.len() < total_len {
            return Err(FramebufferError::BufferTooSmall);
        }

        out[..header_len].copy_from_slice(header_bytes);
        let mut offset = header_len;

        for y in 0..self.height {
            let row_start = y * self.stride;
            for x in 0..self.width {
                let pixel = self.pixels[row_start + x];
                let r = ((pixel >> 16) & 0xFF) as u8;
                let g = ((pixel >> 8) & 0xFF) as u8;
                let b = (pixel & 0xFF) as u8;
                out[offset] = r;
                out[offset + 1] = g;
                out[offset + 2] = b;
                offset += 3;
            }
        }

        Ok(total_len)
    }

    /// Compute 32-bit CRC or FNV-1a hash over rendered pixels for deterministic image tests.
    pub fn compute_pixel_hash(&self) -> u64 {
        let mut hash: u64 = 0xcbf29ce484222325; // FNV offset basis
        for y in 0..self.height {
            let row_start = y * self.stride;
            for x in 0..self.width {
                let pixel = self.pixels[row_start + x];
                for shift in [0, 8, 16, 24] {
                    let byte = ((pixel >> shift) & 0xFF) as u8;
                    hash ^= byte as u64;
                    hash = hash.wrapping_mul(0x100000001b3); // FNV prime
                }
            }
        }
        hash
    }
}

/// Format PPM P6 ASCII header without standard library formatting.
fn format_ppm_header(width: usize, height: usize) -> ([u8; 32], usize) {
    let mut buf = [0u8; 32];
    let prefix = b"P6\n";
    let mut idx = 0;
    for &b in prefix {
        buf[idx] = b;
        idx += 1;
    }
    idx += write_usize_ascii(&mut buf[idx..], width);
    buf[idx] = b' ';
    idx += 1;
    idx += write_usize_ascii(&mut buf[idx..], height);
    buf[idx] = b'\n';
    idx += 1;
    let maxval = b"255\n";
    for &b in maxval {
        buf[idx] = b;
        idx += 1;
    }
    (buf, idx)
}

fn write_usize_ascii(buf: &mut [u8], mut val: usize) -> usize {
    if val == 0 {
        buf[0] = b'0';
        return 1;
    }
    let mut temp = [0u8; 10];
    let mut len = 0;
    while val > 0 {
        temp[len] = b'0' + (val % 10) as u8;
        val /= 10;
        len += 1;
    }
    for i in 0..len {
        buf[i] = temp[len - 1 - i];
    }
    len
}
