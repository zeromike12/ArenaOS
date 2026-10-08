# ADR-0109: Phase-13 mixed application workload receipt

## Status

Accepted as the measured Phase-13 guest resource receipt. The T2/T3
preservation checkpoints and final T4 qualification passed on the preserved
native image.

## Context

Phase 12 proved 32 built-in one-window Desktop sessions. That did not exercise
the Phase-13 separation of installed application identity, AppInstance,
ProcessGroup, helper process, ordinary window, stream, VM region, user thread,
and synchronization domain under concurrent load. The additional resource
detail syscall (75) is restricted to an exact MemoryPool/READ capability. It
reports total physical frames, live threads, endpoints, notifications, timers,
VM regions and committed pages, synchronization domains/keys/waiters. Desktop
adds AppInstance, window, helper, stream-set, and per-instance window counts;
the existing manager receipt reports process and SharedRegion accounting.

## Decision

Use a representative interactive workload instead of raising resource
constants to chase a maximum. The signed APB1 guest installs an ordinary
multi-window application and a separate headless application. It launches five
ordinary installed AppInstances and one headless instance across the scenario,
keeps a helper in each ordinary instance, two document workers parked, opens a
read-only document, and uses native streams, timers, notifications, VM,
threads, and SyncDomains. It also retains the 21 built-in sessions used by the
historical pressure fixture. Each ordinary instance owns three real
compositor-backed windows.

The guest reaches the full 26-instance/32-window state, closes half of its
ordinary windows, closes all windows and reaps the associated helpers, reuses
the freed capacity with 16 additional real signed launches, then closes all
32 windows. The fixture requires identity-bearing resource counts to return
to their measured boot values. Its AFS2 reload boot independently verifies the
persisted launcher favorite.

## Measured receipt

The full-workload Desktop snapshot reports 115,670 total frames,
49/64 scheduler threads, 12/16 endpoints, 56/64 notifications, 5 timers at
the full snapshot (8 observed transiently during churn)/32, 7 VM regions,
355 committed VM pages, 5/64 synchronization domains, 2 keys, 2 waiters,
26 AppInstances, 32 ordinary windows, 5 helpers, 5 stream sets, and a maximum
of 3 ordinary windows in one AppInstance. Transient maxima across the guest
were 12 VM regions, 416 committed VM pages, 4 sync keys, 6 stream sets, and
8 timers.

The existing manager reports a high-water of 46/64 process records/processes,
71/96 SharedRegions, 30,555/36,864 SharedRegion pages, 119/160 maps, and
119/128 broker cap occupancy at a snapshot. The broker's observed cap
high-water was 120; the process capability-table width remains exactly 128,
and slot 127 retains its separate accounting/authority contract. The fixture
does not increase the historical low-32 manager accounting meaning.

At boot, free frames were 114,189 and the resource-detail syscall reported
16 live threads. At the full workload's 32-window snapshot, the manager
reported 79,188 free frames. After all windows closed, the manager reported
114,110 free frames; process/spawn records, processes, SharedRegions/pages,
maps, and broker caps returned to `(15, 15, 2, 470, 4, 47)`. Detailed
thread/IPC/timer/VM/synchronization/AppInstance/window/helper/stream counts
also returned to their boot values. The remaining 79-frame difference is
consistent with the 77 retained page-table frames measured by the Phase-12
32-session teardown proof. This run did not isolate individual page-table
frames, so it records the measured delta rather than asserting an exact
allocator decomposition.

The workload's own peak resource commitment is 36,482 frames relative to
115,670 total frames, including a boot-time used-frame baseline of 1,481.
The largest incremental workload allocation over that baseline was 35,001
frames. All SharedRegion, VM, app, process, and transient helper capacities
remain bounded; admission failures continue to be reported instead of
overwriting another authority slot.

## Consequences

- The mixed workload now demonstrates a materially broader application
  platform profile than the Phase-12 one-window broker fixture.
- The 79-frame residual is measured and bounded, but its exact table/page
  allocator attribution remains open to a future page-table accounting
  improvement.
- This T1 proof does not replace the required Phase-13 T2 preservation set or
  final T4 historical and artifact qualification.

## Evidence

- `python3 tools/test_phase13_registry_guest.py` on QEMU 10.0.11 / OVMF
  2025.02 passed the main 26-AppInstance/32-window pressure boot and the fresh
  AFS2 favorite-reload boot. It verified installed registry discovery and
  launch, All Applications search and keyboard/pointer launch, document
  Open-With authority, a three-window application, real helper/headless
  lifecycle, standard streams, threads, VM, synchronization, close-half,
  helper failure, relaunch, complete teardown, and clean shutdown.
- Main boot log: `build/serial-phase13-registry.log`.
- Favorite reload log: `build/serial-phase13-registry-favorites-reload.log`.
