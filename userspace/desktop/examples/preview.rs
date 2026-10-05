//! Host preview: render every application view at a given surface size to
//! binary PPM files, so layouts can be inspected at any window size.
//!
//! `cargo run --example preview --target x86_64-unknown-linux-gnu -- W H DIR [--dark]`
use arena_desktop::apps::{
    self, explorer, explorer_ctl, explorer_view,
    model::{Editor, Line, Terminal},
    scene::View,
    view,
};
use arena_gfxkit::Canvas;
use std::io::Write;

fn ppm(path: &std::path::Path, w: usize, h: usize, px: &[u32]) {
    let mut out = Vec::with_capacity(w * h * 3 + 32);
    write!(out, "P6\n{w} {h}\n255\n").unwrap();
    for p in px {
        out.extend_from_slice(&[(p >> 16) as u8, (p >> 8) as u8, *p as u8]);
    }
    std::fs::write(path, out).unwrap();
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let w: u16 = args.get(1).and_then(|v| v.parse().ok()).unwrap_or(448);
    let h: u16 = args.get(2).and_then(|v| v.parse().ok()).unwrap_or(288);
    let dir = std::path::PathBuf::from(args.get(3).map(String::as_str).unwrap_or("."));
    let dark = args.iter().any(|a| a == "--dark");
    std::fs::create_dir_all(&dir).unwrap();
    let mut terminal = Terminal::new();
    terminal.write(b"ArenaOS ordinary command session\nType help. Files are scoped to user-*.");
    for i in 0..40 {
        terminal.write(format!("line {i}: the quick brown fox jumps over the lazy dog").as_bytes());
    }
    for b in b"echo hello" {
        terminal.key(u16::from(*b));
    }
    let mut editor = Editor::new();
    let text = "A resizable editor.\n\tIndented line\n".repeat(30);
    editor.load(text.as_bytes(), "user-note").unwrap();
    let line = Line::new();
    let names: Vec<[u8; 32]> = (0..30)
        .map(|i| {
            let mut n = [0u8; 32];
            let s = format!("user-file-{i:02}");
            n[..s.len()].copy_from_slice(s.as_bytes());
            n
        })
        .collect();
    let counts = [1000, 4000, 20, 18, 7, 1231, 14, 28, 3];
    let processes: Vec<(u64, u64)> = (1..30).map(|p| (p, 1 + p % 3)).collect();
    let preview = b"Preview of the selected file.\nSecond line of text.".as_slice();
    for (kind, name) in apps::TITLES.iter().enumerate() {
        let v = View {
            kind: kind as u8,
            appearance: u8::from(dark) | 2,
            status: "READY",
            terminal: &terminal,
            editor: &editor,
            line: &line,
            top: 0,
            dialog: 0,
            names: &names,
            selected: 3,
            preview,
            display: (800, 600),
            counts: &counts,
            processes: &processes,
            gallery_theme: 2,
            size: (w, h),
            files: None,
        };
        let (uw, uh) = (usize::from(w), usize::from(h));
        let mut px = vec![0u32; uw * uh];
        let mut c = Canvas::new(&mut px, uw, uh, uw).unwrap();
        v.paint(&mut c);
        let file = dir.join(format!(
            "{}-{w}x{h}.ppm",
            name.to_lowercase().replace(' ', "-")
        ));
        ppm(&file, uw, uh, &px);
        println!("{}", file.display());
    }
    // The Phase 11.8 explorer, list and grid, with a selection, a hover and
    // an inline rename; light and dark.
    let mut ctl = Box::new(explorer_ctl::Controller::new());
    ctl.resize((w, h));
    let names: [(&str, bool, u64, u64); 9] = [
        ("Projects", true, 0, 1_791_158_400_000_000),
        ("Archive", true, 0, 1_788_000_000_000_000),
        ("notes.txt", false, 5_120, 1_791_160_000_000_000),
        ("todo.txt", false, 340, 0),
        (
            "Quarterly report draft with a long name.txt",
            false,
            18_400,
            1_790_000_000_000_000,
        ),
        ("photo.png", false, 1_250_000, 1_789_000_000_000_000),
        ("readme", false, 900, 1_791_000_000_000_000),
        ("budget.csv", false, 2_048, 1_791_100_000_000_000),
        ("log.log", false, 77, 1_791_150_000_000_000),
    ];
    for (i, (n, dir, size, mtime)) in names.iter().enumerate() {
        ctl.ex.items[i].set(n.as_bytes(), *dir, *size, *mtime);
    }
    ctl.ex.count = names.len();
    ctl.ex.path = explorer::Path::of(b"Documents/Work");
    ctl.ex.resort();
    ctl.ex.selection.click(2, false, false);
    ctl.ex.selection.click(4, false, true);
    ctl.ui.hover = Some(explorer_view::Hit::Item(5));
    ctl.resize((w, h));
    let (ew, eh) = (usize::from(w), usize::from(h));
    for (view, rename, tag) in [
        (explorer::View::List, false, "list"),
        (explorer::View::List, true, "rename"),
        (explorer::View::Grid, false, "grid"),
    ] {
        ctl.ex.view = view;
        ctl.ex.rename = None;
        if rename {
            ctl.ex.begin_rename(3);
        }
        ctl.resize((ew as u16, eh as u16));
        let status = String::from(ctl.status_line());
        for dark_mode in [false, true] {
            let mut px = vec![0u32; ew * eh];
            let mut c = Canvas::new(&mut px, ew, eh, ew).unwrap();
            let v = View {
                kind: apps::FILES,
                appearance: u8::from(dark_mode) | 2,
                status: &status,
                terminal: &terminal,
                editor: &editor,
                line: &line,
                top: 0,
                dialog: 0,
                names: &[],
                selected: 0,
                preview: &[],
                display: (800, 600),
                counts: &counts,
                processes: &processes,
                gallery_theme: 2,
                size: (ew as u16, eh as u16),
                files: Some(&ctl),
            };
            v.paint(&mut c);
            let file = dir.join(format!(
                "explorer-{tag}{}-{ew}x{eh}.ppm",
                if dark_mode { "-dark" } else { "" }
            ));
            ppm(&file, ew, eh, &px);
            println!("{}", file.display());
        }
    }
    // The context menu as its own transient surface.
    let items = ["New", "Open...", "Save", "Save As..."];
    let (mw, mh) = (
        usize::from(view::MENU_WIDTH),
        usize::from(view::menu_height(items.len())),
    );
    let mut px = vec![0u32; mw * mh];
    let mut c = Canvas::new(&mut px, mw, mh, mw).unwrap();
    view::menu(&mut c, &items, Some(2), arena_ui::theme::palette(dark));
    ppm(&dir.join("menu.ppm"), mw, mh, &px);
}
