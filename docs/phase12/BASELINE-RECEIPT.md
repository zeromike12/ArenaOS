# Phase-11 live baseline receipt for Phase 12

- **Observed:** 2026-10-06 UTC
- **Source commit:** `a077ee4a929a0271a28308ac2c7bb28fd2597aae` (session branch parent)
- **Guest:** existing Phase-11 `tools/test_m11_wm.py`, 800×600, QEMU 11.0.2, 512 MiB profile
- **Result:** process exit 0; all WM, twelve-session, thirteenth-refusal, and teardown assertions passed
- **EFI SHA-256:** `f1b8c1f4c89851b1104807975ef402cdcda86edfde5035891210cfa1dcfe134d`
- **ESP SHA-256:** `9d0e8a84bb65b4bfe70ace03c133ad12ded05fe045b1deb6900cc5dc0a593808`

The source checkout was dirty because Phase-12 documentation, host tests, and
an unlinked APB1 library were being added. The Phase-11 kernel, desktop, service
and test sources used for this boot were not modified. The above EFI and ESP
hashes bind this receipt to the exact artifacts that actually booted; this is
a Phase-11 baseline receipt, not Phase-12 qualification.

## Direct guest observations

The serial log contained:

```text
[desktop] session reservation shared/snapshot pages=471/469
[desktop] measured frames/records/processes/regions/pages/maps/caps=115127/15/15/1/469/2/23
```

The session reservation is therefore **471 shared client pages and 469 private
snapshot pages**, 940 pages per live desktop session. The Phase-11 accepted
ADR/progress number 470/469 is retained as history; the live source formula
and guest receipt agree on 471/469. No unrelated limit was changed.

`SYS_OBSERVE` counter order in the desktop receipt is
`free frames / spawn records / live processes / SharedRegion slots / shared
pages / mappings / broker cap occupancy`.

| State | Guest receipt | Delta from settled baseline |
|---|---|---|
| Settled boot/base after packaged READY and permission-app reap | `115127/15/15/1/469/2/23` | — |
| Twelve sessions at peak, all three maps/session landed | `101891/27/27/25/11749/38/47` | +12 records, +12 processes, +24 regions, +11,280 shared pages, +36 maps, +24 broker caps; free frames fell by 13,236 |
| After all twelve closed | `115103/15/15/1/469/2/23` | Every non-frame counter returned; 24 fewer free frames, equal to 2 retained broker page-table frames per one of the 12 warmed session VA slots |

The 24-frame residual is consistent with existing Phase-11 proofs:
`tools/test_m10_boundaries.py` expects two warmed broker page-table frames per
session (six for three sessions), `tools/test_m10_client_death.py` expects four
for two sessions, and Phase-11 progress records 28 retained frames at the
12-session/4-filesd-session peak. It is accounted as bounded page-table
residency, not claimed as a fully returned physical-frame baseline.

The thirteenth launch was refused with status `-4`. Records, processes,
regions, pages, maps, and broker caps were unchanged. The free-frame sample at
the forced refusal receipt was `101899`, eight more free frames than the
preceding settled peak sample `101891`; the existing Phase-11 test deliberately
compares the other six counters and does not establish a frame-stable refusal
receipt. Phase 12 must separately determine whether this is concurrent/warm
page-table reclamation or an admission-path effect before claiming complete
all-resource mutation-free refusal. The API/function limit itself was not
raised in this baseline exercise.

`tools/test_m11_wm.py` also observed maximize, restore, edge resize, menu,
keyboard repeat/chords, wheel scroll, minimize/dock restore, and Alt+Tab, and
verified non-frame counts at capacity and after teardown. The full serial log,
QMP trace and screenshots were generated under ignored `build/`; the durable
machine-checkable summary and exact artifact hashes are recorded here.
