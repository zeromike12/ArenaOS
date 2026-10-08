//! Writes a specimen sheet of Arena Sans 13 as a PPM (host review aid).
use arena_gfxkit::{Canvas, Rect};
fn main() {
    let (w, h) = (360usize, 150usize);
    let mut px = vec![0x00f4_f2eeu32; w * h];
    let mut c = Canvas::new(&mut px, w, h, w).unwrap();
    let lines = [
        ("ABCDEFGHIJKLMNOPQRSTUVWXYZ", false),
        ("abcdefghijklmnopqrstuvwxyz 0123456789", false),
        ("!\"#$%&'()*+,-./:;<=>?@[\\]^_`{|}~", false),
        ("\u{201c}Quoted\u{201d} \u{2018}it\u{2019}s\u{2019} \u{2013} \u{2014} \u{2026} \u{2022} 3\u{00d7}4 20\u{00b0}", false),
        ("Documents   Desktop   Trash   12 items", true),
        ("The quick brown fox jumps over the lazy dog.", false),
        ("Rename \u{201c}Report final v2.txt\u{201d}\u{2026}", false),
    ];
    for (i, (s, bold)) in lines.iter().enumerate() {
        c.text13(6, 4 + i as i32 * 20, s, 0x0020_2428, *bold).unwrap();
    }
    c.fill_rect(Rect { x: 0, y: 145, width: w as u32, height: 1 }, 0x00c0_c0c0);
    let scale = 3;
    let mut out = format!("P6\n{} {}\n255\n", w * scale, h * scale).into_bytes();
    for y in 0..h * scale {
        for x in 0..w * scale {
            let p = px[(y / scale) * w + x / scale];
            out.extend_from_slice(&[(p >> 16) as u8, (p >> 8) as u8, p as u8]);
        }
    }
    std::fs::write(std::env::args().nth(1).unwrap(), out).unwrap();
}
