# ADR-0088: Phase 12 desktop capacity envelope for 32 managed windows

Status: Accepted; 32-session built-in checkpoint passed 2026-10-06; broader qualification remains a release gate
Date: 2026-10-06
Authors: ArenaOS project / Phase 12

## Context

ADR-0075 qualified twelve simultaneous Desktop sessions on Phase 11. The
measured 800×600 session reservation is 471 shared pages plus 469 private
snapshot pages; a complete session contributes two SharedRegion records and
three live maps after client startup. The settled twelve-session boot uses
11,749 of 20,480 shared pages, 25 of 32 region records, 38 of 64 maps, 27 of
32 processes/spawn records, and a transient Desktop capability high-water of
61 of 64. The existing notification table is exactly full at boot with twelve
private client clocks. Those bounds cannot safely support Phase 12's required
32 simultaneously managed ordinary windows.

The twelve-session values remain valid Phase-11 historical evidence. This
ADR deliberately does not edit ADR-0075 or reinterpret any APKG v1 bytes. The
new bounds are a Phase-12 measured-capacity change, not a general unbounded
resource policy.

## Decision

Raise only the fixed tables that the 32-window topology demonstrably needs:

| Resource | Phase 11 | Phase 12 bound | 32-window budget |
|---|---:|---:|---:|
| Desktop ordinary windows/sessions | 12 | 32 | 32 app-owned sessions |
| Processes | 32 | 64 | 15 boot + 32 app processes = 47, leaving 17 |
| Spawn records | 32 | 64 | same 47 steady records, explicit reap required |
| Per-process capability slots | 64 | 128 | 108 steady Desktop occupancy; measured peak 109, leaving 19 |
| SharedRegion records | 32 | 80 | 2 measured baseline + 2×32 sessions = 66, leaving 14 |
| Aggregate SharedRegion pages | 20,480 | 36,864 | 470 measured baseline + 940×32 = 30,550, leaving 6,314 |
| Shared mappings | 64 | 128 | 4 measured baseline + 3×32 sessions + 12 file I/O maps = 112, leaving 16 |
| IPC caller queue per endpoint | 16 | 32 | 32 callers queued; caller 33 receives mutation-free `STATUS_BUSY` (ADR-0089) |
| Notifications | 31 | 64 | 19 fixed + 32 private clocks = 51, leaving 13 |

The aggregate page bound is 144 MiB. The M12 guest measures 30,550 total
SharedRegion pages at 32 sessions (about 119.3 MiB), leaving 6,314 pages
(about 24.7 MiB) inside the SharedRegion budget; the independent physical-frame
check also passed with over 8,192 frames free at the full working set. Every
session reservation remains bounded by the existing 1,024-page per-region
limit. `MAX_ENDPOINTS` (16), timer quotas, the kernel-thread table (64),
dynamic Image slots/quota, and unrelated limits remain unchanged. ADR-0089
raises only the bounded IPC caller queue from 16 to 32 and specifies mutation-
free BUSY backpressure/retry. The session clocks share the existing Desktop
event topology; no endpoint per window is added.

`CAP_SLOTS` changes the startup inventory width and the size of each process's
capability space. The native startup runtime continues to scan the *entire*
slot range and fail closed on any unexpected occupancy. Every caller-facing
mirror, capacity test, occupancy diagnostic and proof that assumes 64 must be
updated in lockstep. No new cap rights or implicit inheritance are introduced.

The broker keeps its current fixed per-session allocation and validates a
session slot and group slot before allocating shared memory. After successful
spawn it drops its duplicate SharedRegion capability; the child retains the
exact explicitly delegated cap, while the Desktop keeps its two mapping pins
for rendering and later exact-VA unmap after reaping. This lowers persistent
manager cap use without adding inheritance or changing the child's authority.
The app manager continues to hold each child's exact Process capability
through `ProcessGroup`; numeric PIDs and window/app metadata remain
descriptive.

The 33rd launch refuses without changing processes, records, regions, pages,
mappings, notifications, caps, or frames. The guest samples both the complete
resource receipt and every caller-owned cap descriptor immediately before and
after refusal, while the input request's temporary capability is landed; the
seven resource fields and 128 cap descriptors compare exactly equal. Refusal
is not an invitation to evict another session or weaken existing authority.

## Qualification gates

The targeted M12 built-in-session guest passed 32 simultaneously live apps
across all six built-in kinds, exact full/half/reuse/final resource receipts,
full cap-inventory and resource-inventory equality on refusal, and real
maximize/minimize/dock-restore operations. This passes the built-in capacity
checkpoint, not the broader release gate: the run did not mix signed/dynamic
applications, prove multiple windows per app, or exercise headless helpers.

Before accepting the envelope for release, the guest suite must:

1. Boot 32 real ordinary sessions, including at least four signed/dynamic
   applications mixed with built-ins; prove all 32 own distinct regions,
   maps, Process caps and session clocks.
2. Attempt the 33rd launch and compare the full resource snapshot and cap
   inventory before/after; every field must be unchanged.
3. Close half, relaunch into reused slots, exercise window operations, close
   all 32 and compare records/processes/regions/pages/maps/caps/notifications
   to the settled baseline. Retained empty page tables are budgeted separately.
4. Confirm aggregate SharedRegion and physical-frame headroom on the exact
   512 MiB artifact; no OOM may be mislabeled as a capacity success.
5. Keep the 12-session interactions, APKG v1 suite, and extracted historical
   Phase-10/11 archives intact. Run affected M1–M11 and M12 regressions.

The 32-window projection is only a capacity checkpoint. Multi-window-per-app
lifecycle, separate headless helper processes, user-thread creation, streams,
installed-app discovery, full resource ownership on parent death, and the
artifact-bound Phase-12 stability/final archive gates remain separate work.

## Consequences

The larger arrays are fixed and remain bounded. The maximum theoretical
resource exposure rises, but does not become unbounded: per-process caps,
processes, spawn records, SharedRegions, pages, mappings, notifications, and
windows each have explicit independent limits. A partial table increase is not
qualified. If a gate fails, lower workload must be reported as unsupported
rather than silently changing an unrelated limit or treating an early failure
as a 32-window pass.
