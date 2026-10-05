# Phase 11 progress and evidence

Branch `arena/phase11-desktop-maturity`, from the qualified desktop-maturity
tip (`d288cd7`, containing qualified source `1231e5a`). Every figure below
is QEMU TCG on the same host; absolute times include a ~4.8 ms host
screendump per sample and are only meaningful against the same-host
baseline in the same table.

## 11.0 — Kernel event plumbing

ADR-0071 (endpoint-bound notifications, per-process timer quota) and
ADR-0072 (direct handoff to a server woken by a blocking caller).

| Measure (`tools/profile_desktop.py`, 30 trials) | Baseline `1231e5a` | 11.0 |
|---|---|---|
| Pointer motion-to-photon p50 / p95 | 14.1 / 21.7 ms | 12.3 / 15.2 ms |
| Terminal key-to-photon p50 / p95 | 35.0 / 47.9 ms | 17.3 / 27.1 ms |
| Client poll IPC round trip (mean) | 4.66 ms | 0.93 ms |
| Compositor wakes/s, empty desktop | 195 | 1.06 |
| Compositor wakes/s, six idle apps | 163 | 4.69 |
| Static idle client wakes/s | (20 ms pacing: 50/s each) | 0 (Monitor 1.95, its sample rate) |

Raw receipts: `profile-11.0-baseline-1231e5a.json`, `profile-11.0.json`.

Targets met: idle compositor ≤ 5 wakes/s with six apps, static clients 0
wakes/s. Not yet met: key-to-photon p50 ≤ 12 / p95 ≤ 20 ms and pointer p50
≤ 6 ms. The remaining key path cost is the Terminal repainting and
publishing its whole 448×288 raster per key (paint ~1.3 ms, Damage copy
~1 ms, recompose/present of the full window ~6 ms) plus a second Damage per
key from the key-release event; that is milestone 11.1's work.

Proofs: `m11` boot suite 6/6; ring-3 timertest `STATUS_QUOTA`;
`tools/test_m11_event_red.py` — 5 production RED controls (bound signal,
notification-destroy unbind, orphan unbind, timer quota, handoff) each
observed failing in a real guest, byte-exact restore, GREEN.

Bug found and fixed during 11.0: a reply-path handoff variant livelocked
the phase-9 polling fixture (ADR-0072 records the mechanism).
`tools/test_m10_client_death.py`'s hung-client mutant was updated for the
new `idle(deadline)` signature (same intent: a client that never polls).

### Complete suite on the first 11.0 commit (`a1327aa`)

`tools/run_tests.sh` in a clean worktree, 18:55–20:10: **94 of 98 suites
passed, 4 failed**. None is treated as a flake; each has a recorded cause
and fix:

| Failed suite | Cause | Fix |
|---|---|---|
| Phase 8.2 cap/IPC layout audit | ADR-0071 grew `Endpoint`/`Notif`; the audit's struct extractor missed `Binding` and mis-decoded LLVM's `\\` | receipts re-derived (later commit) |
| `test_m10_ui.py` | rustfmt/clippy on new code | formatted (later commit) |
| `test_m8_dependencies.py`, `test_m8_stop.py` | **real regression**: ADR-0072 handoff let a CALL's server overtake the STOP the caller had just notified (`m8: stackstop FAIL (old bearer accepted or wrong transport)`) | `40279e8`: a handoff never overtakes the caller's own earlier wake (ADR-0072 amendment, m11 test, RED control `handoff-causal`) |

A complete rerun on the final commit is still required (11.9).

## 11.1 — Keyed partial repaint and regional publication (ADR-0073)

Host proofs: terminal/editor partial repaint equals full repaint (random
edits, now also at 400x200, 712x470 and 1024x768); compositor regional
publication equals full composition. `tools/test_m11_repaint_red.py`: five
RED controls (old caret, exposed rows, client merge, region merge, region
offset), GREEN after byte-exact restore. Guest: `test_m10_dynamic` 10x10
regional Damage.

## 11.2 — Badged endpoint capabilities (ADR-0074)

`m11:test:badged_endpoint` (mint authority, transfer/copy/attenuation,
stale generation, teardown); RED controls `badge-generation`,
`badge-mint-authority`.

## 11.3 — Variable and transient surfaces (ADR-0075)

