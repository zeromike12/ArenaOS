//! Deterministic host raster fixture; real guest QMP capture is a separate gate.
use std::io::{self, Write};
fn main() {
    let dark = std::env::args().any(|s| s == "--dark");
    let mut pixels = vec![0u32; 448 * 288];
    let mut canvas = arena_gfxkit::Canvas::new(&mut pixels, 448, 288, 448).unwrap();
    arena_ui::components::gallery(&mut canvas, arena_ui::theme::palette(dark));
    let stdout = io::stdout();
    let mut out = io::BufWriter::new(stdout.lock());
    write!(out, "P6\n448 288\n255\n").unwrap();
    for p in pixels {
        out.write_all(&[(p >> 16) as u8, (p >> 8) as u8, p as u8])
            .unwrap();
    }
}
