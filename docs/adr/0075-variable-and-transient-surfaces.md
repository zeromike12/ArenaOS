# ADR-0075 — Variable and transient surfaces; twelve desktop sessions

Status: accepted (Phase 11.3). Budgets below are re-measured in the
qualification receipts (`docs/phase11/PROGRESS.md`).

## Problem

Phase-10 windows were fixed at 448x288: the broker allocated one 127-page
shared region per session (I/O page + raster) and kept a static
6 x 448 x 288 private snapshot array. Resizing, maximizing, menus and
dialogs need surfaces of other sizes and surfaces outside the window
rectangle, and the desktop was capped at six sessions by kernel tables
(regions 8, pages 2048, maps 32, capability slots 32, notifications 25,
16 registered user windows per thread).

## Decision

### Surface reservation per session

At startup the broker computes the **work area** (screen minus system bar
and dock) and reserves for every session, once, at launch:

* a **shared region**: 1 I/O page + `ceil(work_w * work_h * 4 / 4096)`
  main-surface pages + 64 transient pages (65,536 pixels);
* a **private snapshot region** of the same surface + transient pages,
  created by the broker, mapped by the broker and **its capability
  destroyed immediately** — it lives exactly as long as the broker's
  mapping pin (ADR-0056), can never be granted to anyone and costs no
  capability slot.

A resize never reallocates: any size from the client's declared minimum
up to the work area fits the reservation, so the session's identity (the
shared region id, the session's only identity) never changes and there is
no window in which a stale region could be presented. The main surface
starts at byte 4096 with stride = surface width (byte-compatible with the
Phase-10 signed fixture); the transient surface occupies the final 64
pages of both regions.

Measured cost at 800x600 (work area 800x518): shared 470 pages, snapshot
469 pages, **939 pages (3.7 MiB) per session**, 11,268 pages for twelve
sessions plus the 469-page scanout. At 1024x768: 1,501 pages per session,
18,780 in total. The guest has 512 MiB.

Alternative rejected: reallocating a surface region on every growing
resize. It needs a capability transfer and remap in the middle of a
resize drag, a second identity for "the surface" beside the session
region, and a protocol for presenting the old raster while the new region
is not yet published. The reservation trades memory for a resize path
with no allocation, no new capability and no stale-surface state.

### Protocol (desktop wire, all frames canonical, Phase-10 bytes unchanged)

* `Create{w,h}` — any size from 80x60 to the work area.
* `Resizable{handle, min_w, min_h}` — opt in; fixed-size clients (the
  signed fixture, the Gallery sheet) are never resized, maximized or
  snapped.
* Policy resize → `Event::Configure{w,h}` (coalesced: a burst of resizes
  costs one queue slot). The client re-lays out, paints the whole surface
  and sends `Resize{handle,w,h}`; the broker accepts it only for exactly
  the configured size and copies exactly `w*h` pixels. Until then the old
  raster is shown clipped and the rest of the frame filled — nothing the
  client did not publish becomes visible (ADR-0067 unchanged).
* `Popup{owner, kind, x, y, w, h}` → transient surface handle;
  `Damage{popup handle, rects}` publishes it; `Dismiss{handle}` closes it;
  `Event::PopupPointer` and `Event::Dismissed(handle)` report input and
  policy dismissal.

### Transient surfaces (window policy, `model.rs`)

One per window, at most 512x384 and 65,536 pixels. Only the focused
window's own client may open one (no focus stealing); a second open
replaces the first and its handle goes stale (handles come from the same
never-reused counter as windows). Kinds: **Menu** (dismissed by any press
outside it — the press is consumed — by focus loss and by moving or
resizing the owner), **Tooltip** (takes no input; dismissed by any press
or focus change), **Dialog** (modal: presses on the owner's body are
swallowed; survives focus changes; moves with its owner). Transient
surfaces are clamped fully on screen and drawn directly above their
owner, so a window above the owner covers both. Nothing of a transient
surface is visible before its first Damage.

### Kernel budgets (measured, not guessed)

| Constant | Before | After | Why |
|---|---|---|---|
| `shared::MAX_REGIONS` | 8 | 32 | scanout + 2 x 12 sessions + headroom |
| `shared::MAX_PAGES` | 512 | 1024 | one 1024x768 work-area reservation (751) |
| `shared::TOTAL_PAGES` | 2048 | 20480 | 18,780 at 1024x768 |
| `shared::MAX_MAPS` | 32 | 64 | broker 1 + 24, clients 12, displayd |
| `sched::USER_REGIONS_MAX` | 16 | 40 | broker maps scanout + 24 session regions |
| `cap::CAP_SLOTS` | 32 | 64 | broker: 22 fixed + 2 per session (region, Process); 32-process table +25,600 B |
| `ipc::MAX_NOTIFS` | 25 | 31 | twelve client clocks; still exactly full at boot |
| `ipc::QUEUE_DEPTH` | 8 | 16 | callers queued per endpoint: a focus change makes all twelve clients plus the input producer call the broker at once (found by the guest proof: the eleventh client's first CALL was refused BUSY); 12 endpoints 23,616 -> 46,656 B |
| `spawn::MAX_SPAWN_RECS` | 24 | 32 | 14 boot processes + 12 sessions = 26 (found by the guest proof: the eleventh session was refused); one per possible process |
| desktop `LIMIT` / `MAX_WINDOWS` | 6 | 12 | |

`SYS_SHARED_MAP` reserves page-table frames per 2 MiB spanned (a region
above 2 MiB needs more than one PT). The servicemgr keeps its own 32-cap
policy bound; its proven schedules still fit 32.

## Proof obligations

* Host: window policy (ownership, focus-only open, stale handles, outside
  press dismissal, modal dialogs, on-screen clamping, coalesced Configure,
  twelve windows and the thirteenth refused); composition of variable
  surfaces and transient layers equals a full redraw over hundreds of
  random steps; application partial repaint equals full repaint at
  several sizes; layout fits at every supported size.
* Guest: twelve real sessions with exact resource receipts and the
  thirteenth refused with nothing allocated; resize/maximize publishes a
  correctly sized raster; a context menu opens, is dismissed by an outside
  press and its stale handle is refused; client death with an open
  transient surface releases everything.
