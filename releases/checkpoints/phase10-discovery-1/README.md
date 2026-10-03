# Failed Phase-10 discovery run

Source: b1422bfd2ea2f5478e8523451b5db96ba8dbac16.
Result: 90/94; this is an invalid qualification attempt.

The checksummed archive preserves the complete historical/Phase-10 run and
four failing receipts. Findings: stale static IPC layout expectations; one
M2 busy-wait measurement overshoot; two signed-selection receipt deadlines.
The last case exposed short-circuited original Process cleanup. ADR-0068
records the bounded corrections and native mutation controls. No failed boot
or failed suite is credited toward the later engineering baseline.