Measured reservation at 800x600: shared 470 + snapshot 469 pages per
session. Kernel tables raised with receipts (see ADR-0075 table); the
capspace audit now measures the production 64-slot table (32-process
table 52,736 B, +25,600 B) and 31 notifications (992 B).

## 11.4 — Window management and richer input

Host: 24 window-policy tests (drag threshold, double-click, title wells,
edge resize with minimum/work-area/bar clamps, snap preview and halves,
Super placement, minimize/dock restore, Alt+Tab, key repeat without
stacking on host autorepeat, chords, wheel), 600-step composition
equivalence including chrome hover, switcher, snap outline and dock
state. Guest proof: `tools/test_m11_wm.py`.

## 11.5 — AFS2 (ADR-0076), filesd (ADR-0077), RTC

Host: the Python model `tools/afs2.py` and the Rust engine
`userspace/afs2` cross-check images both ways. Engine proofs cover
randomized operations against a model with remount and audit after every
step, directory split and merge, every crash prefix of every mutating
operation (exactly old or exactly new; a torn commit is old), and every
crash prefix of `format` (never committed, or mountable and clean).
They also cover 10,000 objects, a 16 MiB file and fail-closed mount.
`tools/test_afs2_rust.py` runs five RED controls: in-place metadata,
early free, missing parent update, commit not last and
reformat-damaged.

Guest (`tools/test_m11_afs2.py`, the real desktop, AFS1 seeded by a real
boot plus host records):

* Migration: 5 AFS1 files are imported. User files go to
  `/Users/user/Documents`, the rest to `/System/imported-afs1`, and the
  marker is committed last (129 block writes). The host model finds the
  exact namespace and bytes, the AFS1 sectors are byte-identical, and
  timestamps come from the RTC.
* A second boot mounts with zero writes.
* Interrupted import: boots are killed after BLKW4K #1, 2, 3, 32, 64 and
  96. Each kill is verified to fall inside the import (no completion
  line). The crashed region is either never committed or committed
  without the marker, and the next boot re-imports to the exact
  namespace. The kill lands a few writes after its trigger (the log
  records how many were logged), so the never-committed state is reached
  in some runs and not in others. That property is also proven
  exhaustively on the host.
* A committed volume with both commit records destroyed is refused (fail
  closed) and is byte-identical afterwards.
* With no clock, filesd reports "unknown" and every timestamp is 0. The
  absent clock is injected at the kernel RTC reader, because OVMF
  rewrites an out-of-range CMOS date: `-rtc base=1990` booted as 2090.
  The source and EFI are restored byte-exactly.

Found by the guest:

| Problem | Fix |
|---|---|
| filesd faulted at startup | The engine built a whole `Volume` (368 KiB) as a stack temporary; it now resets in place |
| The endpoint table was full | `MAX_ENDPOINTS` raised from 12 to 16 (ADR-0075 table) |
| The strict syscalls failed through `syscall1`/`syscall2` (garbage in the unused argument registers) | They are called with explicit zeros |
| The kernel EFI was not reproducible: relinking identical sources changed the PE timestamps and PDB GUID | `kernel/.cargo/config.toml` now passes `/Brepro /timestamp:0 /DEBUG:NONE`; three relinks are byte-identical |

## 11.6 — File capabilities and the trusted chooser (ADR-0077)

The desktop applications use AFS2 only through filesd capabilities. The
AFS1 `user-*` function scopes are no longer granted; Settings keeps its
AFS1 preference record.

| Application | What it holds and does |
|---|---|
| Terminal | A `/Users/user` capability. `ls cd pwd cat put mkdir rm rmdir mv` resolve paths client-side; `/` is home and `..` never climbs out. |
| Files | Browses folders. Opening a file offers its capability; the broker re-grants it, attenuated, in the new Editor's lineage, but only after filesd confirms the offer belongs to the requesting session's own lineage. |
| Editor | Holds one document capability. Open, Open Read-Only and Save As go through the broker's trusted chooser. |

The chooser is drawn by the shell and is modal: its keys and presses never
reach applications. It resolves names through the broker's own filesd
page and asks before replacing a file. Any windowed session may ask it,
signed applications included, because asking grants nothing.

Guest (`tools/test_m11_files.py`) PASS on a migrated volume:

* The terminal's put, mkdir and mv results are exact, and five `/System`
  attempts change nothing.
* A cancelled chooser grants nothing; a read-write grant saves exact
  bytes.
