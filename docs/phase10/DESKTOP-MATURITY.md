# Desktop maturity and performance pass

Branch `arena/phase10-desktop-maturity`, from the frozen visual reference
`arena/phase10-opus-design-reference` (`de7551c3cda8c387991656b11d84a69e79d2ce4d`).
This pass changes engineering, not the visual skin: no colours, metrics or
component looks were polished. The concrete Phase-11 plan derived from this
audit is [docs/phase11/PLAN.md](../phase11/PLAN.md).

## 1. How latency was measured

* **Guest probes** (`userspace/desktop/src/perf.rs`): compiled in only when
  the image is built with `ARENA_PERF=1`. The compositor reports per second
  `render`, `compose`, `present`, Damage `copy`, `input2frame` (input request
  received → frame presented), `request`, `sleep` and presented kilopixels;
  each built-in client reports `paint`, `damage` (IPC), `event2damage` (first
  event seen → damage published) and `poll` (IPC). Default images contain
  none of this.
* **Host probes** (`tools/profile_desktop.py`): boots the real desktop and
  drives QMP tablet/keyboard through fixed scenarios (idle, empty-desktop
  pointer sweep, launch, Terminal typing, six-app idle, six-app pointer
  sweep, title drag). Motion-to-photon: move the pointer, dump the scanout
  back-to-back until the arrow is drawn there. Key-to-photon: type into the
  focused Terminal, dump until its input line changes. Both include one
  screendump (~3.3 ms).
* All numbers are QEMU TCG (no KVM here): absolute values are inflated;
  ratios and same-host comparisons are meaningful.

## 2. Findings (baseline = frozen visual reference)

| Cost | Baseline | Cause |
|---|---|---|
| Host pointer motion-to-photon | 33.7 ms median, 44.9 max | full-frame render + full present + idle halt + 25 ms compositor poll |
| Host key-to-photon (Terminal) | 106.9 ms median, 137.3 max | as above, plus 20 ms client poll period and per-poll stalls |
| Render per frame | 13–40 ms, always 480k px | full background + every window + shell, even for a cursor move |
| PRESENT per frame | 9–27 ms | full frame, per-pixel volatile copy, **and a fixed ~9 ms per call** |
| Client poll IPC | ~23–25 ms | caller and server separated by tick waits |
| Terminal event → damage | 226 ms mean | client poll period + contended compositor |
| Pointer events rendered | ~13 of 150 sent | compositor saturated; virtio queue overflow |

The fixed ~9 ms per PRESENT, independent of area, exposed the root cause in
the kernel: the idle loop executed `sti; hlt` after every `yield_now` even
with threads in the ready ring, so an IPC server woken by a blocking caller
often waited for the next 10 ms tick (ADR-0069).

## 3. Fixes

| Change | Where | ADR |
|---|---|---|
| Idle halts only when the ready ring is empty | `kernel/kernel/src/entry.rs`, `sched::ready_pending` | 0069 |
| Canvas clip; row-slice fills and blits | `userspace/gfxkit` | 0070 |
| Retained scene, damage diff, clipped composition, partial PRESENT | `userspace/desktop/src/compose.rs`, `shell.rs`, `bin/desktop.rs` | 0070 |
| Identity-format row copy in GOP PRESENT | `userspace/displayd/src/main.rs` | 0070 |
| Broker signals a built-in client's own clock when events are queued; `idle()` cancels its timer | `bin/desktop.rs`, `app_client.rs`, `model::State::pending` | 0070 |
| Compositor polls its endpoint each scheduler tick | `bin/desktop.rs` (`POLL_TICK_US`) | 0070 |
| Opt-in probes and host profiler | `perf.rs`, `tools/profile_desktop.py` | — |

## 4. Results (same harness, same host)

| Measure | Reference | Now | |
|---|---|---|---|
| Pointer motion-to-photon, median / max | 33.7 / 44.9 ms | **8.6 / 11.7 ms** | 3.9x |
| Key-to-photon in Terminal, median / max | 106.9 / 137.3 ms | **28.6 / 38.2 ms** | 3.7x |

