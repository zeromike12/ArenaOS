# ADR-0068 — Clock sampling and signed-child diagnostic cleanup

Status: implemented; native clock and bad-receipt cleanup mutation controls
proven; complete frozen-source qualification pending.

## Evidence

The first complete Phase-10 discovery run passed 90/94 suites. The static IPC
audit still expected the Phase-9 queue layout. A configuration seed boot failed
the M2 two-millisecond busy-wait upper bound. Two signed-selection fixtures
refused a later child receipt while the actual child was still completing its
filesystem query. That refusal also exposed conditional Process cleanup.

## Decisions

M2 measures at most three busy-wait windows. Every window must advance
monotonically, return no earlier than requested and pass the anti-hang guard.
At least one window must meet the existing four-times upper bound; three
overshoots fail. This matches the bounded sampling of the existing PIT scale
cross-check and accounts for host descheduling of QEMU's vCPU. The clock API,
calibration, oscillator cross-check and busy-wait implementation are unchanged.
Native controls compile an early-return implementation, a single injected
measurement stall and persistent overshoot. A failed boot never earns a retry
or a qualification credit.

The SELECT diagnostic's child wait has a finite 15-second budget for its four
concurrent signed filesystem clients on the full 32-object platter. Production
service readiness and restart retain their two-second budget. Receipt bits and
actual held-Process liveness remain independent checks. The original Process
witness is finished unconditionally before combining a bad receipt result.
No name, PID or receipt grants destruction authority.

An actual signed ELF fixture forces the later receipt to fail. Omitting FINISH
must leave a native process/record/frame leak; the repaired path must return
all three counters exactly to baseline. Restored GREEN uses the saved EFI and
ESP bytes rather than rebuilding a nominally equivalent image.

The static audit retains the historical four-entry Endpoint layout projection
and checks the actual eight-entry production layout and 25-notification pool
from source-extracted definitions on the bare-metal target.