* After a rename, the save lands in the renamed file.
* After a delete, a save through the stale capability is refused and
  creates nothing; a Save As then writes the exact held text.
* A read-only grant's save is refused, and a Save As copy proves the text
  was really typed.
* A 2,592-byte signed hostile probe, granted one document read-only, gets
  14 raw requests answered as refusals. Its lineage (3 records) is
  retired when it closes.
* The AFS1 originals stay untouched.

RED (`tools/test_m11_files_red.py`): the no-attenuation and shallow-revoke
mutants each fail the guest test, and the source and EFI are restored
byte-exactly.

Found by the guest:

| Problem | Fix |
|---|---|
| Lineage creation was implicit, so the broker had no session for its own walks | `OP_NEW_LINEAGE` |
| Path walks lost rights at intermediate folders | Each intermediate opens with LIST plus the final rights |
| The chooser's folder lacked the rights it grants | The folder is opened with them |
| In the chooser, Backspace on an empty name left the folder | Backspace edits the name; Left goes up |

An unpaced burst of 62 key events overran inputd's bounded 64-entry key
ring. That ring is the documented design (ADR-0026); the test now paces
its keys like typing.

## 11.7 — UI primitives, Arena Sans 13 and the typography pass

* **gfxkit:** origin translation, opaque blend and darken rectangles,
  8-bit alpha-masked blit, 1-bit row-mask blit, and i64 pen arithmetic
  (hostile origins never overflow); host tests.
* **Arena Sans 13:** an owned proportional face drawn in
  `tools/face13.txt` (106 glyphs), generated into `face13.rs`. The suite
  checks the art and the generated face stay in sync. `measure13` is
  shared by layout and hit testing.
* **Widgets** (`ui/src/widgets.rs`): scroll and scrollbar, a
  1024-item selection, a virtualized list with sortable columns, an icon
  grid, a text input with selection, a breadcrumb, a splitter, focus
  order, and ellipsized text. 8 host tests.
* **Typography pass:** Strong (window titles, names) and Caption (labels,
  status lines, bar and dock text) are set in Arena Sans 13. Its capitals
  sit in the old 5x7 box, so every baseline holds. Body and Display keep
  the 5x7 grid, which the terminal and editor grids depend on. The
  explorer and the desktop icons use Arena Sans throughout, with 32 px
  folder and document icons drawn from shapes.

## 11.8 — Files explorer and the desktop surface

Files is now an explorer over the session's /Users/user capability
(`apps/explorer.rs` logic over a `Store`, `explorer_view.rs` pixels and hit
testing, `explorer_ctl.rs` interaction). It has:

* a toolbar with back, forward, up, a breadcrumb and a list/grid toggle;
* a sidebar of places (Home, Desktop, Documents, Trash) with a splitter;
* a virtualized list with sortable Name/Size/Modified/Kind columns, or a
  grid of 32 px icons;
* selection by click, Shift range, Ctrl toggle and rubber band;
* multi-item drag onto folders and onto places (onto the Trash: delete);
* keyboard: arrows, Home/End, Enter, Backspace, Delete, Esc, type-ahead,
  Alt+Left/Right/Up, Ctrl+A/C/X/V, Ctrl+N, Ctrl+Shift+N, Ctrl+R (rename;
  F1-F6 launch applications), Ctrl+O, Ctrl+I, Ctrl+1/2;
* inline rename, context menus chosen by what was clicked, a Properties
  sheet, delete to the Trash with a restore record, restore, empty Trash;
* open by type, with "Open With Editor" for text by content sniff.

The broker adds Shift and Ctrl to pointer buttons (bits 6 and 7), and
Shifted navigation keys arrive as chords. A folder changed elsewhere is
re-listed on focus, on interaction and on request. There are no directory
watches (not built): Files does not wake itself while idle.

Guest (`tools/test_m11_explorer.py`) PASS, judged on AFS2 bytes: new folder
named inline, copy/paste into a folder entered by double-click, history
back, cut/paste through a place, delete to the Trash with its restore
record, a drag move that takes a free name, restore by context menu,
keyboard rename, double-click open in a real Editor with an exact save,
empty Trash; sorting and the grid by pixels; a terminal change seen on
focus; AFS1 untouched.

Found by the guest:

