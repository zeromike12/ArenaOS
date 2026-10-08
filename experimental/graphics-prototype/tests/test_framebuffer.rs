//! Framebuffer, stride, bounds, damage tracking, format tests, and client adapter.

use arena_graphics_prototype::arenaos_adapter::{ArenaClientContext, WindowSurface};
use arena_graphics_prototype::framebuffer::{
    DamageRect, Framebuffer, FramebufferError, PixelFormat,
};

#[test]
fn test_framebuffer_allocation_and_rejections() {
    let mut storage = [0u32; 100];

    // Zero dimension rejection
    assert_eq!(
        Framebuffer::new(&mut storage, 0, 10, 10, PixelFormat::Xrgb8888).err(),
        Some(FramebufferError::InvalidDimensions)
    );
    assert_eq!(
        Framebuffer::new(&mut storage, 10, 0, 10, PixelFormat::Xrgb8888).err(),
        Some(FramebufferError::InvalidDimensions)
    );

    // Stride less than width rejection
    assert_eq!(
        Framebuffer::new(&mut storage, 10, 10, 8, PixelFormat::Xrgb8888).err(),
        Some(FramebufferError::InvalidDimensions)
    );

    // Buffer too small rejection: 10 * 11 = 110 > 100
    assert_eq!(
        Framebuffer::new(&mut storage, 10, 11, 10, PixelFormat::Xrgb8888).err(),
        Some(FramebufferError::BufferTooSmall)
    );

    // Valid exact allocation
    assert!(
        Framebuffer::new(&mut storage, 10, 10, 10, PixelFormat::Xrgb8888).is_ok()
    );
}

#[test]
fn test_framebuffer_stride_isolation() {
    // Stride 16, width 10, height 4: 64 words total
    // Pixels at x >= 10 are padding / pitch margin and must never be corrupted
    let mut storage = [0xAA_AA_AA_AA; 64];
    {
        let mut fb = Framebuffer::new(
            &mut storage,
            10,
            4,
            16,
            PixelFormat::Xrgb8888,
        )
        .expect("Valid framebuffer");

        fb.clear(0x00_11_22_33);

        // Check visible pixels vs stride padding
        for y in 0..4 {
            for x in 0..10 {
                assert_eq!(
                    fb.get_pixel(x, y),
                    Some(0x00_11_22_33),
                    "Visible pixel at ({x}, {y}) should be cleared"
                );
            }
        }
    }

    // Verify raw padding bytes were untouched by clear()
    for y in 0..4 {
        for x in 10..16 {
            assert_eq!(
                storage[y * 16 + x],
                0xAA_AA_AA_AA,
                "Padding at ({x}, {y}) must remain untouched"
            );
        }
    }
}

#[test]
fn test_framebuffer_bounds_and_damage() {
    let mut storage = [0u32; 100];
    let mut fb = Framebuffer::new(
        &mut storage,
        10,
        10,
        10,
        PixelFormat::Xrgb8888,
    )
    .expect("Valid framebuffer");

    // Initially undamaged
    assert_eq!(fb.take_damage(), None);

    // Out-of-bounds writes are safely ignored and do not trigger damage
    fb.set_pixel(15, 5, 0xFF);
    fb.set_pixel(5, 15, 0xFF);
    assert_eq!(fb.get_pixel(15, 5), None);
    assert_eq!(fb.take_damage(), None);

    // Write at (2, 3) and (7, 8)
    fb.set_pixel(2, 3, 0x00_AA_BB_CC);
    fb.set_pixel(7, 8, 0x00_DD_EE_FF);

    assert_eq!(fb.get_pixel(2, 3), Some(0x00_AA_BB_CC));
    assert_eq!(fb.get_pixel(7, 8), Some(0x00_DD_EE_FF));

    // Bounding box should be: min_x = 2, min_y = 3, max_x = 8, max_y = 9
    // width = 8 - 2 = 6, height = 9 - 3 = 6
    let dmg = fb.take_damage().expect("Damage must be recorded");
    assert_eq!(dmg, DamageRect::new(2, 3, 6, 6));

    // After take_damage(), accumulator is reset
    assert_eq!(fb.take_damage(), None);
}

#[test]
fn test_ppm_p6_export() {
    let mut storage = [0u32; 4]; // 2x2 image
    storage[0] = 0x00_FF_00_00; // Red
    storage[1] = 0x00_00_FF_00; // Green
    storage[2] = 0x00_00_00_FF; // Blue
    storage[3] = 0x00_FF_FF_FF; // White

    let fb = Framebuffer::new(&mut storage, 2, 2, 2, PixelFormat::Xrgb8888).unwrap();
    let mut out = [0u8; 128];
    let len = fb.write_ppm_p6(&mut out).expect("Export succeeded");

    let ppm_str = core::str::from_utf8(&out[..11]).unwrap();
    assert_eq!(ppm_str, "P6\n2 2\n255\n");

    // Check pixel payload (RGB order)
    let payload = &out[11..len];
    assert_eq!(payload.len(), 12); // 4 pixels * 3 bytes
    assert_eq!(&payload[0..3], &[255, 0, 0]);   // Red
    assert_eq!(&payload[3..6], &[0, 255, 0]);   // Green
    assert_eq!(&payload[6..9], &[0, 0, 255]);   // Blue
    assert_eq!(&payload[9..12], &[255, 255, 255]); // White
}

struct MockArenaClientSurface {
    pixels: [u32; 100],
    last_published_damage: Option<DamageRect>,
}

impl WindowSurface for MockArenaClientSurface {
    fn pixel_slice_mut(&mut self) -> &mut [u32] {
        &mut self.pixels
    }
    fn width(&self) -> usize { 10 }
    fn height(&self) -> usize { 10 }
    fn stride(&self) -> usize { 10 }
    fn publish_damage(&mut self, damage: DamageRect) {
        self.last_published_damage = Some(damage);
    }
}

#[test]
fn test_arenaos_adapter_zero_copy_flow() {
    let mut surface = MockArenaClientSurface {
        pixels: [0u32; 100],
        last_published_damage: None,
    };

    let mut ctx = ArenaClientContext::new(&mut surface);
    let render_res = ctx.render_with(|fb| {
        fb.set_pixel(3, 4, 0x00_55_66_77);
    });

    assert!(render_res.is_ok());
    assert_eq!(surface.pixels[4 * 10 + 3], 0x00_55_66_77);
    assert_eq!(
        surface.last_published_damage,
        Some(DamageRect::new(3, 4, 1, 1))
    );
}