Guest probes (mean per event; pre-fix probes on the reference code, then
after each fix):

| Scenario / metric | Pre-fix | + damage | + idle fix | + client wake |
|---|---|---|---|---|
| Cursor move: render | 13.3 ms | 18.8 ms | 1.0 ms | 2.0 ms |
| Cursor move: PRESENT | 9.5 ms | 18.4 ms | 0.6 ms | 1.6 ms |
| Cursor move: input → frame | 11.6 ms | 20.2 ms | 1.2 ms | 2.3 ms |
| Pointer sweep: events rendered / 150 | 13 | 14 | 50 | 49 |
| Pixels presented per cursor frame | 480k | ~1–3k | ~1–3k | ~1–3k |
| Terminal typing: event → damage | 226 ms | 215 ms | 215 ms | **12 ms** |
| Terminal typing: client poll IPC | 25.6 ms | 23.5 ms | 23.4 ms | 5.1 ms |
| Terminal typing: client damages for 31 keys | 4 | 4 | 4 | 31 |
| Six apps idle: render (Monitor sample) | 40.2 ms | 11.5 ms | 7.7 ms | 7.0 ms |
| Title drag, six apps: render | 22.9 ms | 13.4 ms | 5.3 ms | 5.4 ms |

Reading the table honestly: damage composition alone cut pixels ~200x but
not time, because the per-call ~9 ms tick wait dominated (a cursor move
made two PRESENT calls, each paying it, so that column briefly got worse); the idle fix
unlocked it; the client wake removed the client-side poll period. The small
rise in cursor cost in the last column comes from the extra request traffic
of clients that now wake promptly.

**Remaining latency** (key-to-photon ~25 ms after subtracting the
screendump): the compositor still cannot be woken by an IPC arrival, so
inputd → compositor, client Poll and client Damage each wait for the next
tick (~5 ms average each); a Terminal damage still copies the full 448x288
raster (~0.8 ms) and recomposes the full window rectangle. Idle cost moved
the other way: the compositor now wakes on every 10 ms tick (no-damage wakes
present nothing). Both are addressed by Phase-11 workstreams K and C.

## 5. Invariants kept

* No change to capability rights, grants, scopes, spawn topology, wire
  formats, publication (ADR-0067), lifecycle or resource bounds.
* The scene is descriptive data; damage chooses which pixels to recompute,
  never what a client may do.
* Host proofs: gfxkit clip equivalence; randomised damage-vs-full-redraw
  equivalence (shown RED for a missing old-pointer region and for a missing
  drop-ledge margin); layout and palette tests unchanged.
* Guest oracle kept at full strength. `test_m10_dynamic` proves that a
  client's unpublished staging bytes never reach the screen by moving the
  pointer after the client stages a drawing. Under full-frame rendering
  any pointer move recomposed every window; under damage composition a
  move far from the window recomposes nothing of it, so the old check
  would pass vacuously. The oracle now first sweeps the arrow across the
  signed raster (forcing those window pixels to be recomposed) and then
  makes the unchanged exact comparison. The `frame-publication` control in
  `test_m10_boundaries_red` now mutates the single place composition reads
  window content (the `compose` closure in `render`) to serve the staging
  bytes, and the real guest oracle rejects it.
* Guest: the Phase-10 suites and the complete historical suite are rerun on
  this source (receipts in §8).

## 6. Audit: what prevents a real file explorer and desktop filesystem

### Storage format (AFS1, ADR-0023/0063)

