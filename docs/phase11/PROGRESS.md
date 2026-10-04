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
