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
