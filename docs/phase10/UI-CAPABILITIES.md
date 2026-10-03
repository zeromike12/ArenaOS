# UI capabilities

Status: implemented inventory; qualification and measured display matrix pending.

| Area | Implemented capability |
|---|---|
| Pixels | Opaque 32-bit XRGB8888, clipped rectangles, borders and blits; no alpha blending |
| Text | Arena-owned 5x7 ASCII bitmap glyphs, distinct lowercase, integer scale 1..3; body scale 1 and title scale 2 |
| Icons | Small geometric pixel icons assembled from rectangles; raw owned rasters may be blitted |
| Windows | Six owned surfaces, pointer activation/click-to-front, title drag with retained visible title, close, exact clipping, keyboard focus |
| Pointer | Actual QEMU virtio tablet absolute motion and left/right/middle button state; clients receive bounded local events, press/release capture |
| Keyboard | US ASCII with Shift, backspace, Enter, tab, Escape, arrows, Home/End/Delete; F1..F6 launch, F7 cycle focus, F8 close |
| Motion | Monotonic integer linear/smooth interpolation, current/target retargeting, open/close reveal, focus chrome color transition, 20ms frame pacing |
| Appearance | Shared light/dark tokens, durable theme and motion preference, live client repaint |
| Components | Chrome, labels/headings, buttons, field/cursor, rows/selection, sidebar, toolbar, geometric icons, progress, status/refusal and terminal raster |

Presentation files: `userspace/ui/src/theme.rs`, `metrics.rs`, `motion.rs`,
`components/mod.rs`; `userspace/desktop/src/desktop_view.rs`; application
`apps/view.rs` and `apps/layout.rs`. They contain drawing and layout, with no
filesystem, lifecycle, native syscall or IPC operations. Hit testing uses the
same app layout rectangles as drawing. Wire protocols contain no palette or
component-style tokens.

No live compositor restart, alpha compositing, blur, vector paths, antialiasing,
image decoder, Unicode shaping, dynamically loaded font, minimize/maximize,
resizing, menus, scrollbar, clipboard, networking UI or unsupported hardware
status is claimed. Monitor samples actual counters and process/thread counts.
The top bar shows active application metadata and monotonic uptime, not a fake
wall clock or battery/network state.

The kernel retains eight SharedRegions, 2048 aggregate pages, 512 pages per
region, 32 mappings and 32 cap slots per process. The desktop reserves six
127-page app backings and holds each exact original Process cap. Standard
800x600 GOP requires 469 scanout pages. Six apps use 1231 shared pages, 14 maps
and 28 steady broker caps; additional display paths and transient peaks remain
to qualify. The dynamic child limit is four system-wide, independently of six
desktop sessions. Built-ins are actual static userspace BootImage processes.

`python3 tools/test_m10_ui.py` checks host components/motion/policy, formatting,
Clippy, no_std library compilation and deterministic host gallery rasters.
`test_m10_keyboard.py`, `test_m10_desktop.py`, `test_m10_apps.py`,
`test_m10_files.py`, `test_m10_dynamic.py` and `test_m10_boundaries.py` exercise
actual guest processes, native audits, QMP pointer/keyboard and owned pixels.
Host gallery rasters are reference fixtures; guest screenshots are the handoff
proof. Final capture commands and hashes belong in SCREENSHOT-MANIFEST.md.
