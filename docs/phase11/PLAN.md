# Phase 11 plan — a real desktop: hierarchy, explorer, windows, primitives, speed

Owner: O (design and engineering). Inputs: the desktop maturity audit
([../phase10/DESKTOP-MATURITY.md](../phase10/DESKTOP-MATURITY.md)), ADR-0069/0070,
the Opus design requests ([../phase10/DESIGN-REQUESTS.md](../phase10/DESIGN-REQUESTS.md))
and ARCHITECTURE §10 ("files are capability objects; a namespace service
resolves paths once; paths are UI").

The visual skin stays frozen at `arena/phase10-opus-design-reference` until
workstreams K, C and W land: new surfaces (explorer, menus, resizable
windows) will be designed once the primitives they need exist, rather than
polishing today's fixed 448x288 layouts.

## Goals and measurable exit criteria

| Area | Exit criterion (QEMU TCG, `tools/profile_desktop.py`) |
|---|---|
| Latency | pointer motion-to-photon p50 ≤ 6 ms; key-to-photon (Terminal) p50 ≤ 12 ms, p95 ≤ 20 ms; title drag frame p95 ≤ 12 ms |
| Idle | ≤ 5 compositor wakes/s with six idle apps; 0 client wakes/s when nothing changes (Monitor excepted at its sample rate) |
| Filesystem | hierarchical volume ≥ 64 MiB; ≥ 10,000 objects; names ≤ 255 bytes; files ≥ 16 MiB; atomic rename/move; timestamps; crash-prefix proofs for every mutating op |
| Authority | apps hold directory/file capabilities, not a name-prefix scope; open/save through a trusted chooser; system records unreachable from user scope |
| Explorer | navigate, create folder, rename, move (drag and cut/paste), copy, trash/restore, sort by name/size/date, multi-select, open with — all on real AFS2 objects |
| Desktop | desktop folder shown as icons with persisted positions; open, drag, select, context menu |
| Windows | resize, maximize, minimize/restore, snap halves, keyboard switcher, menus/popups; ≥ 12 concurrent windows |
| Qualification | complete historical suite + new suites green; exact-image graphical stability 100/100 |

## Workstreams

### K — Kernel event plumbing (first; everything else leans on it)

1. **Endpoint-bound notifications** (ADR-0071). `SYS_ENDPOINT_BIND(serve-side
   endpoint READ, notification READ|WRITE, badge)`: a CALL queued while the
   server is not parked in RECV signals the bound notification. Bindings are
   cleared when either object is destroyed, when the endpoint is orphaned or
   taken over, and are never left pointing at a reusable notification index
   (destroy scans and unbinds under IF=0). The compositor then waits on one
   notification for "timer OR IPC" and stops tick polling.
2. **Direct IPC handoff** (ADR-0072). On CALL to a parked server and on REPLY
   to a blocked caller, switch directly to the woken thread (donating the
   remainder of the quantum) instead of re-entering round-robin. Bounded by
   the existing preemption tick; no priority inversion beyond today's RR.
3. **Timer quotas.** Per-process timer budget (e.g. 4) inside the 32-entry
   table (or grow the table), so no notification holder can exhaust timers.
4. **Badged endpoint capabilities** (ADR-0073). Minted endpoint copies carry
   an unforgeable badge delivered to the server with each request. This is
   the mechanism for userspace object capabilities (file handles, directory
   handles, surfaces) required by workstreams F and N.
5. Keep 100 Hz tick; revisit tickless only if idle measurements require it.

Tests: native ring-3 suites for bind/unbind/reuse (a destroyed and reused
notification index must never be signalled), handoff ordering, quota
refusal, badge unforgeability across copy/attenuate/transfer; RED mutants
for each; full historical suite.

### C — Composition and surfaces v2

1. **Damage rectangles on the wire** (Damage v2: up to 8 rects). The broker
   copies only damaged rows into the private snapshot, preserving ADR-0067's
   "nothing is visible before authenticated Damage" while making
   publication cost proportional to change. Present v2 takes a rect list in
   one displayd call; the GPU path transfers/flushes per rect.
2. **Client partial repaint.** Views paint under the gfxkit clip; Terminal
   repaints the input line, Editor the edited lines, Files the changed rows.
3. **Variable surface sizes.** Raise kernel graphics budgets (MAX_REGIONS
   8 → 48, TOTAL_PAGES 2048 → 24576, MAX_PAGES/region 512 → 1600 so a
   1024x768 surface fits) and replace the static 6 x 3 MiB snapshot array
   with per-session snapshots allocated from the session's own budget.
   Budgets are measured and re-qualified like Phase-10's 1231-page figure.
4. **Transient surfaces** for menus, tooltips and dialogs: child surfaces
   owned by a session, stacked above their parent, auto-dismissed, never
   focus-stealing from another session.
5. Move per-second native snapshot logging out of production images (keep
   it in the test profile the oracles use).

### W — Window management and input

1. Resize from edges/corners (8 px hit zones outside the content), with a
   Configure event carrying the new size; clients re-layout. Minimum/maximum
   sizes per window.
2. Maximize to the work area (screen minus bar and dock), restore, minimize
   to the dock with a visible indicator, snap left/right halves (drag to edge
   and keyboard).
3. Input model: modifier state (Shift/Ctrl/Alt/Super) in key events, key
   repeat in inputd with configurable delay/rate, tablet wheel (REL_WHEEL)
   → scroll events, double-click and drag thresholds, right-click → context
   menu request.
4. Keyboard switcher (Super/Alt+Tab overlay), window list in the dock,
   F-keys kept as fallbacks.
5. Clipboard service (text first, then file references) with explicit
   per-paste authority: the focused app receives clipboard data only on a
   user paste gesture.
6. Session limit from 6 to ≥ 12 desktop sessions within the new budgets.

### F — Filesystem: AFS2 and fsd v2

On-disk format AFS2 (ADR-0074), keeping AFS1's proven crash discipline
(CoW metadata, single-sector ping-pong commit written last,
two-generation-delayed freeing, data written before the commit that makes
it reachable):

* Superblock v2 with geometry from the device (≥ 64 MiB test volumes),
  4 KiB logical blocks, free-space bitmap blocks sized to the volume.
* Object table as a CoW tree of 4 KiB blocks keyed by 64-bit object id:
  record = type (file, directory, symlink reserved), flags, size, ctime,
  mtime, generation, link count, extent-tree root.
* Directories are objects whose data is sorted entry blocks
  (`name_len u8, name ≤ 255 bytes, object id u64, type u8`); lookup is a
  binary search per block; small directories fit one block.
* Multi-level extent trees (files ≥ 16 MiB; larger by format).
* Operations: lookup, create, mkdir, unlink, rmdir (empty), **rename/move**
  (atomic within the volume, cross-directory, cycle-checked), range write
  with CoW of overwritten blocks (transactional partial writes), truncate,
  set times, read-dir with stable cursors, statfs.
* Time: a userspace CMOS RTC service provides wall time; timestamps record
  "time unknown" honestly when it is absent.
* Change notification: per-directory watch delivering a badge on change.
* fsd v2 serves object handles as badged endpoint capabilities (K4) with
  rights per handle (read, write, list, create, delete, rename). Bulk data
  moves through a per-session shared I/O window (≥ 64 KiB), chunked.
* Migration: a one-shot, crash-safe importer mounts AFS1 read-only and
  writes an AFS2 volume: `/System` (configd, permission, package records),
  `/Users/user/{Desktop,Documents,.Trash}` (user files). Host tooling
  (`tools/afs2.py`) mirrors the format exactly as `afs1.py` does today.

Tests: host format/property tests (random op sequences against a model
filesystem), guest crash-prefix recovery for every mutating op (exact old or
exact new state), rename cycle refusal, no-space refusal without mutation,
directory watch delivery, migration from real Phase-10 disks, RED mutants
(in-place metadata write, early free, missing parent update).

### N — Namespace, authority and file service placement

1. **filesd** (new service) replaces the compositor's embedded file broker,
   so the compositor never blocks on storage. It holds the volume root,
   mints per-session directory/file capabilities and serves the desktop's
   file operations asynchronously.
2. Sessions receive capabilities, not scopes: Files gets the user home
   directory cap; editors and viewers get exactly the document caps the user
   chose. `user-*` name filtering and the `FILE_READ/WRITE` scope bits are
   retired after migration.
3. **Trusted chooser (powerbox):** Open/Save dialogs are drawn by the shell
   (not the requesting app) and return a capability for the chosen object.
4. Paths are resolved client-side by walking directory caps; there is no
   global path namespace with ambient authority.
5. `/System` is not reachable from any user-granted capability.

### X — Files explorer and Desktop surface

Built only on workstreams C, W, F, N and P:

* Explorer window: back/forward/up, breadcrumb path bar, sidebar (Home,
  Desktop, Documents, Trash), list view with sortable Name/Size/Modified/Kind
  columns and a grid view with 32 px icons, live updates from directory
  watches, keyboard navigation, type-ahead.
* Operations: new folder, inline rename (F2), move by drag-and-drop and by
  cut/paste, copy, delete to Trash with restore, empty Trash, properties,
  open / open with (type registry by extension and content sniffing).
* Selection: click, Shift range, Ctrl toggle, rubber band; multi-item drag.
* Desktop: `/Users/user/Desktop` rendered as icons on the desktop surface,
  positions persisted, double-click open, drag to arrange or into folders,
  desktop context menu (New Folder, New Document, Arrange).
* Editor and other apps open and save through the chooser.

### P — UI rendering primitives

1. gfxkit: origin translation (scroll views), opaque darken/blend rect and
   8-bit alpha-masked blit onto opaque targets (shadows, selection overlays,
   icons), 1-bit and 8-bit masked icon blit at 16/32/48 px, glyph-run fast
   path (row masks instead of per-pixel checks).
2. A second owned bitmap face with proportional advances (e.g. 7x13) and a
   Latin-1 subset; text measurement API shared by layout and hit testing.
3. Components: ScrollView + Scrollbar, virtualized ListView with columns,
   icon GridView, Menu/ContextMenu (on transient surfaces), TextInput with
   selection, Breadcrumb, Splitter, Dialog, focus traversal.
4. Design tokens extended for new surfaces after primitives exist (the
   visual pass resumes here).

## Kernel budgets that bound the desktop today

| Constant | Today | Phase-11 target | Driver |
|---|---|---|---|
| `shared::MAX_REGIONS` | 8 | 48 | per-window surfaces, transient surfaces, I/O windows |
| `shared::TOTAL_PAGES` / per region | 2048 / 512 | 24576 / 1600 | 1024x768 windows, maximize |
| `ipc::MAX_ENDPOINTS` | 12 | 24 | filesd, chooser, clipboard, RTC service |
| `ipc::MAX_NOTIFS` | 25 | 48 | one event notification per session plus directory watches |
| `timer::MAX_TIMERS` | 32 global | 64, ≤ 4 per process | timer quotas (K3) |
| `cap::CAP_SLOTS` | 32 / process | 64 | apps holding several document caps |
| `proc::MAX_PROCESSES` / `MAX_THREADS` | 32 / 64 | unchanged until W6 measures ≥ 12 windows | session count |

Every raise is a deliberate, measured change: the Phase-10 resource
receipts (`frames/records/processes/regions/pages/maps/caps`) are
re-derived and the suites that assert them are updated in the same commit
with the measured reason.

## Milestone 11.0 work breakdown

| Item | Files | New ABI | Proof |
|---|---|---|---|
| Bound notification | `kernel/kernel/src/ipc.rs`, `arch/x86_64/syscall.rs`, `userspace/abi.rs` | `SYS_ENDPOINT_BIND = 46` (endpoint, notification, badge); `SYS_ENDPOINT_UNBIND = 47` (next free after 45; gaps below are retired numbers) | ring-3 suite: CALL while server not in RECV signals the badge; unbind on destroy of either side; destroyed-and-reused notification index never signalled (RED: skip unbind on destroy) |
| Direct handoff | `kernel/kernel/src/sched/mod.rs`, `ipc.rs` | none | ordering suite (caller → server → reply order unchanged), latency probe: client poll round trip ≤ 1 ms in `profile_desktop.py` |
| Timer quota | `kernel/kernel/src/timer.rs` | `SYS_TIMER_ARM` returns a distinct quota error | fifth concurrent timer of one process refused, other processes unaffected (RED: quota check removed) |
| Event-driven compositor | `userspace/desktop/src/bin/desktop.rs` | uses bind | idle profile: ≤ 5 compositor wakes/s; the per-tick poll (`POLL_TICK_US`) is deleted |
| Event-driven clients | `userspace/desktop/src/app_client.rs`, `bin/application.rs` | none | idle profile: 0 client wakes/s for static apps; Terminal key-to-photon p50 ≤ 12 ms |

## Sequencing

| Milestone | Content | Depends on |
|---|---|---|
| 11.0 | K1 bound notifications, K2 direct handoff, K3 timer quotas; event-driven compositor and clients | — |
| 11.1 | C1–C2 damage rects and partial repaint; C5 logging split | 11.0 |
| 11.2 | K4 badged endpoint caps | 11.0 |
| 11.3 | C3–C4 variable and transient surfaces; budgets raised and measured | 11.1 |
| 11.4 | W1–W6 window management and input | 11.3 |
| 11.5 | F AFS2 format, fsd v2, RTC, migration | 11.2 |
| 11.6 | N filesd, capability grants, chooser | 11.5 |
| 11.7 | P primitives and components; visual pass resumes | 11.3 |
| 11.8 | X explorer and desktop surface | 11.4, 11.6, 11.7 |
| 11.9 | Qualification: full suite, perf gates, 100/100 | all |

## Design-request mapping

DR-01 typed status → P3/X; DR-02 terminal echo marking → C2; DR-03 file
sizes → F/X; DR-04 chrome pointer state and DR-05 reveal-aware chrome → W1;
DR-06 dock motion → W4; DR-07 client focus flag → W; DR-08 pressed state → P3;
DR-09 darken/blend → P1; DR-10 rounded windows → P1 (deferred); DR-11 second
face → P2; DR-12 Unicode punctuation → P2; DR-13 scrolling → P3; DR-14 theme
cross-fade → C (deferred).

## Risks

* AFS2 is the largest correctness surface. Mitigation: model-based host
  testing before guest code; crash-prefix proofs per op; AFS1 kept readable
  for migration only.
* Badged capabilities touch every IPC path. Mitigation: additive syscall,
  existing unbadged endpoints unchanged, dedicated RED suite.
* Raising kernel graphics budgets changes memory accounting the historical
  suites assert; budgets are re-measured and receipts updated deliberately.
* TCG timing noise: perf gates use medians over repeated runs and stay
  advisory until three consecutive stable runs.
