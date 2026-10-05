//! Retained-scene damage composition.
//!
//! The compositor describes everything it draws as a [`Scene`]: windows in
//! stacking order (geometry, reveal, focus amount, title and a content
//! generation that advances on every authenticated Damage), the shell facts
//! and the theme. [`damage`] compares the previously presented scene with
//! the next one and lists the screen rectangles whose pixels can differ;
//! [`compose`] redraws a scene under the canvas clip, so composing only the
//! damaged rectangles produces exactly the pixels of a full redraw.
//!
//! Presentation only: the scene carries descriptive values copied from the
//! window policy and session table. Nothing here grants or checks authority.
use crate::shell::{self, Shell};
use arena_gfxkit::{Canvas, Rect};
use arena_ui::{components as c, metrics as m, theme::Theme};

pub const MAX_WINDOWS: usize = crate::model::MAX_WINDOWS;
/// Published regions remembered per window between two presented frames.
pub const REGIONS_MAX: usize = 8;

/// Window-local rectangles `[x, y, width, height]` whose published pixels
/// changed since the last presented frame (Phase 11.1). Bounded: an
/// overflowing list, or a publication of unknown extent, becomes `full`,
/// which damages the whole window exactly as before.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Regions {
    pub r: [[u16; 4]; REGIONS_MAX],
    pub n: u8,
    pub full: bool,
}

impl Regions {
    pub const NONE: Self = Self {
        r: [[0; 4]; REGIONS_MAX],
        n: 0,
        full: false,
    };
    pub fn rects(&self) -> &[[u16; 4]] {
        &self.r[..self.n as usize]
    }
    pub fn mark_full(&mut self) {
        *self = Self::NONE;
        self.full = true;
    }
    /// Add one rectangle, merging it into any rectangle it overlaps or
    /// touches (bounding box); overflow degrades to `full`, never drops.
    pub fn add(&mut self, rect: [u16; 4]) {
        if self.full || rect[2] == 0 || rect[3] == 0 {
            return;
        }
        let mut cur = rect;
        loop {
            let hit = self.rects().iter().position(|o| region_touches(*o, cur));
            match hit {
                Some(i) => {
                    cur = region_union(self.r[i], cur);
                    let last = self.n as usize - 1;
                    self.r[i] = self.r[last];
                    self.r[last] = [0; 4];
                    self.n -= 1;
                }
                None => break,
            }
        }
        if self.n as usize == REGIONS_MAX {
            self.mark_full();
            return;
        }
        self.r[self.n as usize] = cur;
        self.n += 1;
    }
}

fn region_touches(a: [u16; 4], b: [u16; 4]) -> bool {
    let (a0, a1) = (u32::from(a[0]), u32::from(a[0]) + u32::from(a[2]));
    let (b0, b1) = (u32::from(b[0]), u32::from(b[0]) + u32::from(b[2]));
    let (c0, c1) = (u32::from(a[1]), u32::from(a[1]) + u32::from(a[3]));
    let (d0, d1) = (u32::from(b[1]), u32::from(b[1]) + u32::from(b[3]));
    a0 <= b1 && b0 <= a1 && c0 <= d1 && d0 <= c1
}

