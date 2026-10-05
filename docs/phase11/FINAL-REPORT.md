# Phase 11 — Desktop maturity: final report

Branch `arena/phase11-desktop-maturity`, not merged to main. The stage log
with every receipt is `PROGRESS.md`; this report summarizes what was built
and demonstrated, what was found, and what was not met.

## Qualification

<<QUALIFICATION>>

## What Phase 11 delivered

| Stage | Delivered (proof) |
|---|---|
| 11.0 kernel events | Endpoint-bound notifications, direct IPC handoff with causal order, per-process timer quota; event-driven compositor and clients (ADR-0071/0072; m11 boot suite, 5 RED controls) |
| 11.1 partial repaint | Keyed bands in clients, regional Damage publication in the compositor (ADR-0073; host equivalence proofs, 5 RED controls; guest `test_m10_dynamic`) |
| 11.2 badged endpoints | Server-minted, generation-safe object handles (ADR-0074; m11 boot test, 2 RED controls; amended in 11.8: a mint may carry DESTROY) |
| 11.3 surfaces | Variable-size and transient surfaces, twelve sessions, measured kernel budgets (ADR-0075) |
| 11.4 windows | Drag, edge resize, maximize, snap halves, minimize/dock restore, Alt+Tab switcher, menus/popups, key repeat, chords, wheel (24 host tests; guest `test_m11_wm`) |
| 11.5 AFS2 | Hierarchical copy-on-write filesystem: crash-prefix proofs of every mutating operation and of format, 10,000 objects, 16 MiB files, fail-closed mount; filesd; AFS1 migration with the marker committed last; RTC timestamps or an honest "unknown" (ADR-0076/0077; host model + Rust engine with RED controls; guest `test_m11_afs2` with verified crash triggers) |
| 11.6 file capabilities | Lineage-scoped badged file and directory capabilities, rights attenuated along every walk, `/System` unreachable, trusted chooser (powerbox) for Open/Save/Read-Only, Files→Editor offers (guest `test_m11_files`, 14-check hostile probe, 2 RED controls) |
| 11.7 primitives and type | gfxkit origin/blend/darken/mask/bits blits; Arena Sans 13, an owned proportional face; widgets (scroll, virtualized list, grid, input, breadcrumb, splitter, selection); titles and captions set in Arena Sans |
| 11.8 explorer and desktop | Files explorer (toolbar, breadcrumb, sidebar places, sortable list and icon grid, multi-select with Shift/Ctrl/rubber band, drag-and-drop moves, cut/copy/paste, inline rename, Trash with restore records, properties, open by type) and the desktop surface (`/Users/user/Desktop` icons with persisted positions, open, drag to cells or into folders, context menus) (ADR-0078; guest `test_m11_explorer`, `test_m11_desk`) |

## Problems found by the proofs (none treated as a flake)

| Where | Problem | Resolution |
|---|---|---|
| 11.0 full suite | A handoff let a server overtake the STOP the caller had just notified | Causal-order rule (ADR-0072 amendment, RED control) |
| 11.1 receipt | `test_m10_dynamic` never passed after 11.1: the probe's 10x10 Damage sat under the opaque title band; a later oracle had never been reached | Rerun at the introducing commit confirmed it; probe and oracle corrected; whole test PASS |
| 11.5 guest | filesd stack overflow, endpoint table full, strict syscalls with dirty registers, non-reproducible EFI | Fixed; reproducible PE link flags |
| 11.6 guest | Rights lost along walks; implicit lineages; chooser rights; Backspace semantics | Fixed (see PROGRESS 11.6) |
| 11.8 guest | filesd kept every minted cap (IPC transfer copies) until its 64-slot space filled; the kernel refused the destroy (badged mints had no DESTROY) | filesd drops its copy; ADR-0074 amendment with a kernel test |
| 11.8 guest | Launch/Started kept the Phase-10 file-name rule: opening "Desktop/New Folder" made the broker die (graphics halted fail-closed); names with spaces could not open | Both frames take the printable title rule |
| 11.8 guest | The desk lost the press after a menu click (stale button state) | Unrouted pointer state is tracked; host RED test |
| Historical RED | Four `test_m10_boundaries_red` needles had drifted (one control vacuous since 11.4) | Retargeted to the same boundaries in today's code; 12/12 RED |

## Not met, not built

* **Latency targets** (`PROGRESS.md` 11.9 table): pointer motion-to-photon
  p50 10.6 ms (target ≤ 6), Terminal key-to-photon p50/p95 17.8/23.6 ms
  (≤ 12/≤ 20), title-drag compositor work 12.9 + 8.2 ms per frame
  (p95 ≤ 12). Idle targets are met (4.87 compositor wakes/s with six
  apps; static clients 0).
* **Directory watches** were not built. Files re-lists a changed folder
  on focus, interaction or request. The broker re-checks the Desktop
  folder once a second on its existing uptime tick.
* **Desktop rename** happens in Files; the desktop creates items under
  default names.
* One explorer view holds at most 256 items, and the desktop shows 24;
  both say when there are more.
* Body text and the terminal/editor grid stay on the 5x7 face.
* The historical inputd service mode still needs the five-key `arena`
  boot input before the desktop starts. The archive witness types it.
