# Phase 11 — Desktop maturity: final report

Branch `arena/phase11-desktop-maturity`, not merged to main. The stage log
with every receipt is `PROGRESS.md`; this report summarizes what was built
and demonstrated, what was found, and what was not met.

## Qualification

* **Source:** `fcc09a5ab9f0530adddbd2a2b446f17df6001bf1`. It was
  checked out clean in its own worktree, and every later commit changes
  only `docs/phase11/` and `releases/checkpoints/phase11-complete/`.
* **Complete suite** on that clean source (20:07–21:36 UTC):
  **ALL TESTS PASSED (110 test suites)**, which is 96 `test_m*.py` plus
  14 host blocks. `QUALIFICATION SOURCE CLEAN: yes`, with the same commit
  at start and end. The log is `full-suite.log` in the archive.
* **Shipping EFI** (`tools/build.sh --image`, desktop profile; two builds
  gave the same bytes):
  `33134367244674cbba85d24ff634d918b27688f4bb878b0209a6a58fd876b806`.
* **Stability:** a fresh `tools/stability_loop.sh 100` from zero on that
  EFI (21:36–21:51 UTC) ran **100/100 boots fully green, fail = 0**. Each
  boot ran the full kernel suites, the network and console fixtures, the
  real desktop pixel/input/process proof and the file-service and
  desktop-surface markers. The receipt is
  `33134367…b806 100/100`, and the log is `100-boot.log`.
* **Archive:** `releases/checkpoints/phase11-complete/`. Its SHA-256 and
  the independent extracted boot are recorded below and in `PROGRESS.md`.
* **Earlier attempts:** six attempts before this one failed or were
  invalid. Each is recorded with its root cause in `PROGRESS.md`
  ("Qualification attempts"). None counts, and none was rerun until
  green without a cause.

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
| 11.9 directory watches | Bounded per-directory watches on held directory capabilities, exact object identity, both rename parents, lineage lifetime; Files and the desktop update from them (no poll) (ADR-0079; host RED for stale generation and missing rename parent; guest `test_m11_watch`) |
| 11.9 latency | Nine profiled costs removed (`PROGRESS.md` 11.9): no window re-damage after publication, occlusion, chunked tight damage, multi-rect present, bounded reply handoff (ADR-0072 amendment; m11 test plus RED), wake-before-reply, change-only receipts, clip-aware shell layers |

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
| 11.9 profile | Every client publication made the compositor redraw that whole window on the next frame (the Monitor: 132 kpx twice a second) | Frame equality ignores per-frame region bookkeeping; host test RED on the old code |
| 11.9 guest | The Desktop watch's record took the slack a launch reservation had counted twice, and the twelfth session was refused (`test_m10_apps`) | Reservation counts the request's landed cap once; refusal logs its status |
| 11.9 guest | After the reply handoff the desktop presented before boot transients ended, so `test_m10_dynamic` (and potentially every stability boot's pixel check) took a stale resource base | Base taken from receipts after the permission app is reaped |
| Historical RED | Four `test_m10_boundaries_red` needles had drifted (one control vacuous since 11.4) | Retargeted to the same boundaries in today's code; 12/12 RED |

## Not met, not built

* **Latency targets.** The figures are final-image, production, same-host
  medians over interleaved runs (`PROGRESS.md` 11.9, `latency-ab-11.9.json`):

  | Target | Measured | Status |
  |---|---|---|
  | Pointer motion-to-photon p50 ≤ 6 ms | 6.5–6.6 ms | **Not met** |
  | Terminal key-to-photon p50 ≤ 12 ms | 11.5–12.3 ms | At the target, not robust |
  | Terminal key-to-photon p95 ≤ 20 ms | 15.4–18.0 ms | Met |
  | Title-drag render p95 ≤ 12 ms | max 10.9–11.1 ms on two clean runs | Met |
  | Six idle apps ≤ 5 compositor wakes/s | 3.98 | Met |
  | Static idle clients 0 wakes/s | 0 | Met |

  Every host sample contains a 4.3–5.1 ms QMP screendump stall, while the
  guest pointer path is 1.5–2.1 ms. A revised statement of the gates, net
  of that measured stall, is proposed to M/C in `PROGRESS.md`. It is not
  assumed accepted.
* **Desktop rename** happens in Files; the desktop creates items under
  default names.
* One explorer view holds at most 256 items, and the desktop shows 24;
  both say when there are more.
* Body text and the terminal/editor grid stay on the 5x7 face.
* The historical inputd service mode still needs the five-key `arena`
  boot input before the desktop starts. The archive witness types it.