fn region_union(a: [u16; 4], b: [u16; 4]) -> [u16; 4] {
    let x0 = a[0].min(b[0]);
    let y0 = a[1].min(b[1]);
    let x1 = (a[0] + a[2]).max(b[0] + b[2]);
    let y1 = (a[1] + a[3]).max(b[1] + b[3]);
    [x0, y0, x1 - x0, y1 - y0]
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WindowScene {
    /// Session slot whose published raster backs this window.
    pub slot: usize,
    pub handle: u64,
    pub x: i32,
    pub y: i32,
    pub width: u16,
    pub height: u16,
    /// Rows of content revealed by the open/close motion (0..=height).
    pub reveal: i32,
    /// Shared focus motion amount (0 resting ..= 65536 focused).
    pub focus: i32,
    /// Advances whenever the published raster changes.
    pub content: u64,
    /// Where it changed since the last presented frame (window-local).
    pub regions: Regions,
    pub published: bool,
    pub title: [u8; 32],
    /// Size of the published raster (Phase 11.3). It differs from the
    /// window size while a resized client has not yet published at the new
    /// size: the raster is shown clipped and the rest of the frame filled.
    pub surface: (u16, u16),
    /// The window's transient surface, drawn directly above it.
    pub popup: Option<PopupScene>,
    /// Title-bar controls and their hover state (Phase 11.4).
    pub controls: c::Controls,
}

/// A transient surface (menu, tooltip, dialog) as presented.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PopupScene {
    pub handle: u64,
    pub x: i32,
    pub y: i32,
    pub width: u16,
    pub height: u16,
    pub content: u64,
    pub regions: Regions,
    /// Nothing of a transient surface is visible before its first Damage.
    pub published: bool,
}

impl PopupScene {
    pub fn bounds(&self) -> Rect {
        Rect {
            x: self.x,
            y: self.y,
            width: u32::from(self.width) + c::POPUP_SHADOW as u32,
            height: u32::from(self.height) + c::POPUP_SHADOW as u32,
        }
    }
}

