# Design primitive requests

Opus records requests here. Engineering reviews changes to compositor, kernel,
IPC, input, filesystems, package formats or authority topology. Do not silently
change those mechanisms to achieve a visual effect.

## Request template

Requested primitive:

Current limitation:

Why desired:

Affected presentations:

Engineering decision / ADR / test evidence:

Status: proposed / accepted / refused / implemented.

---

The Opus design pass (`arena/phase10-opus-design`) needed none of these to
ship a complete visual system; each entry names the graceful fallback that is
in place today. Requests are ordered by expected design value per unit of
engineering risk. None was implemented by Opus.

## DR-01 Typed status tone from application controllers

Requested primitive: controllers pass a small presentation enum
(`Neutral | Success | Warning | Error`) alongside the status string they
already hand to `view::frame` (and per scrollback line to `view::terminal`).

Current limitation: `application.rs` passes only `&'static str`. The view
infers tone by prefix (`view::tone`: `REFUSED`, `FILE NOT FOUND`, `SAVED`,
`MODIFIED`, ...). This is presentation-only and confers nothing, but it is
coupled to controller wording: renaming a message silently drops its tone.

Why desired: refusal/error/success styling (status band tint, glyph, terminal
error lines) should not depend on string spelling.

Affected presentations: every application status band; Terminal error lines.

Engineering decision / ADR / test evidence: requires a controller change in
`userspace/desktop/src/bin/application.rs` (engineering-review file).

Status: proposed.

## DR-02 Terminal command-echo marking

Requested primitive: one bit per scrollback line in `apps::model::Terminal`
recording whether the line is the echoed command or command output.

Current limitation: the model stores plain lines; the view cannot tell a typed
command from its result, so commands cannot carry the Signal prompt chevron
used on the live input line (the Gallery console sample shows the intended
treatment).

Why desired: command/result distinction is the main readability gain still
missing from Terminal.

Affected presentations: Terminal scrollback.

Engineering decision / ADR / test evidence: app model change plus model unit
test; no authority impact.

Status: proposed.

## DR-03 Per-file size in the Files view

Requested primitive: add the already-fetched `sizes[..count]` to
`view::FilesView`.

Current limitation: the controller lists real sizes but does not pass them to
the view. Today only the selected file's size is shown (derived from the
previewed byte count, which is the real file length when readable).

Why desired: a size column for every row; clearer empty-file vs refused
preview distinction.

Affected presentations: Files list rows and preview header.

Engineering decision / ADR / test evidence: one-field controller change.

Status: proposed.

## DR-04 Pointer-relative chrome state (close hover / press)

Requested primitive: pass the pointer position and primary-button state, or
a precomputed `hover_close` flag, to `desktop_view::chrome`.

Current limitation: `chrome(canvas, window, title, focused, amount, t)` has no
pointer input. The close well is always drawn (quieter at rest) instead of
reacting to hover/press. The dock *does* show hover because `system` receives
`State::pointer`.

Why desired: immediate affordance on the only destructive chrome control.

Affected presentations: window close control.

Engineering decision / ADR / test evidence: signature change in
`desktop_view.rs` plus one call site in `bin/desktop.rs`.

Status: proposed.

## DR-05 Reveal-aware chrome

Requested primitive: pass the sampled reveal height to `chrome`.

Current limitation: during open/close the frame, rail and drop ledge are drawn
at full window height immediately while content unrolls inside. The design
embraces this ("frame first, then content"), but a frame that grows with the
reveal would make close read as the window folding into its title bar.

Affected presentations: open/close motion.

Engineering decision / ADR / test evidence: signature change, one call site.

Status: proposed.

## DR-06 Dock and launch motion state

Requested primitive: a per-dock-item `Motion` in the compositor session table
(launch acknowledgement, running-indicator growth), driven by the existing
monotonic clock and 20ms pacing, collapsed by the motion preference.