| Limit | Value | Consequence |
|---|---|---|
| Namespace | flat, `/` refused | no folders, no hierarchy, no per-folder scope |
| Objects | 32 in total, shared with system records (boot `arena.txt`, configd, permission, package, `ui10-prefs`) | a user realistically has ~20–25 files |
| Names | ≤ 31 bytes, ASCII alnum/`-_.` for desktop scope | no spaces, no long names |
| File size | ≤ 62 extents; desktop PUT ≤ 8 sectors (4096 B) | the desktop cannot hold a real document, image or package |
| Disk | 16384 sectors (8 MiB); RAM bitmap sized to that | no growth |
| Time | `mtime` reserved, no wall clock | no "modified" column, no sorting by date, no recents |
| Operations | create, open, read, in-place write, close, list (cursor), unlink, complete PUT | no rename/move, copy, truncate, partial transactional write, mkdir, stat-by-handle, change notification |
| Crash model | CoW metadata, ping-pong commit, 2-generation delayed free | sound; must be preserved by AFS2 |

### File authority (ADR-0065, `scope.rs`)

* Desktop file access is one coarse grant over the `user-*` name prefix,
  enforced by a name filter in the broker. That was the right minimal step;
  it cannot express "this folder", "this document" or "read-only here".
* The architecture already prescribes the target (ARCHITECTURE §10): file
  handles are capabilities to file objects with rights; a namespace service
  resolves paths once into capabilities; paths are UI. Nothing implements it
  yet.
* There is no trusted open/save dialog (powerbox), so an application cannot
  be handed one user-chosen document without holding the whole scope.

### File service placement and bandwidth

* All desktop file operations execute **synchronously inside the
  compositor process** (`service()` in `bin/desktop.rs`). A directory
  listing (one IPC per entry, each a raw fsd call) or a save stalls input
  and rendering for its whole duration.
* Each session has a single 4 KiB I/O page in its backing; transfers are
  bounded to it. fsd moves at most `FS_XFER_MAX = 3584` bytes per call.

### Window system

| Limit | Value | Blocks |
|---|---|---|
| SharedRegions | 8 system-wide (6 apps + scanout + GPU) | more windows, double buffering |
| Shared pages | 2048 total, 512 per region | windows larger than 448x288; maximize to 800x600 needs ~470 pages each |
| Window size | fixed at creation, ≤ 448x288 (127-page backing) | resize, maximize, snapping |
| Sessions | 6 desktop sessions, 4 dynamic children | real multitasking |
| Publication | full-raster copy per Damage (ADR-0067), 3 MiB static snapshots | partial damage, larger windows |
| Window ops | focus, raise, drag, close | resize, minimize, maximize, snap, switcher, menus |
| Popups | none (no transient/child surfaces) | menus, context menus, tooltips, dialogs |

### Input

No modifier state model (Ctrl/Alt/Super chords), no key repeat guarantee, no
scroll wheel (inputd ignores tablet wheel events), no double-click or drag
threshold policy, no clipboard, no drag-and-drop payloads.

### Rendering primitives

Now present: opaque fill, clip, row blit, 5x7 text at integer scale. Missing:
translate/origin (scroll views), alpha or darken (shadows, selection
overlays), masked/alpha image blit (icons at 32/48 px, thumbnails), a second
designed type face, glyph-run fast path, scroll container and scrollbar,
popup surfaces.

### Kernel event plumbing

No endpoint-bound notification (a server cannot wait on "IPC arrived OR
timer"), no direct IPC handoff (wake does not yield to the woken server),
one global 32-entry timer table with no per-process quota (any holder of a
notification can exhaust it), 100 Hz tick, debug-write available to every
process.

## 7. Files touched by this pass

Kernel: `kernel/kernel/src/entry.rs`, `kernel/kernel/src/sched/mod.rs`.
Userspace: `userspace/gfxkit/src/lib.rs`, `userspace/displayd/src/main.rs`,
`userspace/desktop/src/{compose,perf,shell,lib,model,app_client,client}.rs`,
`userspace/desktop/src/bin/{desktop,application}.rs`,
`userspace/ui/src/components/{icons,mod}.rs` (pointer size constants only).
Tools: `tools/profile_desktop.py` (new). Docs: ADR-0069, ADR-0070, this
file, `docs/phase11/PLAN.md`.
