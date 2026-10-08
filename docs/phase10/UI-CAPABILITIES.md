# UI capabilities

Status: Sol engineering inventory; 97/97 suite and exact-image 100/100 gates
passed. GOP800, GPU800 and GPU640 six-app matrix passed.

| Area | Implemented capability |
|---|---|
| Pixels | Opaque 32-bit XRGB8888, clipped rectangles, borders and blits, a settable canvas clip (ADR-0070); no alpha blending |
| Text | Arena-owned 5x7 ASCII bitmap glyphs, distinct lowercase, integer scale 1..3; Opus styles: Body (scale 1), Strong (double-struck, advance 7), Caption (upper-case, advance 7), Display (scale 2) |
| Icons | Small geometric pixel icons assembled from rectangles; raw owned rasters may be blitted |
| Windows | Six owned surfaces, pointer activation/click-to-front, title drag with retained visible title, close, exact clipping, keyboard focus |
| Pointer | Actual QEMU virtio tablet absolute motion and left/right/middle button state; clients receive bounded local events, press/release capture |
| Keyboard | US ASCII with Shift, backspace, Enter, tab, Escape, arrows, Home/End/Delete; F1..F6 launch, F7 cycle focus, F8 close |
| Motion | Monotonic integer linear/smooth interpolation, current/target retargeting, open/close reveal, focus chrome color transition, 20ms frame pacing |
| Appearance | Shared light/dark tokens, durable theme and motion preference, live client repaint |
| Components | Shared window chrome, text styles, buttons (standard/primary/destructive x normal/focused/selected/disabled/refused), field/I-beam caret, rows/selection, sidebar, switch, progress/meter, chips, status band, original app/interface icon masks, emblem, pointer and terminal raster |

Presentation files: `userspace/ui/src/theme.rs`, `metrics.rs`, `motion.rs`,
`components/mod.rs`; `userspace/desktop/src/shell.rs`; application
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
127-page app backings and holds each exact original Process cap. Six private published snapshots additionally reserve 756 broker pages. Standard
800x600 GOP requires 469 scanout pages. Six apps use 1231 shared pages, 14 maps
and 28 steady broker caps; transient broker cap high-water is 29. Guest display tests pass GOP800x600 and
actual virtio-gpu800x600/640x480 with all six apps. Exact-image engineering stability passed 100/100. The dynamic child limit is four system-wide, independently of six
desktop sessions. Built-ins are actual static userspace BootImage processes.

`python3 tools/test_m10_ui.py` checks host components/motion/policy, formatting,
Clippy, no_std library compilation and deterministic host gallery rasters.
`test_m10_keyboard.py`, `test_m10_desktop.py`, `test_m10_apps.py`,
`test_m10_files.py`, `test_m10_dynamic.py` and `test_m10_boundaries.py` exercise
actual guest processes, native audits, QMP pointer/keyboard and owned pixels.
Host gallery rasters are reference fixtures; guest screenshots are the handoff
proof. Final capture commands and hashes belong in SCREENSHOT-MANIFEST.md.

Only authenticated Damage publishes a complete owned snapshot. Pointer/focus
redraws use private published pixels; unsubmitted client staging writes remain
invisible. GOP hardware publication uses a synchronous framebuffer copy, with
no vsync/page-flip guarantee. Deterministic captures wait for settled owned
rasters so QMP does not mistake a scanout copy in progress for submitted content.
Viewport rows/columns and pointer-to-caret mapping live in `apps/layout.rs`,
shared with app scrolling; typography and spacing changes use those tokens.
