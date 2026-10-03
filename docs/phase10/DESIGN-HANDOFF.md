# Sol → Opus design handoff

Status: implemented functional desktop; final qualification pending. Opus starts
only from the recorded qualified source, after full suite/100/archive proof.
Exact source SHA, EFI, archive, suite and screenshot receipts will be inserted
at the Design Handoff Checkpoint. This file describes the implemented contract.

## Architecture

```mermaid
flowchart TD
    Kernel[Kernel: caps / processes / bounded IPC / VM / clock]
    Root[Boot root: explicit delegation and fail-stop supervision]
    Root --> Display[displayd: GOP or real virtio-gpu]
    Root --> Input[inputd: keyboard and tablet device caps]
    Root --> Desktop[Desktop: compositor / window policy / launch and scoped-file broker]
    Input -->|authenticated neutral events| Desktop
    Desktop -->|complete frame Present| Display
    Manager[servicemgr / packaged: accepted signed Image transaction] -->|actual Image bearer| Desktop
    Desktop -->|own fresh backing / endpoint / function scope / private clock| Apps[Six ordinary app processes or signed dynamic app]
    Apps -->|owned Damage and bounded events| Desktop
    Desktop -->|raw fsd writer cap held only here| Fsd[fsd: flat AFS1 / atomic complete PUT]
    Desktop -->|exact original Process cap| Life[Liveness / stop / retire]
    Kernel -->|Pool READ only| Monitor[System Monitor: scalar observations]
```

Names, PIDs, startup view kind, window IDs and visual placement confer no
execution, resource or filesystem authority. APP-CONTRACTS.md describes grants.

## Apps and screenshot inventory

Terminal, Files, Editor, Settings, System Monitor and UI Gallery are distinct
ordinary processes, built from a statically linked multicall ELF. Generic
signed Image applications use the same owned graphics/liveness topology.
Every app, light/dark desktop, six-app desktop and light/dark owned gallery has
an actual guest PNG reference. SCREENSHOT-MANIFEST.md describes the fixture;
checkpoint `screenshot-manifest.json` records source/EFI and each PNG checksum.
Only the owned gallery crop is a deterministic golden. Full desktop captures
include real uptime and diagnostic values; semantic tests use structural pixels.

## Drawing capabilities

Opaque 32-bit XRGB8888 pixels (little-endian B,G,R,unused byte); Canvas has
clipped rectangles, borders, pixel writes, opaque raster blits and bitmap text.
Alpha blending, blur, antialiasing, paths, image decoders and GPU effects are
absent. Do not assume a PNG in this handoff can be decoded by the guest.
Geometry/icon rasters are Arena-authored and assembled from rectangles.

The Arena-owned font is 5x7 ASCII, including distinct lowercase, with integer
scale 1..3, clipping and a 6x8 base cell. Current body scale is 1, title scale 2.
There is no Unicode shaping, proportional font, font loader or Apple asset.
Changing typography beyond the existing primitive needs an engineering request.

## Windows, input and motion

Six sessions each own one window, max 448x288, min 80x60, and a 127-page backing.
Titles are at most 32 bytes. Highest-z hit testing, click focus/to-front, title
bar drag, clipping and a retained visible title support screen edges. Close
queues an owned event; dirty Editor offers save/discard/cancel. Repeated Close
force-stops an unresponsive exact child. Death reaps its original Process and
backing. No minimize, maximize, resize or live compositor restart is offered.

The real QEMU virtio tablet supplies absolute motion and left/right/middle
button state. Clients get local bounded events and press/release capture, never
raw device capabilities. US keyboard supports ASCII/Shift, backspace/Enter/Tab,
Escape, arrows, Home/End/Delete. F1..F6 launch, F7 cycles focus, F8 closes.
There is no clipboard, shortcut modifier framework, key repeat guarantee or
POSIX terminal. Keyboard-only boot also has the production Desktop.

Shared integer Motion supports current/target, retargeting, linear and smooth
interpolation over the monotonic clock. Production uses reveal for open/close,
chrome focus interpolation and bounded 20ms scheduling. Motion can be disabled
and persisted. Gallery exposes 0/50/100% samples and focus/disabled/refused states.
No independent per-app decorative animation loops or compositor alpha exist.

