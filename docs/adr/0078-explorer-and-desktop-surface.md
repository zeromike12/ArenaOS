# ADR-0078 — The Files explorer and the desktop surface

Status: accepted (Phase 11.8; guest-proven by `tools/test_m11_explorer.py`
and `tools/test_m11_desk.py`).

## Problem

Phase 11.6 put user files on AFS2 behind capabilities (ADR-0077). Two
surfaces had to become real on top of it:

* a file explorer;
* the desktop as a view of `/Users/user/Desktop`.

Neither may add authority. Paths stay presentation, and every effect
must go through a capability that already exists.

## Decision

**Explorer = logic over a `Store`.** `apps/explorer.rs` holds navigation,
history, sorting, selection, rename, Trash, clipboard, drag moves and
open-by-type. It has no pixels and no IPC. The application implements
`Store` as `CapStore`: every path is walked from the session's own
/Users/user capability (`files::walk`), and intermediate capabilities are
released at once. Host tests use an in-memory store with filesd's refusal
semantics. `explorer_view.rs` holds geometry and drawing as pure
functions; `explorer_ctl.rs` maps input to operations. Repaint keys are a
hash of everything the view draws.

**Trash.** A deleted item is renamed into `/Users/user/.Trash` under a
free name. A record `.Trash/.restore/<name>` holds its original path as
presentation text, resolved from the root capability again on restore.
A vanished folder restores to home. Inside the Trash, Delete is
permanent.

**Opening.** The kind comes from the extension, then a content sniff
(the Editor's own acceptance rule). A text file opens through the
ADR-0077 offer: Files offers its capability and the broker re-grants it
in the new Editor's lineage. Other kinds say no application can open
them.

**Desktop surface in the broker.** The broker already holds
`/Users/user` (record 1) for the trusted chooser, and it now also lists
`Desktop` through it. Icons are a comparable `DeskView` in the shell
scene: drawn on the background under the windows, with the desk menu
above them. Damage covers exactly the changed cells, drag ghosts and menu.

* **Positions** are user data in `Desktop/.positions` (`col row name`
  lines), not authority. Malformed lines are ignored and unknown names
  take free cells.
* **Opening a document** walks the file in the broker's lineage, then
  launches the Editor with it. `file_grants` re-grants it in the Editor's
  lineage, and the broker releases its own record.
* **Opening a folder** launches Files with that folder as a printable
  start path. It is presentation inside the home capability Files is
  granted anyway.

**Input routing.** A press on bare desktop, and every pointer event
while the desk holds a press or its menu, goes to the desk; the window
policy sees that pointer without buttons. A bare press blurs the focused
window; with no focus, Enter, Delete and Esc act on the selected icons.

**Wire changes.** Pointer events carry Shift and Ctrl in button bits 6
and 7. Shifted navigation keys (codes ≥ 256) arrive as chords. Launch
and Started paths are printable titles, both ways, exactly as `launch`
validates them.

**Freshness without watches.** There are no directory watches. Files
re-lists a changed folder (by its version) on focus, on interaction and
on request, and never wakes itself while idle. The broker re-checks the
Desktop folder's version once a second on its existing uptime tick.

## Consequences

* A folder holds at most 256 items in one explorer view; the status
  line says when a folder holds more. The desktop shows 24 icons.
* Desktop rename happens in Files: the desk creates new items under
  their default names.
* filesd must not leak its minted caps. The explorer's many opens
  exposed exactly that leak (ADR-0074 amendment).