Current limitation: the dock is redrawn from instantaneous state only.
`motion::DOCK_US` is reserved but unused.

Why desired: launch feedback between click and first published frame.

Affected presentations: dock.

Engineering decision / ADR / test evidence: compositor state change in
`bin/desktop.rs`; must keep refusal and capacity notices exact.

Status: proposed.

## DR-07 Application focus flag in paint

Requested primitive: keep the `Event::Focus(bool)` value that clients already
receive and pass it to views.

Current limitation: `application.rs` marks the frame dirty on focus change but
drops the value, so content cannot dim its caret/selection when the window is
inactive. Focus is fully expressed by compositor chrome today.

Affected presentations: Editor caret, Terminal block caret, Files selection.

Engineering decision / ADR / test evidence: controller change.

Status: proposed.

## DR-08 Pressed control state

Requested primitive: the controller already tracks button state; expose which
control rectangle is pressed so views can draw a `Pressed` state.

Current limitation: actions fire on press; there is no visible pressed frame.

Affected presentations: all buttons.

Status: proposed.

## DR-09 Opaque darken / blend-under primitive (shadow sampling)

Requested primitive: `Canvas::darken_rect(rect, amount)` (read-modify-write of
already composed opaque pixels) or full alpha compositing.

Current limitation: the drop ledge under windows, dock and toast is a solid
`shadow` token. Over the desktop it reads as a shadow; over another window it
reads as a hard dark edge (an intentional, consistent stacking cue). True soft
shadows, translucent bar/dock material and blur are not possible.

Why desired: depth without heavy edges; translucent system bar.

Affected presentations: windows, dock, notice toast, system bar.

Engineering decision / ADR / test evidence: gfxkit/compositor change; affects
per-frame cost.

Status: proposed.

## DR-10 Rounded window corners

Requested primitive: either DR-09, or the compositor painting window corner
pixels from the layer beneath.

Current limitation: windows are square because the compositor blits complete
opaque client rasters and cannot know what lies under a corner. Shell
surfaces drawn last (dock, toast, controls inside apps) are genuinely rounded
by leaving corner pixels unpainted.

Status: proposed (low priority; square windows are part of the current
identity).

## DR-11 Second type size or true bold face

Requested primitive: an Arena-owned 6x9 (or 7x11) bitmap face, and/or a
designed bold variant, selectable through gfxkit.

Current limitation: hierarchy uses the 5x7 face only: scale 1/2, upper-case
tracked captions, and a double-struck "Strong" style (one-pixel horizontal
emboldening, advance 7). Scale 2 is coarse for page titles, so titles use
Strong at scale 1 and Display is reserved for figures.

Why desired: smoother titles and denser, more legible body text.

Affected presentations: all text.

Engineering decision / ADR / test evidence: gfxkit text API change; text
geometry feeds editor/terminal layout and pointer-to-caret mapping.

Status: proposed.

## DR-12 Minimal Unicode punctuation glyphs

Requested primitive: a handful of owned glyphs outside ASCII (middle dot,
ellipsis, arrows, em dash).

Current limitation: ASCII only, so separators are `/`, truncation is `..`, and
scroll direction is described in words.

Status: proposed.

## DR-13 Scroll position primitive for lists

Requested primitive: a standard scrollbar component with a reviewed hit
contract, or at least a list `total/visible/top` triple passed to views.

Current limitation: no scrollbar exists. Monitor shows a numeric `1-11` range
when its process list overflows; Terminal shows `BACK n` and a line count.

Status: proposed.

## DR-14 Theme cross-fade

Requested primitive: a compositor-held previous frame and a bounded cross-fade
(or per-client palette interpolation) when the appearance preference changes.

Current limitation: theme changes repaint instantly in every client. This is
correct and responsive; a 120ms cross-fade would soften the switch. Not
possible without alpha or a dual-buffer blend.

Status: proposed (low priority).