impl WindowScene {
    /// Every pixel this window can draw: frame plus the deepest drop ledge.
    pub fn bounds(&self) -> Rect {
        Rect {
            x: self.x,
            y: self.y,
            width: u32::from(self.width) + m::SHADOW_FOCUSED as u32,
            height: u32::from(self.height) + m::SHADOW_FOCUSED as u32,
        }
    }
    fn title(&self) -> &str {
        let end = self.title.iter().position(|b| *b == 0).unwrap_or(32);
        core::str::from_utf8(&self.title[..end]).unwrap_or("Application")
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Scene {
    /// Windows bottom-to-top; only the first `count` entries are used.
    pub windows: [Option<WindowScene>; MAX_WINDOWS],
    pub count: usize,
    pub shell: Shell,
    pub dark: bool,
}

impl Scene {
    pub const EMPTY: Scene = Scene {
        windows: [None; MAX_WINDOWS],
        count: 0,
        shell: Shell::EMPTY,
        dark: false,
    };
    fn by_slot(&self, slot: usize) -> Option<&WindowScene> {
        self.windows[..self.count]
            .iter()
            .flatten()
            .find(|w| w.slot == slot)
    }
}

/// Up to `CAP` disjoint-ish rectangles; overlapping or touching additions
/// merge, and overflow collapses into one bounding box.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Damage {
    rects: [Rect; Damage::CAP],
    len: usize,
    screen: Rect,
}

fn union(a: Rect, b: Rect) -> Rect {
    let x0 = a.x.min(b.x);
    let y0 = a.y.min(b.y);
    let x1 = (a.x + a.width as i32).max(b.x + b.width as i32);
    let y1 = (a.y + a.height as i32).max(b.y + b.height as i32);
    Rect {
        x: x0,
        y: y0,
        width: (x1 - x0) as u32,
        height: (y1 - y0) as u32,
    }
}

fn touches(a: Rect, b: Rect) -> bool {
    a.x <= b.x + b.width as i32
        && b.x <= a.x + a.width as i32
        && a.y <= b.y + b.height as i32
        && b.y <= a.y + a.height as i32
}

fn intersect(a: Rect, b: Rect) -> Option<Rect> {
    let x0 = a.x.max(b.x);
    let y0 = a.y.max(b.y);
    let x1 = (a.x + a.width as i32).min(b.x + b.width as i32);
    let y1 = (a.y + a.height as i32).min(b.y + b.height as i32);
    (x1 > x0 && y1 > y0).then_some(Rect {
        x: x0,
        y: y0,
        width: (x1 - x0) as u32,
        height: (y1 - y0) as u32,
    })
}

impl Damage {
    pub const CAP: usize = 8;
    pub fn new(width: usize, height: usize) -> Self {
        Self {
            rects: [Rect {
                x: 0,
                y: 0,
                width: 0,
                height: 0,
            }; Damage::CAP],
            len: 0,
            screen: Rect {
                x: 0,
                y: 0,
                width: width as u32,
                height: height as u32,
            },
        }
    }
    pub fn full(&mut self) {
        self.rects[0] = self.screen;
        self.len = 1;
    }
    pub fn add(&mut self, r: Rect) {
        let Some(mut r) = intersect(r, self.screen) else {
            return;
        };
        // Absorb every rectangle the new one touches, repeatedly.
        let mut i = 0;
        while i < self.len {
            if touches(self.rects[i], r) {
                r = union(self.rects[i], r);
                self.len -= 1;
                self.rects[i] = self.rects[self.len];
                i = 0;
            } else {
                i += 1;
            }
        }
        if self.len == Damage::CAP {
            let mut all = r;
            for q in &self.rects[..self.len] {
                all = union(all, *q);
            }
            self.rects[0] = all;
            self.len = 1;
        } else {
            self.rects[self.len] = r;
            self.len += 1;
        }
    }
    pub fn rects(&self) -> &[Rect] {
        &self.rects[..self.len]
    }
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
    pub fn pixels(&self) -> u64 {
        self.rects()
            .iter()
            .map(|r| u64::from(r.width) * u64::from(r.height))
            .sum()
    }
}

/// The window layer alone (its transient surface is a separate layer).
fn frame(w: &WindowScene) -> WindowScene {
    WindowScene { popup: None, ..*w }
}

/// `a` and `b` differ only in published content whose changed regions are
/// known (both published before and after; nothing else moved).
fn content_only(a: &WindowScene, b: &WindowScene) -> bool {
    a.published
        && b.published
        && !b.regions.full
        && b.regions.n > 0
        && WindowScene {
            content: b.content,
            regions: b.regions,
            popup: None,
            ..*a
        } == frame(b)
}

fn popup_content_only(a: &PopupScene, b: &PopupScene) -> bool {
    a.published
        && b.published
        && !b.regions.full
        && b.regions.n > 0
        && PopupScene {
            content: b.content,
            regions: b.regions,
            ..*a
        } == *b
}

fn add_regions(out: &mut Damage, x: i32, y: i32, regions: &Regions) {
    for r in regions.rects() {
        out.add(Rect {
            x: x + i32::from(r[0]),
            y: y + i32::from(r[1]),
            width: u32::from(r[2]),
            height: u32::from(r[3]),
        });
    }
}

/// Rectangles whose pixels may differ between `prev` and `next`.
pub fn damage(prev: &Scene, next: &Scene, out: &mut Damage) {
    let (w, h) = (out.screen.width as i32, out.screen.height as i32);
    if prev.dark != next.dark {
        out.full();
        return;
    }
    for slot in 0..MAX_WINDOWS {
        let (pa, pb) = (prev.by_slot(slot), next.by_slot(slot));
        match (pa, pb) {
            (None, None) => {}
            (Some(a), Some(b)) if frame(a) == frame(b) => {}
            // Only the published raster changed, and exactly where is known:
            // damage those rectangles instead of the whole window.
            (Some(a), Some(b)) if content_only(a, b) => add_regions(out, b.x, b.y, &b.regions),
            (a, b) => {
                if let Some(a) = a {
                    out.add(a.bounds());
                }
                if let Some(b) = b {
                    out.add(b.bounds());
                }
            }
        }
        match (pa.and_then(|w| w.popup), pb.and_then(|w| w.popup)) {
            (None, None) => {}
            (Some(a), Some(b)) if a == b => {}
            (Some(a), Some(b)) if popup_content_only(&a, &b) => {
                add_regions(out, b.x, b.y, &b.regions)
            }
            (a, b) => {
                for p in [a, b].into_iter().flatten() {
                    out.add(p.bounds());
                }
            }
        }
    }
    // Stacking order changes without geometry changes (raise) are covered
    // because each window's z rank is implied by its position: compare it.
    for i in 0..prev.count.max(next.count) {
        let a = prev.windows.get(i).copied().flatten().map(|w| w.slot);
        let b = next.windows.get(i).copied().flatten().map(|w| w.slot);
        if a != b {
            for s in [a, b].into_iter().flatten() {
                for scene in [prev, next] {
                    if let Some(win) = scene.by_slot(s) {
                        out.add(win.bounds());
                        if let Some(p) = win.popup {
                            out.add(p.bounds());
                        }
                    }
                }
            }
        }
    }
    let (p, n) = (&prev.shell, &next.shell);
    if (p.active, p.focused_any, p.open, p.uptime) != (n.active, n.focused_any, n.open, n.uptime) {
        out.add(shell::bar_region(w));
    }
    if p.notice != n.notice {
        out.add(shell::notice_region(w));
    }
    if p.chooser != n.chooser && (p.chooser.is_some() || n.chooser.is_some()) {
        out.add(shell::chooser_region(w, h));
    }
    if p.switcher != n.switcher {
        for s in [p.switcher, n.switcher].into_iter().flatten() {
            out.add(shell::switcher_region(w, h, s.count));
        }
    }
    if p.snap != n.snap {
        for r in [p.snap, n.snap].into_iter().flatten() {
            out.add(shell::snap_region(r));
        }
    }
    let full = |s: &Shell| s.open >= crate::model::MAX_WINDOWS;
    if (
        p.running,
        p.minimized,
        p.active,
        full(p),
        shell::hover_item(w, h, p.pointer),
    ) != (
        n.running,
        n.minimized,
        n.active,
        full(n),
        shell::hover_item(w, h, n.pointer),
    ) {
        out.add(shell::dock_region(w, h));
    }
    if p.pointer != n.pointer {
        out.add(shell::pointer_region(p.pointer.0, p.pointer.1));
        out.add(shell::pointer_region(n.pointer.0, n.pointer.1));
    }
}

/// Draw `scene` within the canvas clip. `contents(slot)` is the published
/// main raster (at least surface width*height pixels) and transient raster
/// (at least popup width*height pixels) of a session slot.
pub fn compose<'c>(
    canvas: &mut Canvas<'_>,
    scene: &Scene,
    contents: impl Fn(usize) -> (&'c [u32], &'c [u32]),
    t: Theme,
) {
    shell::background(canvas, t);
    let clip = canvas.clip();
    for win in scene.windows[..scene.count].iter().flatten() {
        let reveal = win.reveal.clamp(0, i32::from(win.height));
        if reveal > 0 && intersect(win.bounds(), clip).is_some() {
            let width = i32::from(win.width);
            if win.published {
                // The raster as published, clipped to the frame; any part of
                // the frame it does not cover yet is filled.
                let (sw, sh) = (i32::from(win.surface.0), i32::from(win.surface.1));
                let (cw, ch) = (sw.min(width), sh.min(reveal));
                let _ = canvas.blit(
                    contents(win.slot).0,
                    cw as usize,
                    ch as usize,
                    sw as usize,
                    win.x,
                    win.y,
                );
                if cw < width {
                    c::rect(canvas, win.x + cw, win.y, width - cw, reveal, t.elevated);
                }
                if ch < reveal {
                    c::rect(canvas, win.x, win.y + ch, cw, reveal - ch, t.elevated);
                }
            } else {
                c::rect(canvas, win.x, win.y, width, reveal, t.elevated);
            }
            c::window_chrome(
                canvas,
                win.x,
                win.y,
                width,
                i32::from(win.height),
                win.title(),
                win.focus,
                win.controls,
                t,
            );
        }
        if let Some(p) = win.popup.filter(|p| p.published)
            && intersect(p.bounds(), clip).is_some()
        {
            let (w, h) = (usize::from(p.width), usize::from(p.height));
            let _ = canvas.blit(contents(win.slot).1, w, h, w, p.x, p.y);
            c::popup_frame(canvas, p.x, p.y, w as i32, h as i32, t);
        }
    }
    shell::system(canvas, &scene.shell, t);
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use std::vec::Vec;

    const W: usize = 640;
    const H: usize = 480;
    const WW: usize = 96;
    const WH: usize = 64;

    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
        fn below(&mut self, n: u64) -> u64 {
            self.next() % n
        }
    }

    const PW: usize = 40;
    const PH: usize = 24;

    /// Published main and transient rasters of every slot (main rasters
    /// are WW*WH; the surface size decides the stride actually used).
    fn contents(generations: &[u64; MAX_WINDOWS]) -> Vec<(Vec<u32>, Vec<u32>)> {
        (0..MAX_WINDOWS)
            .map(|slot| {
                (
                    raster(slot, generations[slot], WW * WH),
                    raster(slot, 7, PW * PH),
                )
            })
            .collect()
    }

    fn raster(slot: usize, generation: u64, n: usize) -> Vec<u32> {
        (0..n)
            .map(|i| (i as u32).wrapping_mul(2654435761) ^ (generation as u32) ^ slot as u32)
            .collect()
    }

    fn full(scene: &Scene, data: &[(Vec<u32>, Vec<u32>)]) -> Vec<u32> {
        let mut px = std::vec![0u32; W * H];
        let mut canvas = Canvas::new(&mut px, W, H, W).unwrap();
        compose(
            &mut canvas,
            scene,
            |s| (&data[s].0, &data[s].1),
            arena_ui::theme::palette(scene.dark),
        );
        px
    }

    /// Paint random rectangles of a `stride`-wide raster and declare them.
    fn scribble(
        rng: &mut Rng,
        raster: &mut [u32],
        stride: usize,
        size: (usize, usize),
        regions: &mut Regions,
    ) {
        for _ in 0..=rng.below(3) {
            let x = rng.below(size.0 as u64) as u16;
            let y = rng.below(size.1 as u64) as u16;
            let rw = 1 + rng.below(size.0 as u64 - u64::from(x)) as u16;
            let rh = 1 + rng.below(size.1 as u64 - u64::from(y)) as u16;
            let ink = rng.next() as u32;
            for row in y..y + rh {
                for col in x..x + rw {
                    raster[row as usize * stride + col as usize] = ink;
                }
            }
            regions.add([x, y, rw, rh]);
        }
    }

    /// Incremental damage composition must equal a full redraw, pixel for
    /// pixel, across random window, focus, content, pointer and shell
    /// changes, including open/close, raise, theme and notice changes,
    /// frame resizes ahead of and after the client's new surface, and
    /// transient surfaces opening, moving, closing and publishing.
    #[test]
    fn damage_composition_matches_full_redraw() {
        let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
        let mut generations = [0u64; MAX_WINDOWS];
        let mut rasters = contents(&generations);
        let mut scene = Scene::EMPTY;
        scene.shell.pointer = (300, 200);
        let mut screen = full(&scene, &rasters);
        let mut damaged_total = 0u64;
        let mut partial_updates = 0;
        let mut popup_partial = 0;
        let mut resized = 0;
        const STEPS: u64 = 600;
        for step in 0..STEPS {
            let mut next = scene;
            // Regions describe changes since the last presented frame only.
            for w in next.windows.iter_mut().flatten() {
                w.regions = Regions::NONE;
                if let Some(p) = w.popup.as_mut() {
                    p.regions = Regions::NONE;
                }
            }
            let op = rng.below(17);
            match op {
                0 if next.count < MAX_WINDOWS => {
                    // Open a window in a free slot on top.
                    let used: Vec<usize> = next.windows[..next.count]
                        .iter()
                        .flatten()
                        .map(|w| w.slot)
                        .collect();
                    let slot = (0..MAX_WINDOWS).find(|s| !used.contains(s)).unwrap();
                    let mut title = [0u8; 32];
                    title[..4].copy_from_slice(b"Win0");
                    title[3] = b'A' + slot as u8;
                    let (width, height) = (
                        (WW / 2 + rng.below(WW as u64 / 2 + 1) as usize) as u16,
                        (WH / 2 + rng.below(WH as u64 / 2 + 1) as usize) as u16,
                    );
                    next.windows[next.count] = Some(WindowScene {
                        slot,
                        handle: step + 1,
                        x: rng.below(W as u64) as i32 - 40,
                        y: rng.below(H as u64) as i32 - 20,
                        width,
                        height,
                        reveal: rng.below(u64::from(height) + 1) as i32,
                        focus: 0,
                        content: generations[slot],
                        regions: Regions::NONE,
                        published: rng.below(2) == 0,
                        title,
                        surface: (width, height),
                        popup: None,
                        controls: c::Controls {
                            minimize: true,
                            maximize: rng.below(2) == 0,
                            maximized: false,
                            hover: 0,
                        },
                    });
                    next.count += 1;
                    next.shell.open = next.count;
                }
                1 if next.count > 0 => {
                    // Close a window (remove from the stack).
                    let i = rng.below(next.count as u64) as usize;
                    for j in i..next.count - 1 {
                        next.windows[j] = next.windows[j + 1];
                    }
                    next.count -= 1;
                    next.windows[next.count] = None;
                    next.shell.open = next.count;
                }
                2 if next.count > 0 => {
                    let i = rng.below(next.count as u64) as usize;
                    let w = next.windows[i].as_mut().unwrap();
                    let (dx, dy) = (rng.below(61) as i32 - 30, rng.below(41) as i32 - 20);
                    w.x += dx;
                    w.y += dy;
                    if let Some(p) = w.popup.as_mut() {
                        p.x += dx;
                        p.y += dy;
                    }
                }
                3 if next.count > 1 => {
                    // Raise: move one window to the top.
                    let i = rng.below(next.count as u64) as usize;
                    let w = next.windows[i];
                    for j in i..next.count - 1 {
                        next.windows[j] = next.windows[j + 1];
                    }
                    next.windows[next.count - 1] = w;
                }
                4 if next.count > 0 => {
                    // Whole-raster publication (unknown extent).
                    let i = rng.below(next.count as u64) as usize;
                    let w = next.windows[i].as_mut().unwrap();
                    generations[w.slot] += 1;
                    rasters[w.slot].0 = raster(w.slot, generations[w.slot], WW * WH);
                    w.content = generations[w.slot];
                    w.published = true;
                }
                10 | 11 if next.count > 0 => {
                    // Partial publication: only declared rectangles change.
                    let i = rng.below(next.count as u64) as usize;
                    let w = next.windows[i].as_mut().unwrap();
                    generations[w.slot] += 1;
                    w.content = generations[w.slot];
                    if !w.published {
                        w.published = true;
                        w.regions.mark_full();
                    }
                    let size = (usize::from(w.surface.0), usize::from(w.surface.1));
                    let mut regions = w.regions;
                    scribble(&mut rng, &mut rasters[w.slot].0, size.0, size, &mut regions);
                    w.regions = regions;
                    partial_updates += 1;
                }
                12 if next.count > 0 => {
                    // Policy resize: the frame changes before the client has
                    // published at the new size.
                    let i = rng.below(next.count as u64) as usize;
                    let w = next.windows[i].as_mut().unwrap();
                    w.width = (WW / 2 + rng.below(WW as u64 / 2 + 1) as usize) as u16;
                    w.height = (WH / 2 + rng.below(WH as u64 / 2 + 1) as usize) as u16;
                    w.reveal = w.reveal.min(i32::from(w.height));
                    resized += 1;
                }
                13 if next.count > 0 => {
                    // The client commits a whole surface at its frame size.
                    let i = rng.below(next.count as u64) as usize;
                    let w = next.windows[i].as_mut().unwrap();
                    generations[w.slot] += 1;
                    rasters[w.slot].0 = raster(w.slot, generations[w.slot], WW * WH);
                    w.surface = (w.width, w.height);
                    w.content = generations[w.slot];
                    w.published = true;
                    w.regions.mark_full();
                }
                14 if next.count > 0 => {
                    // Open, replace, move or close a transient surface.
                    let i = rng.below(next.count as u64) as usize;
                    let w = next.windows[i].as_mut().unwrap();
                    w.popup = match rng.below(3) {
                        0 => None,
                        _ => Some(PopupScene {
                            handle: step + 1000,
                            x: w.x + rng.below(WW as u64) as i32 - 10,
                            y: w.y + rng.below(WH as u64) as i32 - 10,
                            width: (PW / 2 + rng.below(PW as u64 / 2 + 1) as usize) as u16,
                            height: (PH / 2 + rng.below(PH as u64 / 2 + 1) as usize) as u16,
                            content: 0,
                            regions: Regions::NONE,
                            published: rng.below(3) != 0,
                        }),
                    };
                }
                15 | 16 if next.count > 0 => {
                    // Partial or whole publication of a transient surface.
                    let i = rng.below(next.count as u64) as usize;
                    let w = next.windows[i].as_mut().unwrap();
                    let slot = w.slot;
                    if let Some(p) = w.popup.as_mut() {
                        p.content += 1;
                        if !p.published || rng.below(4) == 0 {
                            p.published = true;
                            rasters[slot].1 = raster(slot, p.content, PW * PH);
                            p.regions.mark_full();
                        } else {
                            let size = (usize::from(p.width), usize::from(p.height));
                            let mut regions = p.regions;
                            scribble(&mut rng, &mut rasters[slot].1, size.0, size, &mut regions);
                            p.regions = regions;
                            popup_partial += 1;
                        }
                    }
                }
                5 if next.count > 0 => {
                    let i = rng.below(next.count as u64) as usize;
                    let w = next.windows[i].as_mut().unwrap();
                    w.focus = rng.below(65_537) as i32;
                    w.reveal = rng.below(u64::from(w.height) + 1) as i32;
                    w.controls.hover = rng.below(4) as u8;
                    w.controls.maximized = rng.below(2) == 0;
                }
                6 => {
                    // Pointer, sometimes across the dock strip.
                    next.shell.pointer = (rng.below(W as u64) as i32, rng.below(H as u64) as i32);
                }
                7 => {
                    next.shell.uptime += 1;
                    next.shell.running[rng.below(6) as usize] = rng.below(3) as u8;
                    next.shell.active = [None, Some(rng.below(6) as u8)][rng.below(2) as usize];
                    next.shell.focused_any = rng.below(2) == 0;
                    next.shell.minimized[rng.below(6) as usize] = rng.below(2) as u8;
                }
                8 => {
                    next.shell.notice =
                        [None, Some("LAUNCH REFUSED / DESKTOP CAPACITY")][rng.below(2) as usize];
                    next.shell.switcher = match rng.below(3) {
                        0 => None,
                        _ => {
                            let mut sw = shell::Switcher {
                                count: 1 + rng.below(MAX_WINDOWS as u64) as u8,
                                selected: 0,
                                titles: [[0; 32]; MAX_WINDOWS],
                                kinds: [6; MAX_WINDOWS],
                            };
                            sw.selected = rng.below(u64::from(sw.count)) as u8;
                            for (i, t) in sw.titles.iter_mut().enumerate() {
                                t[..3].copy_from_slice(b"App");
                                t[3] = b'A' + i as u8;
                                sw.kinds[i] = (i % 7) as u8;
                            }
                            Some(sw)
                        }
                    };
                    next.shell.chooser = match rng.below(3) {
                        0 => None,
                        _ => {
                            let mut ch = shell::ChooserView {
                                save: rng.below(2) == 0,
                                read_only: false,
                                place: [0; 48],
                                rows: [[0; 32]; shell::CHOOSER_ROWS],
                                count: rng.below(shell::CHOOSER_ROWS as u64 + 1) as u8,
                                selected: None,
                                above: rng.below(2) == 0,
                                below: rng.below(2) == 0,
                                name: [0; 32],
                                message: [0; 48],
                            };
                            ch.place[..14].copy_from_slice(b"Home/Documents");
                            for (i, r) in ch.rows.iter_mut().enumerate() {
                                r[..4].copy_from_slice(b"file");
                                r[4] = b'a' + i as u8;
                            }
                            if ch.count > 0 && rng.below(2) == 0 {
                                ch.selected = Some(rng.below(u64::from(ch.count)) as u8);
                            }
                            let n = rng.below(20) as usize;
                            ch.name[..n].fill(b'n');
                            if rng.below(2) == 0 {
                                ch.message[..9].copy_from_slice(b"TYPE NAME");
                            }
                            Some(ch)
                        }
                    };
                    next.shell.snap = match rng.below(3) {
                        0 => Some(Rect {
                            x: 0,
                            y: 26,
                            width: W as u32 / 2,
                            height: H as u32 - 82,
                        }),
                        1 => Some(Rect {
                            x: W as i32 / 2,
                            y: 26,
                            width: W as u32 / 2,
                            height: H as u32 - 82,
                        }),
                        _ => None,
                    };
                }
                9 if rng.below(8) == 0 => next.dark = !next.dark,
                _ => {}
            }
            let mut d = Damage::new(W, H);
            damage(&scene, &next, &mut d);
            damaged_total += d.pixels();
            let data = &rasters;
            let mut canvas = Canvas::new(&mut screen, W, H, W).unwrap();
            for r in d.rects() {
                canvas.set_clip(*r);
                compose(
                    &mut canvas,
                    &next,
                    |s| (&data[s].0, &data[s].1),
                    arena_ui::theme::palette(next.dark),
                );
            }
            drop(canvas);
            let expected = full(&next, &rasters);
            if let Some(i) = (0..W * H).find(|&i| screen[i] != expected[i]) {
                panic!(
                    "step {step} op {op}: stale pixel at ({},{}) damage {:?}\nprev {:?}\nnext {:?}",
                    i % W,
                    i / W,
                    d.rects(),
                    scene.shell,
                    next.shell
                );
            }
            scene = next;
        }
        // Damage must be a real saving, not a disguised full redraw.
        assert!(
            damaged_total < STEPS * (W * H) as u64 / 3,
            "{damaged_total}"
        );
        assert!(
            partial_updates > 10 && popup_partial > 5 && resized > 10,
            "too few partial publications or resizes exercised: {partial_updates} {popup_partial} {resized}"
        );
    }

    #[test]
    fn pointer_motion_damages_only_two_small_rects() {
        let mut a = Scene::EMPTY;
        a.shell.pointer = (100, 100);
        let mut b = a;
        b.shell.pointer = (300, 250);
        let mut d = Damage::new(800, 600);
        damage(&a, &b, &mut d);
        assert_eq!(d.rects().len(), 2);
        assert_eq!(
            d.pixels(),
            2 * (c::POINTER_WIDTH * c::POINTER_HEIGHT) as u64
        );
        let mut same = Damage::new(800, 600);
        damage(&b, &b, &mut same);
        assert!(same.is_empty());
    }

    #[test]
    fn damage_list_merges_and_bounds() {
        let mut d = Damage::new(100, 100);
        d.add(Rect {
            x: 0,
            y: 0,
            width: 10,
            height: 10,
        });
        d.add(Rect {
            x: 10,
            y: 0,
            width: 10,
            height: 10,
        });
        assert_eq!(
            d.rects(),
            &[Rect {
                x: 0,
                y: 0,
                width: 20,
                height: 10
            }]
        );
        d.add(Rect {
            x: -50,
            y: 90,
            width: 500,
            height: 50,
        });
        assert_eq!(
            d.rects()[1],
            Rect {
                x: 0,
                y: 90,
                width: 100,
                height: 10
            }
        );
        for i in 0..20 {
            d.add(Rect {
                x: i * 4 + 1,
                y: 30 + (i % 2) * 20,
                width: 1,
                height: 1,
            });
        }
        assert!(d.rects().len() <= Damage::CAP);
    }
}
