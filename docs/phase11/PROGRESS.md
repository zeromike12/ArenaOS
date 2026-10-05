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

## 11.8 — Files explorer and the desktop surface (in progress)

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