| Problem | Fix |
|---|---|
| filesd kept the badged cap it minted for every OPEN reply (IPC transfer copies); its 64-slot space filled after a few dozen opens and every mint failed (S_FULL) | filesd destroys its copy after each reply |
| The kernel refused that destroy: badged mint rights were WRITE with optional COPY, and a non-landed cap without DESTROY cannot be destroyed | ADR-0074 amendment: a mint may carry DESTROY (it only empties the holder's own slot); `m11:badged_endpoint` proves both cases |
| Each S_FULL looked alike | filesd logs which limit refused (records, lineage quota, own space) |

Historical correction (11.1): `test_m10_dynamic` never passed after 11.1.
Rerun at its own introducing commit 6eab591, it fails identically. The
probe published its 10x10 Damage at surface (0,0), under the 28-pixel
title band the compositor draws opaquely. The rectangle is now at (10,30).
The test's next step then exposed a second never-reached oracle: after a
partial publish, a full publish changes only that one cell. The oracle now
requires the exact pre-partial raster. Guest: the whole test PASS.

The desktop surface (ADR-0078) shows `/Users/user/Desktop` as icons drawn
by the shell. It supports:

* positions persisted in `Desktop/.positions`;
* click and Ctrl selection;
* drag to a cell, or onto a folder icon to move into it;
* double-click open by type: a text file opens in an Editor granted
  exactly that file, a folder opens in Files;
* menus: New Folder, New Document, Arrange Icons, Open, Move to Trash;
* Enter, Delete and Esc when no window is focused.

Guest (`tools/test_m11_desk.py`) PASS: icons appear for objects the
terminal made. A dragged cell is persisted and drawn identically after a
fresh boot. A desktop open gives a granted Editor that saves exactly; an
image opens nothing. The menu creates a folder; a drop onto a folder icon
moves; Move to Trash leaves a restore record; a folder opens in Files.

More found by the guest:

| Problem | Fix |
|---|---|
| Launch and Started still required the Phase-10 file-name rule while `launch` accepted any printable title. Opening "Desktop/New Folder" made the broker die (stage 78) and the graphics service halt fail-closed; any Files open of a name with a space was refused | Both frames take the printable title rule (host test; the explorer test opens "user-note 2") |
| The desk kept the button state of events it was not given, so the press after a menu click read as a move | The broker passes every unrouted pointer's buttons to the desk (host test, RED without the fix) |

Historical receipts with AFS2 online (measured):

* Retained frames: 28 = 12x2 broker PTs + 4 filesd PTs.
* Broker cap high-water: 60 = 23 + 3x12 + 1 (each session's lineage
  head).
* Maps at the settled peak: 44 = 4 + 36 + 4 (filesd maps each of four
  file sessions).

`test_m10_files` was rewritten for the explorer, keeping every Phase-10
invariant (supersessions in its docstring). The `test_m10_boundaries_red`
pointer-hit mutant had been vacuous since 11.4: `State::hit` is
host-only, so it now breaks the live hit in `State::target`.
`test_m10_service_death`'s needle followed the key arm.

Qualification disks: `stability_loop.sh` and `run-desktop.sh` boot 72 MiB
disks, so filesd formats AFS2 and imports AFS1 each boot. The loop also
requires the file service and the desktop surface to come online.


## 11.9 — Directory watches (ADR-0079)

The accepted goals required directory change notification, and it had not
been built. filesd now keeps bounded per-directory watches:

* Authority comes from a held directory record with `R_LIST`, never from
  a path.
* The client lends a notification and names a badge bit.
* The table is capped at 24 watches in all and 4 per lineage, and a
  refusal happens before anything is kept.
* A watch matches the exact AFS2 object id, generation included.
* A rename signals both parents.
* rmdir tells the removed directory's watchers it is gone.
* A watch ends with its record: unwatch, release, revoke, or lineage
  retirement (process death).

The badge is a hint, and `OP_WATCHED` is the authoritative answer. Files
re-lists the folder it shows from its watch, and the focus-time refresh
is gone. The broker watches `/Users/user/Desktop`, and the one-second
Desktop poll is removed.

* **Host.** `filesd/src/watch.rs` has 4 tests. `test_watch_red.py` is RED
  for stale generations (an index-only match) and for a missing
  rename-parent notification, and restores source byte-exactly.
* **Guest.** In `test_m11_watch.py`, Files is snapped left and unfocused
  while the Terminal changes the folder. Files updates for:
  * a create;
  * a rename into the folder;
  * a contents change;
  * a rename out of the folder;
  * removal of the watched folder (Files falls back to home).

  Thirty folder switches never exhaust filesd. Closing Files ends its
  watch with the lineage. The desktop shows a file the Terminal wrote,
  with no poll and no click.

A regression this caused, and its fix: the broker's Desktop watch record
is one more permanent broker cap. That cap took the slack a launch
reservation had been double-counting (the request's already-landed cap),
so `test_m10_apps` refused the twelfth session (status −4, BUSY). The
reservation now counts that cap once (6 free slots), and the twelfth
session launches again. The broker cap peak is 61 = 24 + 3×12 + 1.

## 11.9 — Latency investigation on the final feature set

Each cost was found with the perf probes before anything was changed:

| Finding | Evidence | Fix |
|---|---|---|
| Every publication redrew its whole window on the next frame | Monitor: 132 kpx broker render twice a second for a 31-pixel change; `frame()` compared the per-frame `regions` bookkeeping | Regions stripped from frame equality; host test, RED on the old code |
| Large dirty bands were published whole | Monitor published 118k px per tick (the band exceeded the scratch) | Row-chunked tightening; about 31 px per tick |
| Drag compose drew every window under the top one | 4.0 ms compose per drag frame | Occlusion under the topmost revealed window: 1.9 ms |
| One display IPC per rectangle | Present dominated small frames | `PresentRects`: four rectangles per call, all checked before any copy |
| A resource receipt on the serial line after every input frame | 3.8 ms per pointer frame, plus the console driver mirroring it | Logged only when a counter changes; lifecycle receipts still always log |
| The broker waited behind the whole ready ring after every display reply | Cursor PRESENT 3.5 ms for a 0.1 ms copy; a scheduler trace showed the ring ahead of the broker | Bounded reply handoff (ADR-0072 amendment; m11 test plus RED) |
| A key's release was handled before the application saw the key | Key-to-poll 4.5 ms; inputd's release ran ahead of the woken Terminal | Clients are woken before the reply (causal rule) |
| A cursor-sized repaint laid out the emblem, bar and dock | Host compose of a 14×22 clip: 30 µs, 77% in the background layer | Layers skip boxes that miss the clip (0.7 µs) |
| An empty poll after each event | One extra round trip per key | Poll reports whether more events are queued |

Not changed, with evidence:
* The GOP copy is already `rep movsq`. The remaining drag present cost
  (about 6 ms for 100 kpx) is QEMU's emulated VRAM writes, roughly
  60 ns/pixel.
* A null syscall costs about 21 µs under TCG. The broker spends about
  ten per request.
* The host screendump stalls the guest for 4.3–4.7 ms and sits inside
  every host motion- and key-to-photon sample.

## 11.9 — Measurements on the final image

The first-draft figures (column "11.9 draft") came from
`tools/profile_desktop.py` on the perf image. They are in
`profile-11.9.json`.

The final figures come from `tools/latency_ab.py`:
* the production image (no probes);
* 30 trials per run;
* runs interleaved with a baseline build on the same host, in the same
  container session.

The raw data is in `latency-ab-11.9.json` (session 1: af14e20 vs 44559f2) and `latency-ab-11.9-session2.json` (session 2: 44559f2 vs the final 674eff8). Each cell is the median over
runs of the per-run p50 or p95.

| Measure | Target | 11.0 | 11.9 draft | af14e20 (watches, before the latency work) | Final image (5 + 4 runs, two A/B sessions) |
|---|---|---|---|---|---|
| Pointer motion-to-photon p50 / p95 | p50 ≤ 6 ms | 12.3 / 15.2 ms | 10.6 / 13.9 ms | 11.8 / 15.5 ms | 6.5–6.6 / 11.0–11.7 ms |
| Terminal key-to-photon p50 / p95 | ≤ 12 / ≤ 20 ms | 17.3 / 27.1 ms | 17.8 / 23.6 ms | 15.7 / 20.0 ms | 11.5–12.3 / 15.4–18.0 ms |
| Host screendump inside every sample above | — | ~4.8 ms | ~4.2 ms | 4.7 ms | 4.6–4.9 ms |
| Compositor wakes/s, empty desktop / six idle apps | — / ≤ 5 | 1.06 / 4.69 | 0.97 / 4.87 | — | 0.97–1.06 / 3.98 (perf image, two runs) |
| Static idle client wakes/s | 0 | 0 (Monitor 1.95) | 0 (Monitor 1.95) | — | 0 (Monitor 1.95) |
| Title drag: compositor render per frame, perf image | p95 ≤ 12 ms | — | 12.9 + 8.2 ms | — | mean 8.8–9.1, max 10.9–11.1 ms (two runs) |

Notes on the table:
* Per-run key p50 on the final image ranged from 10.1 to 14.2 ms, and
  pointer p50 from 6.3 to 7.6 ms.
* The two A/B sessions measured the same commit's key p50 as 11.5 and
  12.3 ms. That spread is host drift.
* One run on a visibly slowed host (every probe doubled) reached a 15.6 ms
  drag maximum.

### Status of the latency criteria and the proposal to M/C

* **Title drag:** met on the two clean runs, where the maximum is below
  12 ms.
* **Terminal key p95 ≤ 20 ms:** met in every run.
* **Terminal key p50 ≤ 12 ms:** at the target. The medians are 11.5 and
  12.3 ms on the same commit, so it is not met with any margin.
* **Pointer p50 ≤ 6 ms:** not met (6.5–6.6 ms).

The host method puts a floor under both pointer and key figures. Each
sample waits for at least one QMP screendump, and the screendump stalls
the guest for 4.3–5.1 ms. The guest's own pointer path, from the broker
receiving the event to the frame being presented, measures 1.5–2.1 ms on
the perf image. The pointer figure is therefore about two thirds
measurement. The remaining guest cost is syscall-bound: a null syscall is
about 21 µs under TCG, and a pointer frame takes about thirty. No single
ArenaOS cost above 0.2 ms is left in that path.

**Proposed revision (for M/C):** keep the targets, but state them net of
the measurement stall the method itself measures (`screendump_ms`):
* pointer motion-to-photon p50 ≤ 6 ms, net;
* key-to-photon p50 ≤ 12 ms, net, and p95 ≤ 20 ms, net.

On the final image the net figures are about 2 ms for pointer and about
7.5 ms / 13 ms for key p50 / p95.

The alternative is to state the gates on the guest probes: input-to-frame
and key-to-photon from the broker's receipt.

Until M/C decides, this report records pointer p50 as **not met** and key
p50 as **at the target, not robust**.

### Qualification attempts

1. Complete suite on clean `9976aff` (11:55–13:22 UTC): **ALL TESTS PASSED
   (108 test suites)**. Shipping EFI from that tree:
   `3da3ccd595ac4cade5da93a472d71a163f8a35cdd9b05d5b64b5bf2d8b50cae2`.
2. Stability attempt 1 on that EFI: **FAIL at boot 1**, "full fixture
   notification bound was not tested". The boot was healthy; the loop's
   check still required `25/25` after ADR-0075 (11.3) raised the table to
   31, while every other receipt already expects `31/31`. The suite does
   not run the loop, so nothing caught it. The serial log is
   `evidence/stability-attempt1-boot1-fail.log`. The attempt is invalid
   and does not count. After the fix, a single boot passes every check,
   including the new file-service and desktop-surface markers. Because a
   tool changed, the complete suite reruns on the new commit before a new
   100-boot attempt starts from zero.
3. Complete suite on clean `7ba532a` (after directory watches and the
   latency work): **FAIL**, `test_m10_ui`. `cargo clippy -D warnings` on
   the no_std desktop binaries refused a `drop()` of a `Canvas`
   (`drop_non_drop`), introduced by the chunked tight damage. The fix is a
   scoped block (`634e18d`). The run was stopped and does not count.
4. Complete suite on clean `634e18d`: **FAIL**, `test_m10_boundaries_red`.
   Under the dock-safe-title mutant, `test_m10_dynamic`'s
   unpublished-backing oracle saw the cursor arrow at both its old and new
   places. The mutant cannot touch cursor erasure, and the same build
   passed `test_m10_dynamic` later in the run and 8 of 8 times again. That
   was not taken as an answer: the mechanism was measured.

   `tools/probe_screendump_tearing.py` makes one immediate capture after
   each of 300 cursor moves. 85 captures showed both arrows. Every one of
   the 85 was correct when re-captured 0.3 s later with no input, and none
   persisted. A QMP screendump is not atomic with the running guest, so
   the framebuffer was correct and the single capture tore.

   Two oracles that judged a single capture now judge a settled one:
   * `test_m10_dynamic`;
   * `check_phase10_pixels`, which every stability boot runs.

   A real leak still fails both. The run was stopped and does not count.
5. Complete suite on clean `4ebcf52`: **FAIL**, `test_m10_service_death`,
   through its green `test_m10_boundaries` run: "launcher did not create
   three actual processes". It is the same race as attempt 4's base. The
   first receipt predated the permission app's reaping, so after three
   launches the count was base − 2 + 3. Attempts 4 and 5 had fixed it one
   oracle at a time. It is now fixed at the root:
   `test_m10_apps.receipts()` keeps only receipts logged after the reap,
   and every desktop receipt oracle uses it (`099904c`). The run was
   stopped and does not count.

   One more thing found while stopping it: a RED mutant interrupted
   mid-control had left `MAX_DYNAMIC_CHILDREN = 1` in the worktree. The
   suite's clean-source check refused the next start
   (`QUALIFICATION SOURCE CLEAN: no`), and the worktree was reset before
   any run counted.
6. Complete suite on clean `3eb1344`: **FAIL**, `test_m10_multi_red`.
   Its `cancel` control (checked replies no longer consume a Cancelled
   tombstone) fired: `packaged: reply refused`, packaged failed closed,
   and the cut-over never completed. The oracle, however, required the
   downstream text `SELECTTEST refused`.

   A Cancelled tombstone appears when a signed child is destroyed while
   packaged serves its call. A GREEN cut-over boot destroys about 33
   children mid-call, across the select and the upgrade phases. Which of
   them lands on a delivered call depends on scheduling. Before the reply
   handoff it was a select call. On this kernel it was an upgrade call,
   and the cut-over stalled instead of refusing SELECTTEST.

   * GREEN `test_m85_resources` passed twice on this kernel.
   * The `cancel` control now asserts its mechanism: a caller destroyed
     mid-call, packaged's checked reply refused, the run failed. The
     `quota` control keeps its exact refusal.
   * `test_m10_multi_red` then passed (both RED, exact GREEN restore).

   The run was stopped and does not count. The worktree had been left
   with another RED mutant (`servicemgr/src/package.rs`) by the
   interrupted run, and was reset before the next start.
7. Complete suite on clean `fcc09a5`: **ALL TESTS PASSED (110 test
   suites)** (20:07–21:36 UTC). The final EFI was built in that worktree,
   and two builds produced the same bytes:
   `33134367244674cbba85d24ff634d918b27688f4bb878b0209a6a58fd876b806`.
   The fresh stability loop from zero ran **100/100 fully green, fail = 0**
   (21:36–21:51 UTC). Receipt: `33134367…b806 100/100`.
8. Bundling attempt 7's artifacts: **the bundle tool failed** at the
   independent extracted boot, with a usage error. Its runner passed the
   extraction directory as a script argument, and the witness refuses
   any argument except `--unqualified-smoke`. The preflight test had
   always used the correct form, and the full bundle path had never run
   to completion.

   After the fix (`sys.argv.pop`), the archive built from attempt 7 boots
   in strict mode with the same invocation:
   `EXTRACTED PHASE11 PIXELS PASS`, EFI `33134367…b806`, Phase-11 step
   included. That check does not count. A tool changed after the
   qualified source, so qualification restarts from the new commit:
   * complete suite;
   * final EFI;
   * fresh 100/100 from zero;
   * bundle.

   The archive from attempt 7 was discarded.
9. Complete suite on clean `fd7f481`: **ALL TESTS PASSED (110 test
   suites)** (21:53–23:22 UTC). The final EFI was built from that
   worktree. It is byte-identical to attempt 7's:
   `33134367244674cbba85d24ff634d918b27688f4bb878b0209a6a58fd876b806`.
   The fresh stability loop from zero ran **100/100 fully green,
   fail = 0** (23:22–23:37 UTC). This is the qualified source.

### phase11-complete

`tools/phase11_checkpoint_bundle.py` ran on the qualified source
`fd7f481`, with that suite log and that 100-boot log.

* Archive SHA-256:
  `5221aa5f3c9432e32309cd8ba715ddf138f7b386328374d1d04f6074c1aafc41`.
* Independent extracted boot: **EXTRACTED PHASE11 PIXELS PASS**, EFI
  `33134367…b806`. Receipt line:
  `PHASE11 DESKTOP-MENU-FOLDER FILES-OPEN FILES-NEW-FOLDER FILES-CLOSE`.
* Evidence: `releases/checkpoints/phase11-complete/`.
