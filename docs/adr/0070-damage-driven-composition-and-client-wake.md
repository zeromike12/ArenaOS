# ADR-0070 — Damage-driven composition and broker client wake

Status: implemented on `arena/phase10-desktop-maturity`; complete historical
suite 97/97 on clean source `1231e5a` (receipts in
docs/phase10/DESKTOP-MATURITY.md §8). Not a Phase-10 qualification claim.

## Problem

The desktop recomposed and presented the entire 800x600 scanout after every
state change: a pointer move cost a full background, every window blit, the
shell, and a 480k-pixel displayd copy done one volatile pixel at a time.
Built-in clients slept on a 20 ms pacing timer between polls, so an input
event waited up to one client period before it was even seen.

## Decision

1. **Canvas clip (gfxkit).** `Canvas` carries a clip rectangle (default: the
   whole canvas). `clear`, `fill_rect`, `blit` and text are intersected with
   it; fills and blits work on whole row slices. A host test proves a
   clipped scene equals the full scene inside the clip and leaves every
   outside pixel untouched.
2. **Retained scene (desktop `compose.rs`).** The compositor describes what
   it draws as a `Scene`: windows in stacking order (slot, handle, geometry,
   sampled reveal and focus amounts, title, published flag and a content
   generation advanced by every authenticated Damage), the shell facts
   (`shell::Shell`: pointer, window count, focus, running kinds, notice,
   uptime) and the theme. `compose::damage(prev, next)` lists rectangles
   whose pixels may differ: a window's old and new bounds (frame plus the
   deepest drop ledge) when any of its fields or its stacking position
   change; the bar, notice or dock region when their facts change (dock
   hover is derived from the pointer); the old and new pointer rectangles;
   the whole screen on a theme change. `compose::compose` draws a scene
   under the canvas clip. Rendering composes each damaged rectangle and
   PRESENTs exactly those rectangles; an empty damage list presents
   nothing. A randomised host test (160 steps over open/close, move, raise,
   content, reveal, focus, pointer, dock, notice, uptime and theme changes)
   proves incremental composition equals a full redraw pixel for pixel, and
   goes RED when either the old-pointer region or the drop-ledge margin is
   removed.
3. **Row-copy present (displayd).** For the identity scanout format the GOP
   PRESENT copies each validated row with one non-overlapping copy instead
   of per-pixel volatile operations. Rectangle validation and the
   synchronous single-writer contract are unchanged; swizzled formats keep
   the per-pixel encode path.
4. **Broker client wake.** When events are queued for a built-in session,
   the broker signals that session's private clock notification (which it
   already holds and delegated at spawn) with the pacing badge, once until
   the client next polls; after an appearance change it signals every
   built-in session. `app_client::idle` cancels its still-armed pacing timer
   after any wake so early wakes cannot accumulate timers in the bounded
   kernel table. Signed dynamic applications are not signalled: their
   timer behaviour is not under the broker's control. The compositor itself
   now polls its endpoint every scheduler tick (shortest timer) until
   Phase-11 bound notifications let IPC arrival wake it directly.
5. **Opt-in probes.** `arena_desktop::perf` and the `[perf ...]` lines exist
   only in images built with `ARENA_PERF=1`; default images fold them away.

## Authority

No grant, scope, capability transfer, wire format or lifecycle rule changes.
The scene is descriptive data copied from the window policy; damage only
chooses which pixels to recompute. Publication still happens only through
authenticated Damage into the private snapshot (ADR-0067). The client wake
uses a notification the broker already owned and delegated; it carries the
same badge as the client's own timer and conveys no data.

## Consequences

* Measured (TCG, same harness, frozen visual reference → this branch):
  pointer motion-to-photon 33.7 → 8.6 ms median, key-to-photon in Terminal
  106.9 → 28.6 ms median; Terminal event-to-damage 226 → 12 ms; every typed
  key now produces its own frame (previously most were coalesced).
* The compositor wakes on every 10 ms tick when idle (previously every
  ~25 ms). Each idle wake with no damage presents nothing.
* `desktop_view.rs` moved into the library as `arena_desktop::shell`
  (`system()` now takes `&Shell` instead of the window-policy `State`).
