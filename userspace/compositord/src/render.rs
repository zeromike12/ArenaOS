//! Userspace-only bounded scanout composer. The server validates each mapped
//! SharedRegion by held cap before constructing a Layer; raw pointers are
//! NEVER nominated by an IPC client. This module grants no authority.
use core::ptr;

pub const MAX_LAYERS: usize = 6;
pub const MAX_SCREEN_WIDTH: usize = 1024;
pub const MAX_SCREEN_HEIGHT: usize = 768;

#[derive(Clone, Copy)]
pub struct Layer {
    pub pixels: *const u32,
    pub pixel_len: usize,
    pub width: usize,
    pub height: usize,
    pub x: i32,
    pub y: i32,
    pub z: u64,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Error {
    Bounds,
    Backing,
    Order,
}

/// Build a complete opaque frame from an immutable captured base and
/// distinct, pinned client mappings; ascending z, with signed clipping.
/// Validate EVERY layer before any destination write (transactional refusal).
///
/// # Safety
/// `dest` identifies at least `dest_len` writable mapped pixels and does
/// not alias `base` or any layer. Each non-null layer points to at least
/// `pixel_len` readable pinned pixels for the duration of the call. The
/// caller verifies the full generation and rights of every mapping's cap.
pub unsafe fn compose(
    base: &[u32],
    dest: *mut u32,
    dest_len: usize,
    width: usize,
    height: usize,
    layers: &[Layer],
) -> Result<(), Error> {
    let needed = width.checked_mul(height).ok_or(Error::Bounds)?;
    if width == 0
        || height == 0
        || width > MAX_SCREEN_WIDTH
        || height > MAX_SCREEN_HEIGHT
        || needed > dest_len
        || needed > base.len()
        || dest.is_null()
        || layers.len() > MAX_LAYERS
    {
        return Err(Error::Bounds);
    }
    for (i, layer) in layers.iter().enumerate() {
        let required = layer
            .width
            .checked_mul(layer.height)
            .ok_or(Error::Backing)?;
        if layer.pixels.is_null()
            || layer.width == 0
            || layer.height == 0
            || layer.width > 320
            || layer.height > 200
            || required > layer.pixel_len
        {
            return Err(Error::Backing);
        }
        if layer.z == 0 || layers[..i].iter().any(|earlier| earlier.z == layer.z) {
            return Err(Error::Order);
        }
    }
    for (i, color) in base[..needed].iter().enumerate() {
        // SAFETY: dest points to >=needed distinct writable mapped pixels.
        unsafe { ptr::write_volatile(dest.add(i), *color) };
    }
    let mut after = 0;
    for _ in layers {
        let layer = layers
            .iter()
            .filter(|l| l.z > after)
            .min_by_key(|l| l.z)
            .ok_or(Error::Order)?;
        after = layer.z;
        let x0 = (layer.x as i64).clamp(0, width as i64);
        let y0 = (layer.y as i64).clamp(0, height as i64);
        let x1 = (layer.x as i64 + layer.width as i64).clamp(0, width as i64);
        let y1 = (layer.y as i64 + layer.height as i64).clamp(0, height as i64);
        for y in y0..y1 {
            for x in x0..x1 {
                let src =
                    (y - layer.y as i64) as usize * layer.width + (x - layer.x as i64) as usize;
                let dst = y as usize * width + x as usize;
                // SAFETY: both checked dimensions and intersections are
                // within their pinned, disjoint readable/writable regions.
                unsafe {
                    ptr::write_volatile(dest.add(dst), ptr::read_volatile(layer.pixels.add(src)));
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn layer(pixels: &[u32], width: usize, height: usize, x: i32, y: i32, z: u64) -> Layer {
        Layer {
            pixels: pixels.as_ptr(),
            pixel_len: pixels.len(),
            width,
            height,
            x,
            y,
            z,
        }
    }
    #[test]
    fn overlapping_surfaces_use_z_and_preserve_base() {
        let base = [0x010203; 16];
        let a = [0x111111; 9];
        let b = [0x222222; 4];
        let mut dst = [0; 16];
        // Input enumeration is reversed. Visual order must be based on z.
        unsafe {
            compose(
                &base,
                dst.as_mut_ptr(),
                16,
                4,
                4,
                &[layer(&b, 2, 2, 1, 1, 9), layer(&a, 3, 3, 0, 0, 4)],
            )
        }
        .unwrap();
        assert_eq!(dst[0], 0x111111);
        assert_eq!(dst[5], 0x222222);
        assert_eq!(dst[15], 0x010203);
        assert_eq!(dst[10], 0x222222);
    }
    #[test]
    fn negative_clip_and_offscreen_leave_uncovered_base() {
        let base = [7; 16];
        let src = [8; 9];
        let mut out = [0; 16];
        unsafe {
            compose(
                &base,
                out.as_mut_ptr(),
                16,
                4,
                4,
                &[
                    layer(&src, 3, 3, -2, -2, 1),
                    layer(&src, 3, 3, i32::MAX, i32::MAX, 2),
                ],
            )
        }
        .unwrap();
        assert_eq!(out[0], 8);
        assert_eq!(out[1], 7);
        assert_eq!(out[15], 7);
    }
    #[test]
    fn preflight_does_not_publish_partial_frame() {
        let base = [1; 4];
        let src = [2; 4];
        let mut out = [3; 4];
        let mut bad = layer(&src, 2, 2, 0, 0, 4);
        bad.pixel_len = 3;
        assert_eq!(
            unsafe { compose(&base, out.as_mut_ptr(), 4, 2, 2, &[bad]) },
            Err(Error::Backing)
        );
        assert_eq!(out, [3; 4]);
        bad.pixel_len = 4;
        assert_eq!(
            unsafe { compose(&base, out.as_mut_ptr(), 4, 2, 2, &[bad, bad]) },
            Err(Error::Order)
        );
        assert_eq!(out, [3; 4]);
    }
}