Only authenticated Damage publishes a private complete owned snapshot. Input
redraws reuse that snapshot. GOP Present copies scanout synchronously without
vsync/page-flip; a screenshot can catch a copy in progress. Capture settled
owned regions. Multi-writer/multi-core publication is outside this contract.

## Measured bounds and display matrix

Tested common modes: GOP800x600, real virtio-gpu800x600 and virtio-gpu640x480.
Six real windows are exercised in all three. The source supports bounded mode
validation; other resolutions are not qualified by this matrix.

Six-app GOP: 20 processes/records, seven regions, 1231 shared pages, 14 mappings,
28 steady broker caps and 29 transient caps. GPU800: eight regions/1233 pages/
15 maps; GPU640: eight/1064/15. Backings total 762 pages. Private complete-frame
snapshots reserve 756 more resident broker pages. Exact free-frame minima are
recorded by the final full-suite resource receipts. Four unretired dynamic
children are allowed system-wide, independently of six desktop sessions.
Kernel limits are eight regions, 2048 shared pages, 512 pages/region, 32 mappings,
32 cap slots/process, 24 spawn records, 32 processes and 25 notifications.
Six warmed intermediate page tables remain resident under existing unmap policy;
repeated full cycles return exact warmed accounting.

## Safe presentation sources

- `userspace/ui/src/theme.rs`: light/dark palette and visual tokens.
- `userspace/ui/src/metrics.rs`: typography, spacing, chrome/dock dimensions.
- `userspace/ui/src/motion.rs`: shared interpolation/duration tokens.
- `userspace/ui/src/components/mod.rs`: reusable controls, icons and gallery.
- `userspace/desktop/src/desktop_view.rs`: background, top bar, dock and chrome.
- `userspace/desktop/src/apps/view.rs`: pure application raster views.
- `userspace/desktop/src/apps/layout.rs`: shared layout/hit rectangles, viewport
  dimensions, caret position and visual rows.

These views contain no filesystem, process, IPC or native syscall work. Keep
layout/hit testing consistent and dimensions within the measured backing limit.
Motion algorithm invariants and layout semantics must still pass host tests.

## Engineering review required

Do not silently modify kernel/VM/capability/IPC/spawn/Image bookkeeping;
`userspace/desktop/src/bin/desktop.rs`, `app_client.rs`, `scope.rs`, wire codecs,
fs_backend.rs or app models/controllers; fsd, package/config/permission services;
inputd/displayd drivers; boot delegation/build profiles; signing/package/disk
formats; qualification, lifecycle or authority test oracles. Tests may need new
interaction coordinates after a reviewed layout change, but resource/authority/
actual-input invariants must remain. Add missing primitives to DESIGN-REQUESTS.md.

## Current visual and product limits

The reference skin has neutral light/dark surfaces, simple geometric icons,
small ASCII bitmap text, opaque borders and restrained motion. No decorative
hardware status exists. Uptime is monotonic, not wall time. There are no fake
folders, unsupported toggles, Power in graphical apps, POSIX terminal or larger
signed packages. Flat `user-*` files and 4096-byte text are deliberate limits.
Menus/popups, scrollbars, shadows with alpha, wallpaper/image decoding and richer
font primitives need engineering work before they can be drawn faithfully.

## UI-only checks and deterministic capture

With the pinned Rust/QEMU environment loaded, run from repository root:

```sh
python3 tools/test_m10_ui.py
python3 tools/test_m10_handoff.py
python3 tools/test_m10_desktop.py
python3 tools/test_m10_keyboard.py
python3 tools/test_m10_apps.py
python3 tools/test_m10_files.py
python3 tools/test_m10_display.py
```

Guest suites run sequentially because they share fixed host-peer ports/build
fixtures. `test_m10_handoff.py` builds the explicit production profile, formats
AFS1, boots the historical fixture, seeds known text and canonical light/motion-
disabled preference, then launches each real app and uses QMP `screendump` after
owned-raster settling. Pointer parks at 780,500. It emits lossless RGB PNGs and
`build/phase10-screenshots/manifest.json`. Owned 448x288 gallery crops at 70,60
must match byte-for-byte across an independent boot; local `t` supplies the dark
component palette fixture. The actual dark desktop comes from persisted Settings.
Host gallery PPMs are supplementary fixtures, not guest proof.

After Opus finishes, Sol reviews architecture/resource/performance and performs
all historical tests, a new exact-EFI 100/100 and a new final deployable archive.
The engineering baseline does not replace that post-design qualification.
