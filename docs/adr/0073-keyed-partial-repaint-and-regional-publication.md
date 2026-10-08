# ADR-0073 — Keyed partial repaint and regional Damage publication

Status: accepted; implemented on `arena/phase11-desktop-maturity` (Phase 11.1).

## Problem

After ADR-0070 the compositor recomposed only damaged screen rectangles,
but every client repaint was still whole-surface: a Terminal keystroke
repainted the entire 448×288 raster, published it with a whole-surface
Damage (a 129k-pixel copy into the private snapshot), and the compositor
then recomposed and presented the whole window. A key release, which
changes nothing, did all of that a second time.

## Decision

### Client: keyed bands

`arena_desktop::apps::scene::View` is everything an application window
paints; `View::paint` draws it completely and, under a gfxkit clip, exactly
the clipped part of the same picture. `View::bands` partitions the window
into full-width horizontal bands, each with a key: a 64-bit FNV-1a hash of
exactly the state whose pixels can land in that band.

* Terminal: header (line count, scrollback offset), one band per transcript
  row (the text shown in that row), the input line (text and caret), the
  status band (message).
* Editor: header (toolbar or dialog, document name and state, caret
  line:column), one band per visual text row (the glyphs and their columns,
  caret-row highlight, caret column), the status band.
* Files, Settings, Monitor, Gallery: header and one content band keyed on
  all their content state (conservative), plus the status band.

`scene::dirty(previous, next)` returns the bands whose keys changed as at
most five rectangles: adjacent bands merge; with more runs, the two runs
separated by the smallest clean gap are joined (repainting the gap is always
correct); a layout change repaints everything. The client repaints only
those rectangles (clip + paint) over the frame its backing already holds and
publishes exactly them. Unchanged frames publish nothing.

### Wire: Damage regions

`Damage { handle, rects }` keeps opcode 2. `rects.n == 0` is the whole
surface and is byte-identical to the Phase-10 frame, so existing clients and
the 4 KiB signed fixture are unchanged. Up to five `[x, y, width, height]`
rectangles follow; the encoding is canonical (unused slots zero, nonempty,
inside 448×288) and anything else is refused at decode.

### Compositor: regional publication and composition

On Damage the broker checks every rectangle against the window's own
surface size (any rectangle outside: nothing is published) and copies
exactly those pixels from the client's staging into the private snapshot.
ADR-0067's rule becomes stronger, not weaker: staging bytes outside the
declared rectangles never become visible. Published regions are remembered
per session (merged, bounded to eight; overflow or a whole-surface Damage
degrades to "whole window") until the next presented frame, carried in the
retained scene, and `compose::damage` damages exactly those rectangles when
a window's only change is its published content.

## Proofs

* Host, randomized: Terminal and Editor edit sequences (typing, newline,
  backspace, caret motion in all directions, scrollback, status changes,
  theme changes) — the incremental raster equals a full reference redraw
  after every step, and most steps are partial. Composition: random window
  open/close/move/raise/reveal/focus, pointer, shell and theme changes plus
  partial publications that change only declared rectangles — incremental
  composition equals a full redraw pixel for pixel.
* Host RED (`tools/test_m11_repaint_red.py`): removing the caret column from
  row keys (old caret), the scrollback offset from row keys (newly exposed
  rows), the client run merge, the compositor region union, or the window
  offset of published regions each fails the matching equivalence test.
* Guest: the signed dynamic fixture repaints its whole staging raster and
  declares one 10×10 rectangle; the real screen changes only inside a box of
  at most 10×10 pixels (`tools/test_m10_dynamic.py`). The existing
  unpublished-staging oracle and `frame-publication` RED control still hold.

## Consequences

* A Terminal keystroke repaints and publishes the 24-row input band
  (10,752 pixels instead of 129,024); a key release publishes nothing.
* Bands are horizontal and full width; finer component-level regions can be
  added per view without changing the wire or the compositor.
